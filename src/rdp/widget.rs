/* rdp/widget.rs
 *
 * Copyright 2026 Florian Richter
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use adw::prelude::*;
use gettextrs::gettext;
use gtk::glib::subclass::Signal;
use gtk::glib::{self, Properties};
use gtk::gsk;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::sync::{OnceLock, mpsc};
use tracing::{info, warn};

use crate::model::destination_object::ConnectionOptions;

use super::clipboard::Clipboard;
use super::errors::{friendly_connection_error, friendly_termination_error};
use super::key_handler::{KeyHandler, RemoteKeySender};
use super::session::{
    CertificateDecision, CertificateDetails, Session, SessionEvent, TerminationReason,
};
use super::viewport::{Fit, Viewport};
use super::{config, input, render};

const GRACEFUL_DISCONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, glib::Enum, Default)]
#[enum_type(name = "RdpState")]
pub enum RdpState {
    #[default]
    Disconnected = 0,
    Connecting = 1,
    Connected = 2,
    /// The transport died and the session is being restored. The session
    /// object stays alive throughout, so this is not `Disconnected`.
    Interrupted = 3,
}

mod imp {
    use super::*;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::RdpWidget)]
    pub struct RdpWidget {
        #[property(get, set, builder(RdpState::Disconnected))]
        state: Cell<RdpState>,
        /// Reconnect attempts made so far in the current cycle.
        #[property(get)]
        reconnect_attempt: Cell<u32>,
        /// Seconds until the next attempt; 0 while one is in flight.
        #[property(get)]
        reconnect_seconds_left: Cell<u32>,
        /// Whether a successful reconnect restores the existing remote session
        /// rather than opening a new one.
        #[property(get)]
        reconnect_restores_session: Cell<bool>,
        session: RefCell<Option<Session>>,
        texture: RefCell<Option<gdk::MemoryTexture>>,
        resize_timeout: RefCell<Option<glib::SourceId>>,
        disconnect_timeout: RefCell<Option<glib::SourceId>>,
        pending_certificate: RefCell<Option<mpsc::SyncSender<CertificateDecision>>>,
        pub(in crate::rdp) clipboard: Clipboard,
        pub(super) key_handler: RefCell<KeyHandler>,
        generation: Cell<u64>,
        pointer_x: Cell<u16>,
        pointer_y: Cell<u16>,
        connection_scale: Cell<f64>,
        viewport: Cell<Viewport>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RdpWidget {
        const NAME: &'static str = "RdpWidget";
        type Type = super::RdpWidget;
        type ParentType = gtk::Widget;
    }

    impl RdpWidget {
        fn surface_scale(&self) -> f64 {
            self.obj()
                .native()
                .and_then(|native| native.surface())
                .map(|surface| surface.scale())
                .unwrap_or_else(|| self.obj().scale_factor() as f64)
        }

        fn physical_size(
            &self,
            logical_width: f64,
            logical_height: f64,
        ) -> Option<(u16, u16, u32)> {
            let scale = self.surface_scale();
            let width = u16::try_from((logical_width * scale).round() as i64).ok()?;
            let height = u16::try_from((logical_height * scale).round() as i64).ok()?;
            Some((width, height, (scale * 100.0).round() as u32))
        }

        fn update_viewport(&self, update: impl FnOnce(&mut Viewport)) {
            let mut viewport = self.viewport.get();
            update(&mut viewport);
            self.viewport.set(viewport);
            self.obj().queue_draw();
        }

        pub(super) fn apply_connection_options(
            &self,
            options: ConnectionOptions,
            announce_clipboard: bool,
        ) {
            self.clipboard.set_enabled(options.clipboard_enabled);
            if announce_clipboard && options.clipboard_enabled {
                self.announce_local_clipboard();
            }
            {
                let mut key_handler = self.key_handler.borrow_mut();
                key_handler.set_forward_unicode(options.forward_unicode);
                key_handler.set_inhibit_system_shortcuts(options.inhibit_system_shortcuts);
            }
            self.update_system_shortcut_inhibition();
        }

        pub fn connect_to_server(
            &self,
            hostname: String,
            port: u16,
            username: String,
            password: secrecy::SecretString,
            width: u16,
            height: u16,
            options: ConnectionOptions,
        ) {
            let Some((width, height, desktop_scale)) =
                self.physical_size(width.into(), height.into())
            else {
                return;
            };
            info!("Connecting to {hostname}:{port} {width}x{height}");

            self.abandon_session();
            *self.texture.borrow_mut() = None;
            self.obj().queue_draw();

            let generation = self.generation.get().wrapping_add(1);
            self.generation.set(generation);
            self.connection_scale.set(self.surface_scale());
            self.update_viewport(Viewport::reset_remote);
            self.apply_connection_options(options, false);
            self.obj().set_state(RdpState::Connecting);

            let config = config::build_config(
                hostname,
                port,
                username,
                password,
                width,
                height,
                desktop_scale,
            );
            let (output, receiver) = async_channel::bounded(64);
            let Some(session) = Session::spawn(config, output) else {
                self.obj().set_state(RdpState::Disconnected);
                self.obj().emit_by_name::<()>(
                    "connection-failed",
                    &[&gettext("Could not initialize FreeRDP.")],
                );
                return;
            };
            *self.session.borrow_mut() = Some(session);

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    while let Ok(event) = receiver.recv().await {
                        if imp.generation.get() != generation {
                            break;
                        }
                        imp.process_event(event);
                    }
                }
            ));
        }

        pub fn disconnect(&self) {
            if let Some(response) = self.pending_certificate.borrow_mut().take() {
                let _ = response.send(CertificateDecision::Reject);
            }
            let session = self.session.borrow().as_ref().cloned();
            let Some(session) = session else {
                return;
            };

            // A graceful goodbye only makes sense on a live connection; wait
            // for the worker to deliver it.
            if self.state.get() == RdpState::Connected {
                session.disconnect();
                self.arm_disconnect_watchdog(session);
                return;
            }

            // While connecting or reconnecting the worker is often blocked in
            // a FreeRDP call that cannot be cut short - name resolution and TCP
            // timeouts take tens of seconds. Ask it to stop, then let go of the
            // session right away instead of making the user wait on a screen
            // they have already dismissed. The worker holds its own reference
            // and frees the native session once it finally returns.
            self.finish_session();
        }

        pub fn reconnect_now(&self) {
            if let Some(session) = self.session.borrow().as_ref() {
                session.reconnect_now();
            }
        }

        fn arm_disconnect_watchdog(&self, session: Session) {
            self.clear_disconnect_watchdog();
            let source_id = glib::timeout_add_local_once(
                GRACEFUL_DISCONNECT_TIMEOUT,
                glib::clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        *imp.disconnect_timeout.borrow_mut() = None;
                        if imp.state.get() != RdpState::Disconnected {
                            warn!("Graceful disconnect timed out; forcing connection drop");
                            session.abort();
                        }
                    }
                ),
            );
            *self.disconnect_timeout.borrow_mut() = Some(source_id);
        }

        fn clear_disconnect_watchdog(&self) {
            if let Some(source_id) = self.disconnect_timeout.borrow_mut().take() {
                source_id.remove();
            }
        }

        fn process_event(&self, event: SessionEvent) {
            match event {
                SessionEvent::Frame {
                    buffer,
                    width,
                    height,
                    stride,
                } => {
                    if self.state.get() == RdpState::Connecting {
                        info!("State connected (first frame received)");
                        self.obj().set_state(RdpState::Connected);
                        self.announce_local_clipboard();
                    }
                    if let Some(texture) = render::image_texture(buffer, width, height, stride) {
                        *self.texture.borrow_mut() = Some(texture);
                        self.update_viewport(|viewport| viewport.set_remote_size(width, height));
                    }
                }
                SessionEvent::Cursor {
                    data,
                    width,
                    height,
                    hotspot_x,
                    hotspot_y,
                } => {
                    if let Some(cursor) = render::pointer_cursor(
                        data,
                        width,
                        height,
                        hotspot_x,
                        hotspot_y,
                        self.connection_scale.get(),
                    ) {
                        self.obj().set_cursor(Some(&cursor));
                    }
                }
                SessionEvent::CursorHidden => {
                    self.obj()
                        .set_cursor(gdk::Cursor::from_name("none", None).as_ref());
                }
                SessionEvent::CursorDefault => {
                    self.obj()
                        .set_cursor(gdk::Cursor::from_name("default", None).as_ref());
                }
                SessionEvent::DisplayControl(available) => {
                    info!("Server resize support: {available}");
                    self.update_viewport(|viewport| viewport.set_server_resizable(available));
                    if available {
                        // Catch up with any window resize made before the
                        // channel came up.
                        let obj = self.obj();
                        self.queue_resize_to_logical_size(obj.width(), obj.height());
                    }
                }
                SessionEvent::ClipboardRemoteTextAvailable => {
                    if !self.clipboard.enabled() {
                        return;
                    }
                    if let Some(session) = self.session.borrow().as_ref() {
                        session.request_clipboard_text();
                    }
                }
                SessionEvent::ClipboardRemoteFilesAvailable => {
                    if !self.clipboard.enabled() {
                        return;
                    }
                    if let Some(session) = self.session.borrow().as_ref() {
                        session.request_clipboard_files();
                    }
                }
                SessionEvent::ClipboardRemoteFiles(files) => {
                    if let Some(session) = self.session.borrow().as_ref().cloned() {
                        self.clipboard
                            .set_remote_file_transfer_portal(&self.obj(), session, files);
                    }
                }
                SessionEvent::ClipboardRemoteFileContents { stream_id, data } => {
                    self.clipboard.complete_file_contents(stream_id, data);
                }
                SessionEvent::ClipboardText(text) => {
                    self.clipboard.set_remote_text(&self.obj(), text);
                }
                SessionEvent::CertificateRequest { details, response } => {
                    self.present_certificate_dialog(details, response);
                }
                SessionEvent::ConnectionFailure(error) => {
                    self.finish_session();
                    let message = friendly_connection_error(&error);
                    self.obj()
                        .emit_by_name::<()>("connection-failed", &[&message]);
                }
                SessionEvent::Interrupted {
                    detail,
                    can_restore_session,
                } => {
                    warn!("RDP session interrupted: {detail}");
                    // Keep the texture: the last frame stays on screen behind
                    // the reconnect page and reappears without a black flash.
                    self.clipboard.clear_pending();
                    if let Some(source_id) = self.resize_timeout.borrow_mut().take() {
                        source_id.remove();
                    }
                    self.set_reconnect_progress(0, 0);
                    self.reconnect_restores_session.set(can_restore_session);
                    self.obj().notify_reconnect_restores_session();
                    self.obj().set_state(RdpState::Interrupted);
                }
                SessionEvent::ReconnectCountdown {
                    attempt,
                    seconds_left,
                } => {
                    self.set_reconnect_progress(attempt, seconds_left);
                }
                SessionEvent::ReconnectAttempt { attempt } => {
                    self.set_reconnect_progress(attempt, 0);
                }
                SessionEvent::Reconnected => {
                    self.obj().set_state(RdpState::Connected);
                    self.announce_local_clipboard();
                }
                SessionEvent::Terminated(reason) => {
                    if let TerminationReason::Lost(error) = &reason {
                        let message = friendly_termination_error(error);
                        self.finish_session();
                        self.obj()
                            .emit_by_name::<()>("connection-failed", &[&message]);
                        return;
                    }
                    self.finish_session();
                }
            }
        }

        fn set_reconnect_progress(&self, attempt: u32, seconds_left: u32) {
            if self.reconnect_attempt.replace(attempt) != attempt {
                self.obj().notify_reconnect_attempt();
            }
            if self.reconnect_seconds_left.replace(seconds_left) != seconds_left {
                self.obj().notify_reconnect_seconds_left();
            }
        }

        /// Stops the current session and stops listening to its worker,
        /// without touching the state property.
        ///
        /// The worker thread may still be inside a blocking FreeRDP call. It
        /// owns a reference of its own, so the native session outlives this
        /// and is freed once the worker returns; retiring the generation makes
        /// sure its remaining events are ignored.
        fn abandon_session(&self) {
            if let Some(response) = self.pending_certificate.borrow_mut().take() {
                let _ = response.send(CertificateDecision::Reject);
            }
            if let Some(session) = self.session.borrow_mut().take() {
                // A graceful goodbye, when one is wanted, has already been
                // sent by disconnect(); by the time we get here the worker
                // only needs to stop.
                session.abort();
            }
            self.clear_disconnect_watchdog();
            if let Some(source_id) = self.resize_timeout.borrow_mut().take() {
                source_id.remove();
            }
            self.clipboard.clear_pending();
            self.set_reconnect_progress(0, 0);
            self.generation.set(self.generation.get().wrapping_add(1));
        }

        fn finish_session(&self) {
            self.abandon_session();
            self.obj().set_state(RdpState::Disconnected);
        }

        pub fn queue_resize_to_logical_size(&self, width: i32, height: i32) {
            let state = self.state.get();
            // Resizing an interrupted session is pointless; the window resizes
            // it again once it is back.
            if state == RdpState::Disconnected
                || state == RdpState::Interrupted
                || width <= 0
                || height <= 0
            {
                return;
            }
            if let Some(source_id) = self.resize_timeout.borrow_mut().take() {
                source_id.remove();
            }
            let source_id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(500),
                glib::clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        *imp.resize_timeout.borrow_mut() = None;
                        let state = imp.state.get();
                        if state == RdpState::Disconnected || state == RdpState::Interrupted {
                            return;
                        }
                        let Some((width, height, scale)) =
                            imp.physical_size(width.into(), height.into())
                        else {
                            return;
                        };
                        imp.connection_scale.set(imp.surface_scale());
                        if let Some(session) = imp.session.borrow().as_ref() {
                            session.resize(width.into(), height.into(), scale);
                        }
                    }
                ),
            );
            *self.resize_timeout.borrow_mut() = Some(source_id);
        }

        fn present_certificate_dialog(
            &self,
            details: CertificateDetails,
            response: mpsc::SyncSender<CertificateDecision>,
        ) {
            if let Some(previous) = self.pending_certificate.borrow_mut().replace(response) {
                let _ = previous.send(CertificateDecision::Reject);
            }

            let heading = if details.changed() {
                gettext("The server certificate has changed")
            } else {
                gettext("Untrusted server certificate")
            };
            let mut body = format!(
                "{}: {}:{}\n{}: {}\n{}: {}\n{}: {}",
                gettext("Server"),
                details.host,
                details.port,
                gettext("Subject"),
                details.subject,
                gettext("Issuer"),
                details.issuer,
                gettext("Fingerprint"),
                details.fingerprint
            );
            if !details.common_name.is_empty() {
                body.push_str(&format!(
                    "\n{}: {}",
                    gettext("Common name"),
                    details.common_name
                ));
            }
            if details.host_mismatch {
                body.push_str(&format!(
                    "\n\n{}",
                    gettext("The certificate name does not match this server.")
                ));
            }
            if let Some(old) = details.old_fingerprint.as_deref() {
                body.push_str(&format!("\n{}: {}", gettext("Previous fingerprint"), old));
            }
            if let Some(old_subject) = details.old_subject.as_deref() {
                body.push_str(&format!(
                    "\n{}: {}",
                    gettext("Previous subject"),
                    old_subject
                ));
            }
            if let Some(old_issuer) = details.old_issuer.as_deref() {
                body.push_str(&format!("\n{}: {}", gettext("Previous issuer"), old_issuer));
            }

            let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
            dialog.add_response("cancel", &gettext("Cancel"));
            dialog.add_response("once", &gettext("Trust Once"));
            dialog.add_response("always", &gettext("Trust and Remember"));
            dialog.set_response_appearance("always", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let parent = self.obj().root().and_downcast::<gtk::Window>();

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    let choice = dialog.choose_future(parent.as_ref()).await;
                    let decision = match choice.as_str() {
                        "once" => CertificateDecision::TrustOnce,
                        "always" => CertificateDecision::TrustPermanently,
                        _ => CertificateDecision::Reject,
                    };
                    if let Some(response) = imp.pending_certificate.borrow_mut().take() {
                        let _ = response.send(decision);
                    }
                }
            ));
        }

        pub(super) fn update_system_shortcut_inhibition(&self) {
            let obj = self.obj();
            self.key_handler.borrow().update_system_shortcut_inhibition(
                &*obj,
                self.state.get() == RdpState::Connected && obj.has_focus(),
            );
        }

        fn handle_key_pressed(
            &self,
            keyval: gtk::gdk::Key,
            keycode: u32,
            state: gtk::gdk::ModifierType,
        ) -> glib::Propagation {
            if !self.obj().has_focus() {
                return glib::Propagation::Proceed;
            }

            let mut sender = WidgetRemoteKeySender { imp: self };
            self.key_handler
                .borrow_mut()
                .handle_key_pressed(keyval, keycode, state, &mut sender)
        }

        fn handle_key_released(&self, keycode: u32) {
            if !self.obj().has_focus() {
                return;
            }

            let mut sender = WidgetRemoteKeySender { imp: self };
            self.key_handler
                .borrow_mut()
                .handle_key_released(keycode, &mut sender);
        }

        pub(super) fn register_key_controller_on(&self, widget: &impl IsA<gtk::Widget>) {
            let controller = gtk::EventControllerKey::new();
            controller.set_propagation_phase(gtk::PropagationPhase::Capture);
            controller.connect_key_pressed(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_controller, keyval, keycode, state| {
                    imp.handle_key_pressed(keyval, keycode, state)
                }
            ));
            controller.connect_key_released(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller, _keyval, keycode, _state| imp.handle_key_released(keycode)
            ));
            widget.add_controller(controller);
        }

        fn send_mouse(&self, flags: u16, x: f64, y: f64) {
            if self.state.get() != RdpState::Connected {
                return;
            }
            let (x, y) = self.viewport.get().to_remote(x, y);
            self.pointer_x.set(x);
            self.pointer_y.set(y);
            if let Some(session) = self.session.borrow().as_ref() {
                session.send_mouse(flags, x, y);
            }
        }

        fn setup_motion_controller(&self) {
            let controller = gtk::EventControllerMotion::new();
            controller.connect_motion(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller, x, y| imp.send_mouse(input::PTR_FLAGS_MOVE, x, y)
            ));
            controller.connect_enter(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller, _x, _y| {
                    if imp.state.get() == RdpState::Connected {
                        let obj = imp.obj();
                        obj.grab_focus();
                        imp.announce_local_clipboard();
                        imp.update_system_shortcut_inhibition();
                    }
                }
            ));
            controller.connect_leave(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller| {
                    let obj = imp.obj();
                    imp.key_handler
                        .borrow()
                        .update_system_shortcut_inhibition(&*obj, false);
                    if let Some(root) = obj.root() {
                        root.set_focus(None::<&gtk::Widget>);
                    }
                }
            ));
            self.obj().add_controller(controller);
        }

        pub(in crate::rdp) fn announce_local_clipboard(&self) {
            self.clipboard.announce_local(
                &self.obj(),
                self.session.borrow().as_ref().cloned(),
                self.state.get(),
            );
        }

        fn setup_clipboard(&self) {
            self.clipboard.setup(&self.obj());
        }

        fn setup_input_controller(&self) {
            let click = gtk::GestureClick::new();
            click.set_button(0);
            click.connect_pressed(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |gesture, _count, x, y| {
                    if let Some(button) = input::mouse_button(gesture.current_button()) {
                        imp.send_mouse(button | input::PTR_FLAGS_DOWN, x, y);
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            ));
            click.connect_released(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |gesture, _count, x, y| {
                    if let Some(button) = input::mouse_button(gesture.current_button()) {
                        imp.send_mouse(button, x, y);
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            ));
            self.obj().add_controller(click);

            let scroll =
                gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
            scroll.connect_scroll(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |controller, dx, dy| {
                    if imp.state.get() != RdpState::Connected {
                        return glib::Propagation::Proceed;
                    }
                    if let Some(session) = imp.session.borrow().as_ref() {
                        for flags in input::scroll_flags(dx, dy, controller.unit()) {
                            session.send_mouse(flags, imp.pointer_x.get(), imp.pointer_y.get());
                        }
                    }
                    glib::Propagation::Stop
                }
            ));
            self.obj().add_controller(scroll);
        }
    }

    struct WidgetRemoteKeySender<'a> {
        imp: &'a RdpWidget,
    }

    impl RemoteKeySender for WidgetRemoteKeySender<'_> {
        fn send_key(&mut self, keycode: u16, pressed: bool) {
            if self.imp.state.get() != RdpState::Connected {
                return;
            }
            if let (Some(scancode), Some(session)) = (
                input::key_scancode(keycode),
                self.imp.session.borrow().as_ref(),
            ) {
                session.send_key(scancode, pressed);
            }
        }

        fn send_unicode_char(&mut self, ch: char, pressed: bool) {
            if self.imp.state.get() != RdpState::Connected {
                return;
            }
            if let Some(session) = self.imp.session.borrow().as_ref() {
                let mut buffer = [0; 2];
                for code in ch.encode_utf16(&mut buffer) {
                    session.send_unicode(*code, pressed);
                }
            }
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RdpWidget {
        fn constructed(&self) {
            self.parent_constructed();
            self.setup_motion_controller();
            self.setup_input_controller();
            self.setup_clipboard();
            self.obj().set_focusable(true);
            self.obj().connect_state_notify(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.update_system_shortcut_inhibition()
            ));
            self.obj().connect_has_focus_notify(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.update_system_shortcut_inhibition()
            ));
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("connection-failed")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }

        fn dispose(&self) {
            self.disconnect();
        }
    }

    impl WidgetImpl for RdpWidget {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let scale = self.surface_scale();
            self.update_viewport(|viewport| {
                viewport.set_widget_size(width.into(), height.into(), scale)
            });
            self.queue_resize_to_logical_size(width, height);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let width = self.obj().width() as f32;
            let height = self.obj().height() as f32;
            if width <= 0.0 || height <= 0.0 {
                return;
            }

            let scale = self.surface_scale() as f32;
            let viewport = self.viewport.get();

            let round = gsk::SnapDirection::Round;

            snapshot.save();
            snapshot.set_snap(gsk::RectSnap::new(round, round, round, round));

            if let Some(texture) = self.texture.borrow().as_ref() {
                match viewport.fit() {
                    Fit::Exact => {
                        snapshot.scale(1.0 / scale, 1.0 / scale);
                        snapshot.append_scaled_texture(
                            texture,
                            gsk::ScalingFilter::Nearest,
                            &gtk::graphene::Rect::new(
                                0.0,
                                0.0,
                                texture.width() as f32,
                                texture.height() as f32,
                            ),
                        );
                    }
                    Fit::Stretch => {
                        snapshot.append_texture(
                            texture,
                            &gtk::graphene::Rect::new(0.0, 0.0, width, height),
                        );
                    }
                    Fit::Contain => {
                        let rect = viewport.display_rect();
                        snapshot.append_color(
                            &gdk::RGBA::BLACK,
                            &gtk::graphene::Rect::new(0.0, 0.0, width, height),
                        );
                        snapshot.append_scaled_texture(
                            texture,
                            gsk::ScalingFilter::Trilinear,
                            &gtk::graphene::Rect::new(
                                rect.x as f32,
                                rect.y as f32,
                                rect.width as f32,
                                rect.height as f32,
                            ),
                        );
                    }
                }
            } else {
                snapshot.append_color(
                    &gdk::RGBA::BLACK,
                    &gtk::graphene::Rect::new(0.0, 0.0, width, height),
                );
            }
            snapshot.restore();
            self.parent_snapshot(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct RdpWidget(ObjectSubclass<imp::RdpWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RdpWidget {
    pub fn connect_to_server(
        &self,
        hostname: String,
        port: u16,
        username: String,
        password: secrecy::SecretString,
        width: u16,
        height: u16,
        options: ConnectionOptions,
    ) {
        self.imp()
            .connect_to_server(hostname, port, username, password, width, height, options);
    }

    pub fn disconnect(&self) {
        self.imp().disconnect();
    }

    /// Cuts the current reconnect countdown short.
    pub fn reconnect_now(&self) {
        self.imp().reconnect_now();
    }

    pub fn queue_resize_to_logical_size(&self, width: i32, height: i32) {
        self.imp().queue_resize_to_logical_size(width, height);
    }

    pub fn register_key_controller_on(&self, widget: &impl IsA<gtk::Widget>) {
        self.imp().register_key_controller_on(widget);
    }

    pub fn set_connection_options(&self, options: ConnectionOptions) {
        self.imp().apply_connection_options(options, true);
    }

    pub(in crate::rdp) fn announce_local_clipboard(&self) {
        self.imp().announce_local_clipboard();
    }
}

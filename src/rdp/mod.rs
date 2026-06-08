/* rdp/mod.rs
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

//! The RDP display widget. This module holds the `IronRdpWidget` GObject and
//! its GTK wiring; the pure logic it drives lives in the sibling submodules:
//! [`config`] (connection config), [`input`] (event translation), [`render`]
//! (framebuffer/cursor textures) and [`session`] (the worker thread).

mod config;
mod input;
mod render;
mod session;

use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp::cliprdr::pdu::{ClipboardFormat, ClipboardFormatId};
use ironrdp::input::Operation;
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpInputEvent, RdpOutputEvent};
use crate::clipboard::{self as clip, ClientClipboardMessageProxy, GtkCliprdrBackendFactory};
use gtk::glib::prelude::*;
use gtk::glib::subclass::Signal;
use gtk::glib::{self, Properties};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, OnceCell, RefCell};
use std::sync::OnceLock;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, glib::Enum, Default)]
#[enum_type(name = "RdpState")]
pub enum RdpState {
    #[default]
    Disconnected = 0,
    Connecting = 1,
    Connected = 2,
}

mod imp {
    use super::*;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::IronRdpWidget)]
    pub struct IronRdpWidget {
        #[property(get, set, builder(RdpState::Disconnected))]
        state: Cell<RdpState>,
        input_sender: RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpInputEvent>>>,
        texture: RefCell<Option<gdk::MemoryTexture>>,
        input_database: OnceCell<RefCell<ironrdp::input::Database>>,
        output_relay: OnceCell<async_channel::Sender<RdpOutputEvent>>,
        resize_timeout: RefCell<Option<glib::SourceId>>,
        clipboard_context: OnceCell<clip::SharedClipboardContext>,
        gtk_clipboard_tx: OnceCell<async_channel::Sender<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IronRdpWidget {
        const NAME: &'static str = "IronRdpWidget";
        type Type = super::IronRdpWidget;
        type ParentType = gtk::Widget;
    }

    impl IronRdpWidget {
        fn surface_scale(&self) -> f64 {
            self.obj()
                .native()
                .and_then(|n| n.surface())
                .map(|s| s.scale())
                .unwrap_or_else(|| self.obj().scale_factor() as f64)
        }

        /// Converts a logical size into physical (scaled) pixels plus the
        /// desktop scale factor in percent (e.g. `100` or `200`). Returns
        /// `None` if the scaled dimensions don't fit in `u16`.
        fn physical_size(&self, logical_width: f64, logical_height: f64) -> Option<(u16, u16, u32)> {
            let scale_f = self.surface_scale();
            let phys_width = u16::try_from((logical_width * scale_f).round() as i64).ok()?;
            let phys_height = u16::try_from((logical_height * scale_f).round() as i64).ok()?;
            let scale_factor = (scale_f * 100.0).round() as u32;
            Some((phys_width, phys_height, scale_factor))
        }

        pub fn connect_to_server(
            &self,
            hostname: String,
            port: u16,
            username: String,
            password: secrecy::SecretString,
            width: u16,
            height: u16,
        ) {
            info!("Connecting to {}:{} {}x{}", hostname, port, width, height);

            let Some((phys_width, phys_height, scale_factor)) =
                self.physical_size(width as f64, height as f64)
            else {
                return;
            };

            // Gracefully close any existing connection
            self.disconnect();

            self.obj().set_state(RdpState::Connecting);

            let Some(config) = config::build_config(
                hostname, port, username, password, phys_width, phys_height, scale_factor,
            ) else {
                return;
            };

            let (input_tx, input_rx) = RdpInputEvent::create_channel();
            let dvc_factory = DvcPipeProxyFactory::new(input_tx.clone());

            *self.input_sender.borrow_mut() = Some(input_tx.clone());

            // Wire up clipboard: update the shared context with the new connection's proxy.
            let cliprdr_factory: Option<Box<dyn ironrdp::cliprdr::backend::CliprdrBackendFactory + Send>> =
                if let (Some(ctx), Some(gtk_tx)) = (
                    self.clipboard_context.get(),
                    self.gtk_clipboard_tx.get(),
                ) {
                    let proxy = ClientClipboardMessageProxy::new(input_tx.clone());
                    ctx.lock().unwrap().proxy = Some(proxy);
                    Some(Box::new(GtkCliprdrBackendFactory::new(ctx.clone(), gtk_tx.clone())))
                } else {
                    None
                };

            let relay_tx = self.output_relay.get().unwrap().clone();

            session::spawn_rdp_session(config, input_rx, cliprdr_factory, dvc_factory, relay_tx);
        }

        pub fn disconnect(&self) {
            if let Some(sender) = self.input_sender.borrow().as_ref() {
                let _ = sender.send(RdpInputEvent::Close);
            }
        }

        fn process_message(&self, output_event: RdpOutputEvent) {
            match output_event {
                RdpOutputEvent::ConnectionFailure(reason) => {
                    self.obj().set_state(RdpState::Disconnected);
                    let mut error_string = reason.to_string();
                    let mut source = std::error::Error::source(&reason);
                    while let Some(e) = source {
                        error_string.push_str(&format!(": {e}"));
                        source = e.source();
                    }
                    self.obj()
                        .emit_by_name::<()>("connection-failed", &[&error_string]);
                }
                RdpOutputEvent::Terminated(Ok(reason)) => {
                    info!("Session terminated: {}", reason);
                    self.obj().set_state(RdpState::Disconnected);
                }
                RdpOutputEvent::Terminated(Err(e)) => {
                    warn!("Session error: {}", e);
                    self.obj().set_state(RdpState::Disconnected);
                }
                RdpOutputEvent::Image { buffer, width, height } => {
                    if self.state.get() == RdpState::Connecting {
                        info!("State connected (first frame received)");
                        self.obj().set_state(RdpState::Connected);
                    }
                    *self.texture.borrow_mut() = Some(render::image_texture(buffer, width, height));
                    self.obj().queue_draw();
                }
                RdpOutputEvent::PointerDefault => {
                    let cursor = gdk::Cursor::from_name("default", None);
                    self.obj().set_cursor(cursor.as_ref());
                }
                RdpOutputEvent::PointerHidden => {
                    let cursor = gdk::Cursor::from_name("none", None);
                    self.obj().set_cursor(cursor.as_ref());
                }
                RdpOutputEvent::PointerPosition { x, y } => {
                    debug!("PointerPosition {} {}", x, y);
                }
                RdpOutputEvent::PointerBitmap(pointer) => {
                    debug!(width = ?pointer.width, height = ?pointer.height, "Received pointer bitmap");
                    if let Some(cursor) = render::pointer_cursor(&pointer, self.surface_scale()) {
                        self.obj().set_cursor(Some(&cursor));
                    }
                }
            }
        }

        fn send_input_operation(&self, operation: Operation) {
            let input_events = self
                .input_database
                .get()
                .unwrap()
                .borrow_mut()
                .apply(core::iter::once(operation));
            self.send_fast_path_events(input_events);
        }

        fn send_fast_path_events(
            &self,
            input_events: smallvec::SmallVec<
                [ironrdp::pdu::input::fast_path::FastPathInputEvent; 2],
            >,
        ) {
            if !input_events.is_empty() {
                if let Some(sender) = self.input_sender.borrow().as_ref() {
                    let _ = sender.send(RdpInputEvent::FastPath(input_events));
                }
            }
        }

        fn setup_input_database(&self) {
            assert!(
                self.input_database
                    .set(RefCell::new(ironrdp::input::Database::new()))
                    .is_ok()
            );
        }

        /// Sets up the channel that relays output events from the RDP worker
        /// thread to `process_message` on the GTK main loop.
        fn setup_output_relay(&self) {
            let (relay_tx, relay_rx) = async_channel::bounded::<RdpOutputEvent>(64);
            self.output_relay.set(relay_tx).unwrap();

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                async move {
                    while let Ok(event) = relay_rx.recv().await {
                        imp.process_message(event);
                    }
                }
            ));
        }

        /// Sets up bidirectional clipboard sharing: a task that writes
        /// remote-originated text to the local clipboard, and a monitor that
        /// forwards local clipboard changes to the remote.
        fn setup_clipboard(&self) {
            let context = clip::new_shared_context();
            self.clipboard_context.set(context.clone()).ok();

            let (clip_tx, clip_rx) = async_channel::bounded::<String>(16);
            self.gtk_clipboard_tx.set(clip_tx).ok();

            // GTK task: receive text from remote and write it to the local clipboard.
            glib::spawn_future_local(async move {
                while let Ok(text) = clip_rx.recv().await {
                    if let Some(display) = gdk::Display::default() {
                        display.clipboard().set_text(&text);
                    }
                }
            });

            // Monitor local clipboard changes and forward to the remote.
            if let Some(display) = gdk::Display::default() {
                let context_for_clipboard = context.clone();
                display.clipboard().connect_changed(move |cb| {
                    let cb = cb.clone();
                    let ctx = context_for_clipboard.clone();
                    glib::spawn_future_local(async move {
                        match cb.read_text_future().await {
                            Ok(Some(text)) => {
                                let should_skip = {
                                    let mut c = ctx.lock().unwrap();
                                    if c.last_remote_text.as_deref() == Some(text.as_str()) {
                                        c.last_remote_text = None;
                                        true
                                    } else {
                                        false
                                    }
                                };
                                if should_skip {
                                    return;
                                }
                                let utf16: Vec<u8> = text
                                    .encode_utf16()
                                    .flat_map(|c| c.to_le_bytes())
                                    .chain([0u8, 0u8])
                                    .collect();
                                let proxy = {
                                    let mut c = ctx.lock().unwrap();
                                    c.local_text_utf16 = Some(utf16);
                                    c.proxy.clone()
                                };
                                if let Some(proxy) = proxy {
                                    proxy.send_clipboard_message(
                                        ClipboardMessage::SendInitiateCopy(vec![
                                            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
                                        ]),
                                    );
                                }
                            }
                            Ok(None) => {
                                ctx.lock().unwrap().local_text_utf16 = None;
                            }
                            Err(e) => {
                                debug!("Failed to read clipboard text: {e}");
                            }
                        }
                    });
                });
            }
        }

        /// Sets up the motion controller: forwards pointer motion to the remote
        /// and grabs focus / inhibits system shortcuts while the pointer is over
        /// the connected session.
        fn setup_motion_controller(&self) {
            let event_controller_motion = gtk::EventControllerMotion::new();
            event_controller_motion.connect_motion(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                move |_controller, x, y| {
                    if imp.state.get() != RdpState::Connected {
                        return;
                    }
                    let scale = imp.surface_scale();
                    let operation =
                        Operation::MouseMove(ironrdp::input::MousePosition {
                            x: (x * scale) as u16,
                            y: (y * scale) as u16,
                        });
                    imp.send_input_operation(operation);
                }
            ));
            event_controller_motion.connect_enter(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                move |_controller, _x, _y| {
                    if imp.state.get() != RdpState::Connected {
                        return;
                    }
                    let obj = imp.obj();
                    obj.grab_focus();
                    crate::utils::set_shortcuts_inhibited(&*obj, true);
                }
            ));
            event_controller_motion.connect_leave(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                move |_controller| {
                    let obj = imp.obj();
                    crate::utils::set_shortcuts_inhibited(&*obj, false);
                    if let Some(root) = obj.root() {
                        root.set_focus(None::<&gtk::Widget>);
                    }
                }
            ));
            self.obj().add_controller(event_controller_motion);
        }

        /// Sets up the legacy event controller translating mouse button, key and
        /// scroll events into RDP input operations.
        fn setup_input_controller(&self) {
            let event_controller = gtk::EventControllerLegacy::new();
            event_controller.connect_event(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_controller, event: &gdk::Event| -> glib::Propagation {
                    match event.event_type() {
                        gdk::EventType::ButtonRelease | gdk::EventType::ButtonPress => {
                            let button_event =
                                event.clone().downcast::<gdk::ButtonEvent>().unwrap();
                            let Some(mouse_button) = input::mouse_button(button_event.button())
                            else {
                                return glib::Propagation::Proceed;
                            };
                            let operation = match event.event_type() {
                                gdk::EventType::ButtonPress => {
                                    Operation::MouseButtonPressed(mouse_button)
                                }
                                gdk::EventType::ButtonRelease => {
                                    Operation::MouseButtonReleased(mouse_button)
                                }
                                _ => {
                                    return glib::Propagation::Proceed;
                                }
                            };
                            imp.send_input_operation(operation);
                            glib::Propagation::Stop
                        }
                        gdk::EventType::KeyPress | gdk::EventType::KeyRelease => {
                            let key_event = event.clone().downcast::<gdk::KeyEvent>().unwrap();
                            let keycode: u16 = key_event.keycode().try_into().unwrap();
                            let Some(scancode) = input::key_scancode(keycode) else {
                                return glib::Propagation::Proceed;
                            };
                            let operation = match event.event_type() {
                                gdk::EventType::KeyPress => {
                                    Operation::KeyPressed(scancode)
                                }
                                gdk::EventType::KeyRelease => {
                                    Operation::KeyReleased(scancode)
                                }
                                _ => {
                                    return glib::Propagation::Proceed;
                                }
                            };
                            imp.send_input_operation(operation);
                            glib::Propagation::Stop
                        }
                        gdk::EventType::Scroll => {
                            let scroll_event =
                                event.clone().downcast::<gdk::ScrollEvent>().unwrap();
                            if scroll_event.is_stop() {
                                return glib::Propagation::Proceed;
                            }
                            let (dx, dy) = scroll_event.deltas();
                            for operation in input::scroll_operations(dx, dy) {
                                imp.send_input_operation(operation);
                            }
                            glib::Propagation::Stop
                        }
                        _ => glib::Propagation::Proceed,
                    }
                }
            ));
            self.obj().add_controller(event_controller);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for IronRdpWidget {
        fn constructed(&self) {
            self.parent_constructed();

            self.setup_input_database();
            self.setup_output_relay();
            self.setup_clipboard();
            self.setup_motion_controller();
            self.setup_input_controller();
            self.obj().set_focusable(true);
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
    }

    impl WidgetImpl for IronRdpWidget {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);

            if self.state.get() != RdpState::Connected {
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
                        let Some((phys_width, phys_height, scale_factor)) =
                            imp.physical_size(width as f64, height as f64)
                        else {
                            return;
                        };
                        if let Some(sender) = imp.input_sender.borrow().as_ref() {
                            let _ = sender.send(RdpInputEvent::Resize {
                                width: phys_width,
                                height: phys_height,
                                scale_factor,
                                physical_size: None,
                            });
                        }
                    }
                ),
            );

            *self.resize_timeout.borrow_mut() = Some(source_id);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let w = self.obj().width() as f32;
            let h = self.obj().height() as f32;
            if let Some(texture) = self.texture.borrow().as_ref() {
                snapshot.append_texture(
                    texture,
                    &gtk::graphene::Rect::new(0.0, 0.0, w, h),
                );
            } else {
                snapshot.append_color(
                    &gdk::RGBA::BLACK,
                    &gtk::graphene::Rect::new(0.0, 0.0, w, h),
                );
            }

            self.parent_snapshot(snapshot)
        }
    }
}

glib::wrapper! {
    pub struct IronRdpWidget(ObjectSubclass<imp::IronRdpWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl IronRdpWidget {
    pub fn connect_to_server(
        &self,
        hostname: String,
        port: u16,
        username: String,
        password: secrecy::SecretString,
        width: u16,
        height: u16,
    ) {
        self.imp()
            .connect_to_server(hostname, port, username, password, width, height);
    }

    pub fn disconnect(&self) {
        self.imp().disconnect();
    }
}

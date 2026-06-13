/* window.rs
 *
 * Copyright 2025 Florian Richter
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
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use secrecy::SecretString;

use std::cell::RefCell;

use crate::model::destination_object::DestinationObject;
use crate::rdp::{IronRdpWidget, RdpState};
use crate::destinations_page::LlDestinationPage;
use crate::password_dialog::LongLensPasswordDialog;

fn stack_page(state: RdpState, n_destinations: u32) -> &'static str {
    if state == RdpState::Connected {
        "rdppage"
    } else if state == RdpState::Connecting {
        "connectingpage"
    } else if n_destinations == 0 {
        "emptypage"
    } else {
        "destinationspage"
    }
}

fn parse_domain_port(input: &str) -> (String, u16) {
    let mut parts = input.splitn(2, ':');
    let domain = parts.next().unwrap_or("").to_string();
    let port = parts
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3389);
    (domain, port)
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/window.ui")]
    pub struct LongLensWindow {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub destinations_page: TemplateChild<LlDestinationPage>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub disconnectbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub adddestinationbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub fullscreenbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub fullscreen_bar_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub fullscreen_bar: TemplateChild<gtk::Box>,
        #[template_child]
        pub fullscreen_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub rdpwidget: TemplateChild<IronRdpWidget>,
        pub connection_display_title: RefCell<String>,
        pub hide_timer: RefCell<Option<glib::SourceId>>,
        pub bar_motion: RefCell<Option<gtk::EventControllerMotion>>,
    }
    #[gtk::template_callbacks]
    impl LongLensWindow {
        #[template_callback]
        fn handle_disconnectbutton_clicked(&self, _button: &gtk::Button) {
            self.rdpwidget.disconnect();
        }

        #[template_callback]
        fn handle_adddestinationbutton_clicked(&self, _button: &gtk::Button) {
            self.destinations_page.show_add_dialog();
        }

        #[template_callback]
        fn handle_connection_failed(&self, reason: String) {
            let dialog = adw::AlertDialog::new(Some(&gettext("Connection error")), Some(&reason));
            dialog.add_response("close", &gettext("Close"));
            dialog.present(Some(&*self.obj()));
        }

        #[template_callback]
        fn handle_fullscreenbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().fullscreen();
        }

        #[template_callback]
        fn handle_leavefullscreenbutton_clicked(&self, _button: &gtk::Button) {
            self.obj().unfullscreen();
        }

        #[template_callback]
        fn handle_closebutton_clicked(&self, _button: &gtk::Button) {
            self.obj().close();
        }
    }

    impl LongLensWindow {
        /// Reveals the auto-hiding fullscreen bar and (re)starts the hide timer.
        fn reveal_bar(&self) {
            self.fullscreen_bar_revealer.set_reveal_child(true);
            self.schedule_hide();
        }

        /// Cancels any pending hide and schedules the bar to hide after a delay.
        fn schedule_hide(&self) {
            self.cancel_hide();
            let source = glib::timeout_add_local_once(
                std::time::Duration::from_secs(3),
                glib::clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        imp.hide_timer.borrow_mut().take();
                        // Don't hide while the pointer is still hovering the bar;
                        // re-arm instead. This also covers the case where the bar
                        // was revealed under a motionless pointer (edge trigger),
                        // which never emits an `enter` event.
                        let hovered = imp
                            .bar_motion
                            .borrow()
                            .as_ref()
                            .is_some_and(|c| c.contains_pointer());
                        if hovered {
                            imp.schedule_hide();
                        } else {
                            imp.fullscreen_bar_revealer.set_reveal_child(false);
                        }
                    }
                ),
            );
            *self.hide_timer.borrow_mut() = Some(source);
        }

        /// Cancels a pending hide timer, if any.
        fn cancel_hide(&self) {
            if let Some(source) = self.hide_timer.borrow_mut().take() {
                source.remove();
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LongLensWindow {
        const NAME: &'static str = "LongLensWindow";
        type Type = super::LongLensWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LongLensWindow {
        fn constructed(&self) {
            self.parent_constructed();

            self.rdpwidget
                .bind_property::<gtk::Button>("state", self.disconnectbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state == RdpState::Connected || state == RdpState::Connecting)
                })
                .sync_create()
                .build();
            self.rdpwidget
                .bind_property::<gtk::Button>("state", self.adddestinationbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state != RdpState::Connected && state != RdpState::Connecting)
                })
                .sync_create()
                .build();
            self.rdpwidget
                .bind_property::<gtk::Button>("state", self.fullscreenbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state == RdpState::Connected)
                })
                .sync_create()
                .build();
            self.obj()
                .bind_property::<gtk::Label>("title", self.fullscreen_title.as_ref(), "label")
                .sync_create()
                .build();

            // Hide the headerbar and reveal the auto-hiding control bar while
            // fullscreen; restore the headerbar when leaving fullscreen.
            self.obj().connect_fullscreened_notify(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |obj| {
                    let fullscreen = obj.is_fullscreen();
                    window.headerbar.set_visible(!fullscreen);
                    if fullscreen {
                        window.reveal_bar();
                    } else {
                        window.cancel_hide();
                        window.fullscreen_bar_revealer.set_reveal_child(false);
                    }
                }
            ));

            // Re-reveal the bar when the pointer hits the top-center edge.
            let edge_motion = gtk::EventControllerMotion::new();
            edge_motion.set_propagation_phase(gtk::PropagationPhase::Capture);
            edge_motion.connect_motion(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_controller, x, y| {
                    let obj = window.obj();
                    if !obj.is_fullscreen() {
                        return;
                    }
                    let width = obj.width() as f64;
                    if y <= 5.0 && (x - width / 2.0).abs() <= 150.0 {
                        window.reveal_bar();
                    }
                }
            ));
            self.obj().add_controller(edge_motion);

            // Keep the bar visible while the pointer hovers it.
            let bar_motion = gtk::EventControllerMotion::new();
            bar_motion.connect_enter(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_controller, _x, _y| {
                    window.cancel_hide();
                }
            ));
            bar_motion.connect_leave(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_controller| {
                    window.schedule_hide();
                }
            ));
            self.fullscreen_bar.add_controller(bar_motion.clone());
            *self.bar_motion.borrow_mut() = Some(bar_motion);

            self.rdpwidget.connect_state_notify(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |widget| {
                    let obj = window.obj();
                    let n = window.destinations_page.destinations().n_items();
                    window.stack.set_visible_child_name(stack_page(widget.state(), n));
                    let state = widget.state();
                    if state == RdpState::Connected || state == RdpState::Connecting {
                        let display_title = window.connection_display_title.borrow().clone();
                        obj.set_title(Some(&display_title));
                        crate::utils::set_shortcuts_inhibited(&*obj, state == RdpState::Connected);
                    } else {
                        obj.set_title(Some(&gettext("Long Lens")));
                        crate::utils::set_shortcuts_inhibited(&*obj, false);
                        if obj.is_fullscreen() {
                            obj.unfullscreen();
                        }
                    }
                }
            ));
            let model = self.destinations_page.destinations();
            self.stack.set_visible_child_name(stack_page(RdpState::default(), model.n_items()));
            model.connect_items_changed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |model, _, _, _| {
                    let state = window.rdpwidget.state();
                    window.stack.set_visible_child_name(stack_page(state, model.n_items()));
                }
            ));
            self.obj().setup_actions();
        }
    }
    impl WidgetImpl for LongLensWindow {}
    impl WindowImpl for LongLensWindow {
        fn close_request(&self) -> glib::Propagation {
            let state = self.rdpwidget.state();
            if state != RdpState::Connected && state != RdpState::Connecting {
                return self.parent_close_request();
            }

            let dialog = adw::AlertDialog::new(
                Some(&gettext("Disconnect?")),
                Some(&gettext("You are currently connected to a remote session. Do you want to disconnect and close?")),
            );
            dialog.add_response("cancel", &gettext("Cancel"));
            dialog.add_response("disconnect", &gettext("Disconnect"));
            dialog.set_response_appearance("disconnect", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("disconnect"));
            dialog.set_close_response("cancel");

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = window)]
                self,
                async move {
                    let response = dialog.choose_future(Some(&*window.obj())).await;
                    if response == "disconnect" {
                        window.rdpwidget.disconnect();
                        window.obj().destroy();
                    }
                }
            ));

            glib::Propagation::Stop
        }
    }
    impl ApplicationWindowImpl for LongLensWindow {}
    impl AdwApplicationWindowImpl for LongLensWindow {}
}

glib::wrapper! {
    pub struct LongLensWindow(ObjectSubclass<imp::LongLensWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl LongLensWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn setup_actions(&self) {
        let action_connect = gio::ActionEntry::builder("connect")
            .parameter_type(Some(glib::VariantTy::new("s").unwrap()))
            .activate(move |window: &Self, _action, parameter| {
                let uuid: String = parameter.unwrap().get().unwrap();
                let destinations = window.imp().destinations_page.destinations();
                let Some(dest) = destinations
                    .iter::<DestinationObject>()
                    .filter_map(|r| r.ok())
                    .find(|d| d.uuid() == uuid)
                else {
                    return;
                };
                let hostname = dest.hostname();
                let username = dest.username();
                let display_title = dest.property::<String>("display-title");
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    window,
                    async move {
                        match crate::secrets::get_password(&uuid).await {
                            Some(password) => {
                                window.start_connection(hostname, username, password, display_title);
                            }
                            None => {
                                let dialog = LongLensPasswordDialog::new();
                                dialog.set_hostname(&hostname);
                                dialog.set_username(&username);
                                dialog.set_on_connect(glib::clone!(
                                    #[weak]
                                    window,
                                    move |password| {
                                        window.start_connection(hostname.clone(), username.clone(), password, display_title.clone());
                                    }
                                ));
                                dialog.present(Some(&window));
                            }
                        }
                    }
                ));
            })
            .build();
        self.add_action_entries([action_connect]);
    }

    fn start_connection(&self, hostname: String, username: String, password: SecretString, display_title: String) {
        *self.imp().connection_display_title.borrow_mut() = display_title;
        let (server, port) = parse_domain_port(&hostname);
        let w = self.imp().stack.width();
        let h = self.imp().stack.height();
        let width = if w > 0 { w as u16 } else { 1280 };
        let height = if h > 0 { h as u16 } else { 800 };
        self.imp().rdpwidget
            .connect_to_server(server, port, username, password, width, height);
    }
}


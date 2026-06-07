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
use gtk::{gio, glib};
use secrecy::SecretString;

use std::cell::RefCell;

use crate::model::destination_object::DestinationObject;
use crate::ironrdpwidget::{IronRdpWidget, RdpState};
use crate::destinations_page::LlDestinationPage;
use crate::password_dialog::LongLensPasswordDialog;

fn stack_page(state: RdpState, n_destinations: u32) -> &'static str {
    if state == RdpState::Connected {
        "rdppage"
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
        pub disconnectbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub adddestinationbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub rdpwidget: TemplateChild<IronRdpWidget>,
        pub connection_display_title: RefCell<String>,
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
            let dialog = adw::AlertDialog::new(Some("Connection error"), Some(&reason));
            dialog.add_response("close", "Close");
            dialog.present(Some(&*self.obj()));
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
                    Some(state == RdpState::Connected)
                })
                .sync_create()
                .build();
            self.rdpwidget
                .bind_property::<gtk::Button>("state", self.adddestinationbutton.as_ref(), "visible")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(state != RdpState::Connected)
                })
                .sync_create()
                .build();
            self.rdpwidget.connect_state_notify(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |widget| {
                    let obj = window.obj();
                    let n = window.destinations_page.destinations().n_items();
                    window.stack.set_visible_child_name(stack_page(widget.state(), n));
                    if widget.state() == RdpState::Connected {
                        let display_title = window.connection_display_title.borrow().clone();
                        obj.set_title(Some(&display_title));
                        obj.surface()
                            .as_ref()
                            .and_then(|s| s.downcast_ref::<gdk::Toplevel>())
                            .inspect(|t| t.inhibit_system_shortcuts(None::<gdk::Event>));
                    } else {
                        obj.set_title(Some("Long Lens"));
                        obj.surface()
                            .as_ref()
                            .and_then(|s| s.downcast_ref::<gdk::Toplevel>())
                            .inspect(|t| t.restore_system_shortcuts());
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
    impl WindowImpl for LongLensWindow {}
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
                                dialog.set_destination_name(&display_title);
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


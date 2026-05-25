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

use std::cell::RefCell;
use std::rc::Rc;

use crate::destinations::Destinations;
use crate::ironrdpwidget::{IronRdpWidget, RdpState};
use crate::destinations_page::LlDestinationPage;

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
            let dialog = adw::AlertDialog::new(Some("Connction error"), Some(&reason));
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
            // bind page to connection state
            self.rdpwidget
                .bind_property::<gtk::Stack>("state", self.stack.as_ref(), "visible-child-name")
                .transform_to(|_binding, value: glib::Value| {
                    let state = value.get::<RdpState>().unwrap_or_default();
                    Some(if state == RdpState::Connected {
                        "rdppage"
                    } else {
                        "destinationspage"
                    })
                })
                .sync_create()
                .build();
            self.rdpwidget.connect_state_notify(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |widget| {
                    if widget.state() != RdpState::Connected { return };
                    window
                        .obj()
                        .surface()
                        .as_ref()
                        .and_then(|s| s.downcast_ref::<gdk::Toplevel>())
                        .inspect(|t| t.inhibit_system_shortcuts(None::<gdk::Event>));
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
    pub fn set_destinations(&self, destinations: Rc<RefCell<Destinations>>) {
        self.imp().destinations_page.set_destinations(destinations);
    }

    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn setup_actions(&self) {
        let action_quick_connect = gio::ActionEntry::builder("quick-connect")
            .activate(move |window: &Self, _action, _parameter| {
                let (server, port, username, password) = window.imp().destinations_page.get_quick_connect_data();
                let width = window.imp().stack.width() as u16;
                let height = window.imp().stack.height() as u16;
                window.imp().rdpwidget
                    .connect_to_server(server, port, username, password, width, height);
            })
            .build();
        self.add_action_entries([action_quick_connect]);
    }
}


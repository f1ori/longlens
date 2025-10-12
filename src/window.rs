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
use std::cell::OnceCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::destination_object::{DestinationData, DestinationObject};
use crate::ironrdpwidget::IronRdpWidget;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/fernsichtrdp/window.ui")]
    pub struct FernsichtRdpWindow {
        // Template widgets
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub hostnameentry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub usernameentry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub passwordentry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub destinations_list: TemplateChild<adw::PreferencesGroup>,
        pub destinations: OnceCell<gio::ListStore>,
        #[template_child]
        pub rdpwidget: TemplateChild<IronRdpWidget>,
    }

    #[gtk::template_callbacks]
    impl FernsichtRdpWindow {
        #[template_callback]
        fn handle_connectbutton_activated(&self, _button: &adw::ButtonRow) {
            self.stack.set_visible_child_name("connecting");
            self.rdpwidget.connect_to_server(
                String::from("localhost"),
                String::from("flo"),
                String::from("flo"),
            );
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FernsichtRdpWindow {
        const NAME: &'static str = "FernsichtRdpWindow";
        type Type = super::FernsichtRdpWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FernsichtRdpWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_destinations();
            self.obj().load_destinations();
        }
    }
    impl WidgetImpl for FernsichtRdpWindow {}
    impl WindowImpl for FernsichtRdpWindow {}
    impl ApplicationWindowImpl for FernsichtRdpWindow {}
    impl AdwApplicationWindowImpl for FernsichtRdpWindow {}
}

glib::wrapper! {
    pub struct FernsichtRdpWindow(ObjectSubclass<imp::FernsichtRdpWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl FernsichtRdpWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn destinations(&self) -> gio::ListStore {
        self.imp()
            .destinations
            .get()
            .expect("`destinations` should be set in `setup_destinations`.")
            .clone()
    }

    pub fn setup_destinations(&self) {
        let model = gio::ListStore::new::<DestinationObject>();
        self.imp()
            .destinations
            .set(model.clone())
            .expect("Could not set destinations");

        self.imp().destinations_list.bind_model(
            Some(&model),
            Some(Box::new(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or_panic]
                move |obj| {
                    let destination_object = obj
                        .downcast_ref::<DestinationObject>()
                        .expect("The object should be of type `DestinationObject`.");
                    let row = window.create_destination_row(destination_object);
                    row.upcast()
                }
            ))),
        )
    }

    pub fn load_destinations(&self) {
        let data: Vec<DestinationData> = vec![
            DestinationData {
                hostname: String::from("localhost"),
                username: String::from("flo"),
            },
            DestinationData {
                hostname: String::from("testserver"),
                username: String::from("flo"),
            },
        ];
        let destination_objects: Vec<DestinationObject> = data
            .into_iter()
            .map(DestinationObject::from_destination_data)
            .collect();
        self.destinations().extend_from_slice(&destination_objects);
    }

    fn create_destination_row(&self, destination_object: &DestinationObject) -> adw::ActionRow {
        let row = adw::ActionRow::builder().build();

        destination_object
            .bind_property("hostname", &row, "title")
            .sync_create()
            .build();
        destination_object
            .bind_property("username", &row, "subtitle")
            .transform_to(|_binding, value: glib::Value| {
                let text = value.get::<String>().unwrap_or_default();
                Some(format!("User: {}", text).to_value())
            })
            .sync_create()
            .build();
        row
    }
}

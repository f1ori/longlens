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
use std::fs::File;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::destination_object::{DestinationData, DestinationObject};
use crate::ironrdpwidget::IronRdpWidget;
use crate::utils::data_path;

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
        pub destinations_list: TemplateChild<gtk::ListBox>,
        pub destinations: OnceCell<gio::ListStore>,
        #[template_child]
        pub rdpwidget: TemplateChild<IronRdpWidget>,
    }

    #[gtk::template_callbacks]
    impl FernsichtRdpWindow {
        #[template_callback]
        fn handle_entries_activated(&self, _entry: &adw::EntryRow) {
            self.connect();
        }

        #[template_callback]
        fn handle_connectbutton_activated(&self, _button: &adw::ButtonRow) {
            self.connect();
        }

        fn connect(&self) {
            let hostname = self.hostnameentry.text().to_string();
            let username = self.usernameentry.text().to_string();
            let password = self.passwordentry.text().to_string();
            self.stack.set_visible_child_name("connecting");
            self.obj()
                .add_destination(hostname.clone(), username.clone());
            self.obj().save_destinations();
            self.rdpwidget
                .connect_to_server(hostname, username, password);
        }
        #[template_callback]
        fn handle_destinationslist_rowactivated(&self, row: &gtk::ListBoxRow) {
            let index = row.index();
            let destination = self
                .obj()
                .destinations()
                .item(index as u32)
                .expect("There needs to be an object at this position.")
                .downcast::<DestinationObject>()
                .expect("The object needs to be a `DestinationObject`.");
            self.hostnameentry.set_text(&destination.hostname());
            self.usernameentry.set_text(&destination.username());
            self.passwordentry.grab_focus();
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
            glib::clone!(
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
            ),
        )
    }

    pub fn load_destinations(&self) {
        if let Ok(file) = File::open(data_path()) {
            let data: Vec<DestinationData> = serde_json::from_reader(file)
                .expect("It should be possible to read the connections from the json file.");
            let destination_objects: Vec<DestinationObject> = data
                .into_iter()
                .map(DestinationObject::from_destination_data)
                .collect();
            self.destinations().extend_from_slice(&destination_objects);
        }
    }

    pub fn add_destination(&self, hostname: String, username: String) {
        let found = self
            .destinations()
            .iter::<DestinationObject>()
            .filter_map(|destination_object| destination_object.ok())
            .map(|destination_object| destination_object.destination_data())
            .find(|o| o.hostname == hostname && o.username == username);
        if found.is_none() {
            let destination = DestinationObject::new(hostname, username);
            self.destinations().append(&destination);
        }
    }

    pub fn save_destinations(&self) {
        // Store task data in vector
        let data: Vec<DestinationData> = self
            .destinations()
            .iter::<DestinationObject>()
            .filter_map(|destination_object| destination_object.ok())
            .map(|destination_object| destination_object.destination_data())
            .collect();
        let file = File::create(data_path()).expect("Could not create json file.");
        serde_json::to_writer(file, &data).expect("Could not write data to json file");
    }

    fn create_destination_row(&self, destination_object: &DestinationObject) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .activatable(true)
            .selectable(false)
            .build();

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


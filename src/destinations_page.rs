/* 
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
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::destination_dialog::LongLensDestinationDialog;
use crate::destination_object::DestinationObject;
use crate::destinations::Destinations;


mod imp {
    use super::*;


    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/destinations_page.ui")]
    pub struct LlDestinationPage {
        #[template_child]
        pub destinations_list: TemplateChild<gtk::ListBox>,
        pub destinations: OnceCell<gio::ListStore>,
        pub dialog: OnceCell<LongLensDestinationDialog>,
        pub destinations_data: OnceCell<Rc<RefCell<Destinations>>>,
    }
    #[gtk::template_callbacks]
    impl LlDestinationPage {
        #[template_callback]
        fn handle_destinationslist_rowactivated(&self, row: &gtk::ListBoxRow) {
            let destination = self
                .obj()
                .destinations()
                .item(row.index() as u32)
                .expect("There needs to be an object at this position.")
                .downcast::<DestinationObject>()
                .expect("The object needs to be a `DestinationObject`.");
            let variant = (destination.hostname(), destination.username(), String::new()).to_variant();
            self.obj()
                .activate_action("win.connect", Some(&variant))
                .expect("win.connect action failed");
        }

        #[template_callback]
        fn handle_connection_failed(&self, reason: String) {
            let dialog = adw::AlertDialog::new(Some("Connection error"), Some(&reason));
            dialog.add_response("close", "Close");
            dialog.present(Some(&*self.obj()));
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LlDestinationPage {
        const NAME: &'static str = "LlDestinationPage";
        type Type = super::LlDestinationPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LlDestinationPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_destinations();
            self.dialog
                .set(LongLensDestinationDialog::new())
                .expect("Could not set dialog");
        }
    }
    impl WidgetImpl for LlDestinationPage {}
    impl BinImpl for LlDestinationPage {}
}

glib::wrapper! {
    pub struct LlDestinationPage(ObjectSubclass<imp::LlDestinationPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LlDestinationPage {
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

    pub fn set_destinations(&self, destinations: Rc<RefCell<Destinations>>) {
        self.imp()
            .destinations_data
            .set(destinations)
            .expect("Could not set destinations_data");
        self.load_destinations();
    }

    pub fn load_destinations(&self) {
        let data = self.imp().destinations_data.get().unwrap();
        let destination_objects: Vec<DestinationObject> = data
            .borrow()
            .items()
            .iter()
            .cloned()
            .map(DestinationObject::from_destination_data)
            .collect();
        self.destinations().extend_from_slice(&destination_objects);
    }

    pub fn add_destination(&self, name: String, hostname: String, username: String) {
        let found = self
            .destinations()
            .iter::<DestinationObject>()
            .filter_map(|destination_object| destination_object.ok())
            .map(|destination_object| destination_object.destination_data())
            .find(|o| o.hostname == hostname && o.username == username);
        if found.is_none() {
            let destination = DestinationObject::new(name, hostname, username);
            self.destinations().append(&destination);
        }
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

        let edit_button = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .valign(gtk::Align::Center)
            .build();
        edit_button.add_css_class("flat");

        edit_button.connect_clicked(glib::clone!(
            #[weak]
            destination_object,
            #[weak(rename_to = page)]
            self,
            move |_button| {
                let destinations = page.imp().destinations_data.get().unwrap().clone();
                let dialog = LongLensDestinationDialog::new();
                dialog.imp().nameentry.set_text(&destination_object.name());
                dialog.imp().hostnameentry.set_text(&destination_object.hostname());
                dialog.imp().usernameentry.set_text(&destination_object.username());
                dialog.set_edit_mode(true);
                dialog.set_on_save(glib::clone!(
                    #[weak]
                    destination_object,
                    move |name, hostname, username| {
                        destination_object.set_name(name.clone());
                        destination_object.set_hostname(hostname.clone());
                        destination_object.set_username(username.clone());
                        destinations.borrow_mut().update(&destination_object.uuid(), name, hostname, username);
                    }
                ));
                dialog.set_on_delete(glib::clone!(
                    #[weak]
                    destination_object,
                    #[weak(rename_to = page)]
                    page,
                    move || {
                        let uuid = destination_object.uuid();
                        let model = page.destinations();
                        let pos = model
                            .iter::<DestinationObject>()
                            .enumerate()
                            .find_map(|(i, obj)| {
                                obj.ok().filter(|o| o.uuid() == uuid).map(|_| i as u32)
                            });
                        if let Some(pos) = pos {
                            model.remove(pos);
                        }
                        page.imp()
                            .destinations_data
                            .get()
                            .unwrap()
                            .borrow_mut()
                            .remove(&uuid);
                    }
                ));
                dialog.present(Some(&page));
            }
        ));

        row.add_suffix(&edit_button);
        row
    }

    pub fn show_add_dialog(&self) {
        let dialog = self.imp().dialog.get().expect("Dialog should be initialized");
        dialog.set_on_connect(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |name, hostname, username| {
                page.add_destination(name.clone(), hostname.clone(), username.clone());
                page.imp().destinations_data.get().unwrap()
                    .borrow_mut()
                    .add(name, hostname, username);
            }
        ));
        dialog.present(Some(self));
    }
}


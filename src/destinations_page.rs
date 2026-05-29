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
use crate::destination_object::{DestinationData, DestinationObject};
use crate::destinations::Destinations;
use crate::secrets;


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
            let password = secrets::get_password(&destination.uuid()).unwrap_or_default();
            let variant = (destination.hostname(), destination.username(), password).to_variant();
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

    /// Add a destination to both the list model and persistent storage.
    /// Returns the UUID if added (None if a duplicate hostname+username already exists).
    pub fn add_destination(&self, name: String, hostname: String, username: String) -> Option<String> {
        let destinations = self.imp().destinations_data.get().unwrap();
        let uuid = destinations.borrow_mut().add(name.clone(), hostname.clone(), username.clone())?;
        let data = DestinationData { uuid: uuid.clone(), name, hostname, username };
        self.destinations().append(&DestinationObject::from_destination_data(data));
        Some(uuid)
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

        let menu = gio::Menu::new();
        menu.append(Some("Edit"), Some("row.edit"));
        menu.append(Some("Delete"), Some("row.delete"));

        let menu_button = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu)
            .valign(gtk::Align::Center)
            .build();
        menu_button.add_css_class("flat");

        let action_group = gio::SimpleActionGroup::new();

        let edit_action = gio::SimpleAction::new("edit", None);
        edit_action.connect_activate(glib::clone!(
            #[weak]
            destination_object,
            #[weak(rename_to = page)]
            self,
            move |_, _| {
                let destinations = page.imp().destinations_data.get().unwrap().clone();
                let dialog = LongLensDestinationDialog::new();
                dialog.imp().nameentry.set_text(&destination_object.name());
                dialog.imp().hostnameentry.set_text(&destination_object.hostname());
                dialog.imp().usernameentry.set_text(&destination_object.username());
                // Pre-fill stored password if available
                if let Some(pw) = secrets::get_password(&destination_object.uuid()) {
                    dialog.imp().passwordentry.set_text(&pw);
                } else {
                    dialog.imp().rememberpasswordswitch.set_active(false);
                }
                dialog.set_edit_mode(true);
                dialog.set_on_save(glib::clone!(
                    #[weak]
                    destination_object,
                    move |name, hostname, username, password, remember| {
                        destination_object.set_name(name.clone());
                        destination_object.set_hostname(hostname.clone());
                        destination_object.set_username(username.clone());
                        destinations.borrow_mut().update(&destination_object.uuid(), name, hostname, username);
                        let uuid = destination_object.uuid();
                        if remember && !password.is_empty() {
                            secrets::store_password(&uuid, &password);
                        } else {
                            secrets::delete_password(&uuid);
                        }
                    }
                ));
                dialog.set_on_delete(glib::clone!(
                    #[weak]
                    destination_object,
                    #[weak(rename_to = page)]
                    page,
                    move || {
                        let uuid = destination_object.uuid();
                        secrets::delete_password(&uuid);
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
        action_group.add_action(&edit_action);

        let delete_action = gio::SimpleAction::new("delete", None);
        delete_action.connect_activate(glib::clone!(
            #[weak]
            destination_object,
            #[weak(rename_to = page)]
            self,
            move |_, _| {
                let alert = adw::AlertDialog::new(
                    Some("Delete Destination?"),
                    Some("This action cannot be undone."),
                );
                alert.add_response("cancel", "Cancel");
                alert.add_response("delete", "Delete");
                alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                alert.set_default_response(Some("cancel"));
                alert.set_close_response("cancel");

                alert.connect_response(None, glib::clone!(
                    #[weak]
                    destination_object,
                    #[weak]
                    page,
                    move |_, response| {
                        if response == "delete" {
                            let uuid = destination_object.uuid();
                            secrets::delete_password(&uuid);
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
                    }
                ));

                alert.present(Some(&page));
            }
        ));
        action_group.add_action(&delete_action);

        row.insert_action_group("row", Some(&action_group));
        row.add_suffix(&menu_button);
        row
    }

    pub fn show_add_dialog(&self) {
        let dialog = self.imp().dialog.get().expect("Dialog should be initialized");
        dialog.set_on_connect(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |name, hostname, username, password, remember| {
                if let Some(uuid) = page.add_destination(name, hostname, username) {
                    if remember && !password.is_empty() {
                        secrets::store_password(&uuid, &password);
                    }
                }
            }
        ));
        dialog.present(Some(self));
    }
}

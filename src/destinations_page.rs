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
use std::cell::Cell;
use std::cell::OnceCell;
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use secrecy::ExposeSecret;

use crate::destination_dialog::LongLensDestinationDialog;
use crate::destination_object::{DestinationData, DestinationObject};
use crate::destination_row::LlDestinationRow;
use crate::destinations::Destinations;
use crate::secrets;


mod imp {
    use super::*;


    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/destinations_page.ui")]
    pub struct LlDestinationPage {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub destinations_list: TemplateChild<gtk::ListBox>,
        pub destinations: OnceCell<gio::ListStore>,
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
            self.obj()
                .activate_action("win.connect", Some(&destination.uuid().to_variant()))
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
    pub fn destinations(&self) -> gio::ListStore {
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

    fn create_destination_row(&self, destination_object: &DestinationObject) -> LlDestinationRow {
        let row = LlDestinationRow::new();

        destination_object
            .bind_property("display-title", &row, "title")
            .sync_create()
            .build();
        destination_object
            .bind_property("display-subtitle", &row, "subtitle")
            .sync_create()
            .build();

        row.imp().edit_button.connect_clicked(glib::clone!(
            #[weak]
            destination_object,
            #[weak(rename_to = page)]
            self,
            move |_| {
                let destinations = page.imp().destinations_data.get().unwrap().clone();
                let dialog = LongLensDestinationDialog::new();
                dialog.imp().nameentry.set_text(&destination_object.name());
                dialog.imp().hostnameentry.set_text(&destination_object.hostname());
                dialog.imp().usernameentry.set_text(&destination_object.username());
                // Pre-fill stored password if available
                if let Some(pw) = secrets::get_password(&destination_object.uuid()) {
                    dialog.imp().passwordentry.set_text(pw.expose_secret());
                } else {
                    dialog.imp().rememberpasswordswitch.set_active(false);
                }
                dialog.set_edit_mode(true);
                dialog.set_on_save(glib::clone!(
                    #[weak]
                    destination_object,
                    #[upgrade_or_default]
                    move |name, hostname, username, password, remember| {
                        destination_object.set_name(name.clone());
                        destination_object.set_hostname(hostname.clone());
                        destination_object.set_username(username.clone());
                        destinations.borrow_mut().update(&destination_object.uuid(), name, hostname, username);
                        let uuid = destination_object.uuid();
                        if remember && !password.expose_secret().is_empty() {
                            secrets::store_password(&uuid, &password);
                        } else {
                            secrets::delete_password(&uuid);
                        }
                        Some(uuid)
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

        row.imp().delete_button.connect_clicked(glib::clone!(
            #[weak]
            destination_object,
            #[weak(rename_to = page)]
            self,
            move |_| {
                let uuid = destination_object.uuid();
                let model = page.destinations();
                let pos = model
                    .iter::<DestinationObject>()
                    .enumerate()
                    .find_map(|(i, obj)| {
                        obj.ok().filter(|o| o.uuid() == uuid).map(|_| i as u32)
                    });
                if let Some(pos) = pos {
                    let saved_data = Rc::new(destination_object.destination_data());
                    model.remove(pos);

                    let undone = Rc::new(Cell::new(false));
                    let toast = adw::Toast::new("Destination deleted");
                    toast.set_button_label(Some("Undo"));

                    toast.connect_button_clicked(glib::clone!(
                        #[weak]
                        page,
                        #[strong]
                        undone,
                        #[strong]
                        saved_data,
                        move |_| {
                            undone.set(true);
                            page.destinations().insert(
                                pos,
                                &DestinationObject::from_destination_data((*saved_data).clone()),
                            );
                        }
                    ));

                    toast.connect_dismissed(glib::clone!(
                        #[weak]
                        page,
                        #[strong]
                        undone,
                        #[strong]
                        uuid,
                        move |_| {
                            if !undone.get() {
                                secrets::delete_password(&uuid);
                                page.imp()
                                    .destinations_data
                                    .get()
                                    .unwrap()
                                    .borrow_mut()
                                    .remove(&uuid);
                            }
                        }
                    ));

                    page.imp().toast_overlay.add_toast(toast);
                }
            }
        ));

        row
    }

    pub fn show_add_dialog(&self) {
        let dialog = LongLensDestinationDialog::new();
        dialog.set_on_save(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or_default]
            move |name, hostname, username, password, remember| {
                if let Some(uuid) = page.add_destination(name, hostname, username) {
                    if remember && !password.expose_secret().is_empty() {
                        secrets::store_password(&uuid, &password);
                    }
                    Some(uuid)
                } else {
                    None
                }
            }
        ));
        dialog.present(Some(self));
    }
}

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
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use secrecy::ExposeSecret;
use tracing::warn;

use crate::application::LongLensApplication;
use crate::destination_dialog::{DestinationFormData, LongLensDestinationDialog};
use crate::model::destination_object::{DestinationData, DestinationObject};
use crate::destination_row::LlDestinationRow;
use crate::model::destinations::Destinations;
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
        pub destinations: OnceCell<Rc<Destinations>>,
    }
    #[gtk::template_callbacks]
    impl LlDestinationPage {
        #[template_callback]
        fn handle_destinationslist_rowactivated(&self, row: &gtk::ListBoxRow) {
            let destination = self
                .obj()
                .list_model()
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
            let dialog = adw::AlertDialog::new(Some(&gettext("Connection error")), Some(&reason));
            dialog.add_response("close", &gettext("Close"));
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
            let destinations = LongLensApplication::default().destinations();
            let model = destinations.model();
            self.destinations
                .set(destinations)
                .expect("Could not set destinations");
            let page = self.obj();
            self.destinations_list.bind_model(
                Some(&model),
                glib::clone!(
                    #[weak]
                    page,
                    #[upgrade_or_panic]
                    move |obj| {
                        let destination_object = obj
                            .downcast_ref::<DestinationObject>()
                            .expect("The object should be of type `DestinationObject`.");
                        let row = page.create_destination_row(destination_object);
                        row.upcast()
                    }
                ),
            );
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
    /// The backing list model, exposed for the window's stack-page logic and
    /// the `win.connect` lookup.
    pub fn list_model(&self) -> gio::ListStore {
        self.destinations_store().model()
    }

    /// The shared destinations store that owns the model and persistence.
    pub(crate) fn destinations_store(&self) -> Rc<Destinations> {
        self.imp()
            .destinations
            .get()
            .expect("`destinations` should be set in `set_destinations`.")
            .clone()
    }

    /// Add a destination via the store, returning its UUID
    /// (None if a duplicate hostname+username already exists).
    pub fn add_destination(&self, data: DestinationData) -> Option<String> {
        self.destinations_store()
            .add(data)
            .map_err(|error| {
                warn!(?error, "Could not add destination");
                error
            })
            .ok()
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
                let store = page.destinations_store();
                let dialog = LongLensDestinationDialog::new();
                dialog.imp().nameentry.set_text(&destination_object.name());
                dialog.imp().hostnameentry.set_text(&destination_object.hostname());
                dialog.imp().usernameentry.set_text(&destination_object.username());
                dialog.set_connection_options(destination_object.connection_options());
                dialog.set_edit_mode(true);
                dialog.set_on_save(glib::clone!(
                    #[weak]
                    destination_object,
                    #[weak]
                    page,
                    #[strong]
                    store,
                    #[upgrade_or_default]
                    move |form_data, connect_after_save| {
                        let DestinationFormData {
                            name,
                            hostname,
                            username,
                            password,
                            remember_password,
                            options,
                        } = form_data;
                        let uuid = destination_object.uuid();
                        // Updates the shared DestinationObject in place, so the
                        // bound row title/subtitle refresh automatically.
                        let mut data = DestinationData::new(name, hostname, username, options);
                        data.uuid = uuid.clone();
                        if let Err(error) = store.update(data) {
                            warn!(?error, "Could not update destination");
                            return;
                        }
                        glib::spawn_future_local(glib::clone!(
                            #[strong]
                            uuid,
                            async move {
                                if remember_password && !password.expose_secret().is_empty() {
                                    secrets::store_password(&uuid, &password).await;
                                } else {
                                    secrets::delete_password(&uuid).await;
                                }
                            }
                        ));
                        if connect_after_save {
                            page.activate_action("win.connect", Some(&uuid.to_variant()))
                                .expect("win.connect action failed");
                        }
                    }
                ));
                dialog.set_on_delete(glib::clone!(
                    #[weak]
                    destination_object,
                    #[strong]
                    store,
                    move || {
                        let uuid = destination_object.uuid();
                        if let Err(error) = store.remove(&uuid) {
                            warn!(?error, "Could not remove destination");
                        }
                        glib::spawn_future_local(async move {
                            secrets::delete_password(&uuid).await;
                        });
                    }
                ));
                // Pre-fill stored password if available.
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    dialog,
                    #[weak]
                    destination_object,
                    async move {
                        if let Some(pw) = secrets::get_password(&destination_object.uuid()).await {
                            dialog.imp().passwordentry.set_text(pw.expose_secret());
                        } else {
                            dialog.imp().rememberpasswordswitch.set_active(false);
                        }
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
                let store = page.destinations_store();
                let Some((pos, _)) = store.find(&uuid) else {
                    return;
                };
                let saved_data = Rc::new(destination_object.destination_data());
                if let Err(error) = store.remove(&uuid) {
                    warn!(?error, "Could not remove destination");
                }

                let undone = Rc::new(Cell::new(false));
                let toast = adw::Toast::new(&gettext("Destination deleted"));
                toast.set_button_label(Some(&gettext("Undo")));

                toast.connect_button_clicked(glib::clone!(
                    #[weak]
                    page,
                    #[strong]
                    undone,
                    #[strong]
                    saved_data,
                    move |_| {
                        undone.set(true);
                        if let Err(error) = page.destinations_store().restore(pos, (*saved_data).clone()) {
                            warn!(?error, "Could not restore destination");
                        }
                    }
                ));

                toast.connect_dismissed(glib::clone!(
                    #[strong]
                    undone,
                    #[strong]
                    uuid,
                    move |_| {
                        // The destination is already gone from the store; only the
                        // stored password remains to be cleaned up if not undone.
                        if !undone.get() {
                            let uuid = uuid.clone();
                            glib::spawn_future_local(async move {
                                secrets::delete_password(&uuid).await;
                            });
                        }
                    }
                ));

                page.imp().toast_overlay.add_toast(toast);
            }
        ));

        row
    }

    pub fn show_add_dialog(&self) {
        self.show_add_dialog_with(String::new(), String::new(), String::new());
    }

    /// Show the Add Destination dialog pre-filled with the given values, e.g.
    /// from a parsed `.rdp` file. Empty strings leave the corresponding fields blank.
    pub fn show_add_dialog_with(&self, name: String, hostname: String, username: String) {
        let dialog = LongLensDestinationDialog::new();
        dialog.imp().nameentry.set_text(&name);
        dialog.imp().hostnameentry.set_text(&hostname);
        dialog.imp().usernameentry.set_text(&username);
        dialog.set_on_save(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or_default]
            move |form_data, connect_after_save| {
                let DestinationFormData {
                    name,
                    hostname,
                    username,
                    password,
                    remember_password,
                    options,
                } = form_data;
                let Some(uuid) = page.add_destination(DestinationData::new(name, hostname, username, options)) else {
                    return;
                };
                if remember_password && !password.expose_secret().is_empty() {
                    glib::spawn_future_local(glib::clone!(
                        #[strong]
                        uuid,
                        async move {
                            secrets::store_password(&uuid, &password).await;
                        }
                    ));
                }
                if connect_after_save {
                    page.activate_action("win.connect", Some(&uuid.to_variant()))
                        .expect("win.connect action failed");
                }
            }
        ));
        dialog.present(Some(self));
    }
}

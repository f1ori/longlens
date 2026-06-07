/* application.rs
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
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};


use crate::config::APP_ID;
use crate::model::destinations::Destinations;
use crate::LongLensWindow;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct LongLensApplication {
        pub destinations: OnceCell<Rc<Destinations>>,
        pub settings: OnceCell<gio::Settings>,
        pub pending_connection: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LongLensApplication {
        const NAME: &'static str = "LongLensApplication";
        type Type = super::LongLensApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for LongLensApplication {
        fn constructed(&self) {
            self.parent_constructed();
            self.destinations
                .set(Rc::new(Destinations::load()))
                .expect("Could not set destinations");
            self.settings
                .set(gio::Settings::new(APP_ID))
                .expect("Could not create settings");
            let obj = self.obj();
            obj.setup_gactions();
        }
    }

    impl ApplicationImpl for LongLensApplication {
        fn startup(&self) {
            self.parent_startup();
            let scheme = self.settings.get().unwrap().string("color-scheme");
            super::LongLensApplication::apply_color_scheme(&scheme);

            if let Some(conn) = self.obj().dbus_connection() {
                crate::search_provider::register_search_provider(&conn, &self.obj());
            }
        }

        // We connect to the activate callback to create a window when the application
        // has been launched. Additionally, this callback notifies us when the user
        // tries to launch a "second instance" of the application. When they try
        // to do that, we'll just present any existing window.
        fn activate(&self) {
            let application = self.obj();

            if let Some(uuid) = self.pending_connection.borrow_mut().take() {
                let window = LongLensWindow::new(&*application);
                window.present();
                let window_weak = window.downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(w) = window_weak.upgrade() {
                        let _ = gtk::prelude::WidgetExt::activate_action(&w, "win.connect", Some(&uuid.to_variant()));
                    }
                });
            } else {
                let window = LongLensWindow::new(&*application);
                window.present();
            }
        }
    }

    impl GtkApplicationImpl for LongLensApplication {}
    impl AdwApplicationImpl for LongLensApplication {}
}

glib::wrapper! {
    pub struct LongLensApplication(ObjectSubclass<imp::LongLensApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl LongLensApplication {
    pub fn destinations(&self) -> Rc<Destinations> {
        self.imp().destinations.get().unwrap().clone()
    }

    pub fn new(application_id: &str, flags: &gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .property("resource-base-path", "/de/f1ori/longlens")
            .build()
    }

    fn settings(&self) -> &gio::Settings {
        self.imp().settings.get().unwrap()
    }

    fn apply_color_scheme(scheme: &str) {
        let color_scheme = match scheme {
            "light" => adw::ColorScheme::ForceLight,
            "dark" => adw::ColorScheme::ForceDark,
            _ => adw::ColorScheme::Default,
        };
        adw::StyleManager::default().set_color_scheme(color_scheme);
    }

    fn setup_gactions(&self) {
        let quit_action = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| app.quit())
            .build();
        let about_action = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| app.show_about())
            .build();
        let new_window_action = gio::ActionEntry::builder("new-window")
            .activate(move |app: &Self, _, _| app.activate())
            .build();

        let initial_scheme = self.settings().string("color-scheme");

        let color_scheme_action = gio::SimpleAction::new_stateful(
            "color-scheme",
            Some(glib::VariantTy::STRING),
            &initial_scheme.to_variant(),
        );
        color_scheme_action.connect_activate(glib::clone!(
            #[weak(rename_to = app)]
            self,
            move |action, parameter| {
                let scheme = parameter
                    .and_then(|v| v.get::<String>())
                    .unwrap_or_else(|| "default".to_string());
                action.set_state(&scheme.to_variant());
                app.settings().set_string("color-scheme", &scheme).ok();
                Self::apply_color_scheme(&scheme);
            }
        ));
        self.add_action(&color_scheme_action);

        self.add_action_entries([quit_action, about_action, new_window_action]);
    }

    fn show_about(&self) {
        let window = self.active_window().unwrap();
        crate::about_dialog::show(&window);
    }
}

impl Default for LongLensApplication {
    fn default() -> Self {
        gio::Application::default()
            .expect("Could not get default GApplication")
            .downcast()
            .unwrap()
    }
}

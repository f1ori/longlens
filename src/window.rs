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

use adw::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};

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
    }

    #[gtk::template_callbacks]
    impl FernsichtRdpWindow {
        #[template_callback]
        fn handle_connectbutton_activated(&self, _button: &adw::ButtonRow) {
            self.stack.set_visible_child_name("connecting");
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

    impl ObjectImpl for FernsichtRdpWindow {}
    impl WidgetImpl for FernsichtRdpWindow {}
    impl WindowImpl for FernsichtRdpWindow {}
    impl ApplicationWindowImpl for FernsichtRdpWindow {}
    impl AdwApplicationWindowImpl for FernsichtRdpWindow {}
}

glib::wrapper! {
    pub struct FernsichtRdpWindow(ObjectSubclass<imp::FernsichtRdpWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,        @implements gio::ActionGroup, gio::ActionMap;
}

impl FernsichtRdpWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }
}

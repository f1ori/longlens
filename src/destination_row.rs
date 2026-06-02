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

use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/de/f1ori/longlens/ui/destination_row.ui")]
    pub struct LlDestinationRow {
        #[template_child]
        pub edit_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub delete_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LlDestinationRow {
        const NAME: &'static str = "LlDestinationRow";
        type Type = super::LlDestinationRow;
        type ParentType = adw::ActionRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LlDestinationRow {}
    impl WidgetImpl for LlDestinationRow {}
    impl ListBoxRowImpl for LlDestinationRow {}
    impl PreferencesRowImpl for LlDestinationRow {}
    impl ActionRowImpl for LlDestinationRow {}
}

glib::wrapper! {
    pub struct LlDestinationRow(ObjectSubclass<imp::LlDestinationRow>)
        @extends adw::ActionRow, adw::PreferencesRow, gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl LlDestinationRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

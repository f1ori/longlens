/* 
 * Copyright 2026 Florian Richter
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
use std::cell::RefCell;

use glib::Object;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;


#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionOptions {
    #[serde(default = "default_clipboard_enabled")]
    pub clipboard_enabled: bool,
    #[serde(default = "default_sound_enabled")]
    pub sound_enabled: bool,
    #[serde(default)]
    pub forward_unicode: bool,
    #[serde(default = "default_inhibit_system_shortcuts")]
    pub inhibit_system_shortcuts: bool,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            clipboard_enabled: true,
            sound_enabled: true,
            forward_unicode: false,
            inhibit_system_shortcuts: true,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DestinationData {
    #[serde(default = "new_uuid")]
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    pub hostname: String,
    pub username: String,
    #[serde(default = "default_clipboard_enabled")]
    pub clipboard_enabled: bool,
    #[serde(default = "default_sound_enabled")]
    pub sound_enabled: bool,
    #[serde(default)]
    pub forward_unicode: bool,
    #[serde(default = "default_inhibit_system_shortcuts")]
    pub inhibit_system_shortcuts: bool,
}

fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

fn default_clipboard_enabled() -> bool {
    ConnectionOptions::default().clipboard_enabled
}

fn default_sound_enabled() -> bool {
    ConnectionOptions::default().sound_enabled
}

fn default_inhibit_system_shortcuts() -> bool {
    ConnectionOptions::default().inhibit_system_shortcuts
}

impl DestinationData {
    pub fn new(
        name: String,
        hostname: String,
        username: String,
        options: ConnectionOptions,
    ) -> Self {
        Self {
            uuid: new_uuid(),
            name,
            hostname,
            username,
            clipboard_enabled: options.clipboard_enabled,
            sound_enabled: options.sound_enabled,
            forward_unicode: options.forward_unicode,
            inhibit_system_shortcuts: options.inhibit_system_shortcuts,
        }
    }

    pub fn connection_options(&self) -> ConnectionOptions {
        ConnectionOptions {
            clipboard_enabled: self.clipboard_enabled,
            sound_enabled: self.sound_enabled,
            forward_unicode: self.forward_unicode,
            inhibit_system_shortcuts: self.inhibit_system_shortcuts,
        }
    }

    pub fn display_title(&self) -> String {
        if self.name.is_empty() {
            self.hostname.clone()
        } else {
            self.name.clone()
        }
    }
}

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::DestinationObject)]
    pub struct DestinationObject {
        #[property(name = "uuid", get, set, type = String, member = uuid)]
        #[property(name = "name", get, set, type = String, member = name)]
        #[property(name = "hostname", get, set, type = String, member = hostname)]
        #[property(name = "username", get, set, type = String, member = username)]
        #[property(name = "clipboard-enabled", get, set, type = bool, member = clipboard_enabled)]
        #[property(name = "sound-enabled", get, set, type = bool, member = sound_enabled)]
        #[property(name = "forward-unicode", get, set, type = bool, member = forward_unicode)]
        #[property(name = "inhibit-system-shortcuts", get, set, type = bool, member = inhibit_system_shortcuts)]
        pub data: RefCell<DestinationData>,
        #[property(name = "display-title", get = Self::compute_display_title, type = String)]
        _display_title: (),
        #[property(name = "display-subtitle", get = Self::compute_display_subtitle, type = String)]
        _display_subtitle: (),
    }

    impl DestinationObject {
        fn compute_display_title(&self) -> String {
            let name = self.obj().name();
            if name.is_empty() { self.obj().hostname() } else { name }
        }

        fn compute_display_subtitle(&self) -> String {
            let obj = self.obj();
            let username = obj.username();
            if obj.name().is_empty() {
                format!("User: {}", username)
            } else {
                format!("Host: {}, User: {}", obj.hostname(), username)
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DestinationObject {
        const NAME: &'static str = "LongLensDestinationObject";
        type Type = super::DestinationObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DestinationObject {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.connect_notify_local(Some("name"), |this, _| {
                this.notify("display-title");
                this.notify("display-subtitle");
            });
            obj.connect_notify_local(Some("hostname"), |this, _| {
                this.notify("display-title");
                this.notify("display-subtitle");
            });
            obj.connect_notify_local(Some("username"), |this, _| {
                this.notify("display-subtitle");
            });
        }
    }
}

glib::wrapper! {
    pub struct DestinationObject(ObjectSubclass<imp::DestinationObject>);
}

impl DestinationObject {
    pub fn destination_data(&self) -> DestinationData {
        self.imp().data.borrow().clone()
    }

    pub fn from_destination_data(data: DestinationData) -> Self {
        Object::builder()
            .property("uuid", &data.uuid)
            .property("name", &data.name)
            .property("hostname", &data.hostname)
            .property("username", &data.username)
            .property("clipboard-enabled", data.clipboard_enabled)
            .property("sound-enabled", data.sound_enabled)
            .property("forward-unicode", data.forward_unicode)
            .property("inhibit-system-shortcuts", data.inhibit_system_shortcuts)
            .build()
    }

    pub fn connection_options(&self) -> ConnectionOptions {
        self.imp().data.borrow().connection_options()
    }

    pub fn set_connection_options(&self, options: ConnectionOptions) {
        self.set_clipboard_enabled(options.clipboard_enabled);
        self.set_sound_enabled(options.sound_enabled);
        self.set_forward_unicode(options.forward_unicode);
        self.set_inhibit_system_shortcuts(options.inhibit_system_shortcuts);
    }
}

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

use std::fs::File;

use gtk::gio;
use gtk::prelude::*;

use super::destination_object::{DestinationData, DestinationObject};
use crate::utils::data_path;

/// Owns the `gio::ListStore` of `DestinationObject`s as the single source of
/// truth. All mutations go through this type and persist to disk immediately.
#[derive(Debug)]
pub struct Destinations {
    model: gio::ListStore,
}

impl Destinations {
    pub fn load() -> Self {
        let items: Vec<DestinationData> = File::open(data_path())
            .ok()
            .and_then(|file| serde_json::from_reader(file).ok())
            .unwrap_or_default();
        let model = gio::ListStore::new::<DestinationObject>();
        let objects: Vec<DestinationObject> = items
            .into_iter()
            .map(DestinationObject::from_destination_data)
            .collect();
        model.extend_from_slice(&objects);
        Self { model }
    }

    /// The backing list model, for binding to a view.
    pub fn model(&self) -> gio::ListStore {
        self.model.clone()
    }

    /// A snapshot of all destinations as plain data (e.g. for the search provider).
    pub fn items(&self) -> Vec<DestinationData> {
        self.model
            .iter::<DestinationObject>()
            .filter_map(|obj| obj.ok())
            .map(|obj| obj.destination_data())
            .collect()
    }

    fn save(&self) {
        let file = File::create(data_path()).expect("Could not create json file.");
        serde_json::to_writer(file, &self.items()).expect("Could not write data to json file");
    }

    /// Locate a destination by UUID, returning its position and object.
    pub fn find(&self, uuid: &str) -> Option<(u32, DestinationObject)> {
        self.model
            .iter::<DestinationObject>()
            .enumerate()
            .find_map(|(i, obj)| obj.ok().filter(|o| o.uuid() == uuid).map(|o| (i as u32, o)))
    }

    pub fn add(
        &self,
        name: String,
        hostname: String,
        username: String,
        clipboard_enabled: bool,
        sound_enabled: bool,
        inhibit_system_shortcuts: bool,
    ) -> Option<String> {
        let already_exists = self
            .model
            .iter::<DestinationObject>()
            .filter_map(|obj| obj.ok())
            .any(|d| d.hostname() == hostname && d.username() == username);
        if already_exists {
            return None;
        }
        let object = DestinationObject::new(name, hostname, username, clipboard_enabled, sound_enabled, inhibit_system_shortcuts);
        let uuid = object.uuid();
        self.model.append(&object);
        self.save();
        Some(uuid)
    }

    pub fn update(
        &self,
        uuid: &str,
        name: String,
        hostname: String,
        username: String,
        clipboard_enabled: bool,
        sound_enabled: bool,
        inhibit_system_shortcuts: bool,
    ) {
        if let Some((_, dest)) = self.find(uuid) {
            dest.set_name(name);
            dest.set_hostname(hostname);
            dest.set_username(username);
            dest.set_clipboard_enabled(clipboard_enabled);
            dest.set_sound_enabled(sound_enabled);
            dest.set_inhibit_system_shortcuts(inhibit_system_shortcuts);
            self.save();
        }
    }

    pub fn remove(&self, uuid: &str) {
        if let Some((pos, _)) = self.find(uuid) {
            self.model.remove(pos);
            self.save();
        }
    }

    /// Re-insert a previously removed destination at `pos`, used to undo a delete.
    pub fn restore(&self, pos: u32, data: DestinationData) {
        self.model
            .insert(pos, &DestinationObject::from_destination_data(data));
        self.save();
    }

    pub fn search(&self, terms: &[String]) -> Vec<String> {
        self.items()
            .iter()
            .filter(|dest| {
                let haystack = format!("{} {} {}", dest.name, dest.hostname, dest.username)
                    .to_lowercase();
                terms.iter().all(|term| haystack.contains(&term.to_lowercase()))
            })
            .map(|dest| dest.uuid.clone())
            .collect()
    }
}

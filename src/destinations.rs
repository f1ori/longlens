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

use crate::destination_object::DestinationData;
use crate::utils::data_path;

#[derive(Debug)]
pub struct Destinations {
    items: Vec<DestinationData>,
}

impl Destinations {
    pub fn load() -> Self {
        let items = File::open(data_path())
            .ok()
            .and_then(|file| serde_json::from_reader(file).ok())
            .unwrap_or_default();
        Self { items }
    }

    pub fn save(&self) {
        let file = File::create(data_path()).expect("Could not create json file.");
        serde_json::to_writer(file, &self.items).expect("Could not write data to json file");
    }

    pub fn items(&self) -> &[DestinationData] {
        &self.items
    }

    pub fn add(&mut self, name: String, hostname: String, username: String) -> Option<String> {
        let already_exists = self
            .items
            .iter()
            .any(|d| d.hostname == hostname && d.username == username);
        if !already_exists {
            let uuid = uuid::Uuid::new_v4().to_string();
            self.items.push(DestinationData {
                uuid: uuid.clone(),
                name,
                hostname,
                username,
            });
            self.save();
            Some(uuid)
        } else {
            None
        }
    }

    pub fn update(&mut self, uuid: &str, name: String, hostname: String, username: String) {
        if let Some(dest) = self.items.iter_mut().find(|d| d.uuid == uuid) {
            dest.name = name;
            dest.hostname = hostname;
            dest.username = username;
            self.save();
        }
    }

    pub fn remove(&mut self, uuid: &str) {
        self.items.retain(|d| d.uuid != uuid);
        self.save();
    }
}

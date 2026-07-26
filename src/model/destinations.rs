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

use std::fs::{File, OpenOptions};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use gtk::gio;
use gtk::prelude::*;
use tracing::warn;

use super::destination_object::{DestinationData, DestinationObject};
use crate::utils::data_path;

/// Owns the `gio::ListStore` of `DestinationObject`s as the single source of
/// truth. All mutations go through this type and persist to disk immediately.
#[derive(Debug)]
pub struct Destinations {
    model: gio::ListStore,
}

#[derive(Debug)]
pub enum DestinationError {
    Duplicate,
    NotFound,
    Io(io::Error),
    Serde(serde_json::Error),
}

impl From<io::Error> for DestinationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for DestinationError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serde(error)
    }
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

    fn save(&self) -> Result<(), DestinationError> {
        let path = data_path();
        let tmp_path = path.with_extension("json.tmp");
        {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            options.mode(0o600);
            let file = options.open(&tmp_path)?;
            serde_json::to_writer_pretty(file, &self.items())?;
        }
        std::fs::rename(tmp_path, path)?;
        Ok(())
    }

    /// Locate a destination by UUID, returning its position and object.
    pub fn find(&self, uuid: &str) -> Option<(u32, DestinationObject)> {
        self.model
            .iter::<DestinationObject>()
            .enumerate()
            .find_map(|(i, obj)| obj.ok().filter(|o| o.uuid() == uuid).map(|o| (i as u32, o)))
    }

    pub fn get(&self, uuid: &str) -> Option<DestinationData> {
        self.find(uuid).map(|(_, obj)| obj.destination_data())
    }

    pub fn add(&self, data: DestinationData) -> Result<String, DestinationError> {
        let already_exists = self
            .model
            .iter::<DestinationObject>()
            .filter_map(|obj| obj.ok())
            .any(|d| d.hostname() == data.hostname && d.username() == data.username);
        if already_exists {
            return Err(DestinationError::Duplicate);
        }
        let object = DestinationObject::from_destination_data(data);
        let uuid = object.uuid();
        self.model.append(&object);
        if let Err(error) = self.save() {
            warn!(?error, "Could not save destination after add");
            return Err(error);
        }
        Ok(uuid)
    }

    pub fn update(&self, data: DestinationData) -> Result<(), DestinationError> {
        let Some((_, dest)) = self.find(&data.uuid) else {
            return Err(DestinationError::NotFound);
        };
        let options = data.connection_options();
        dest.set_name(data.name);
        dest.set_hostname(data.hostname);
        dest.set_username(data.username);
        dest.set_connection_options(options);
        self.save()
    }

    pub fn remove(&self, uuid: &str) -> Result<(), DestinationError> {
        let Some((pos, _)) = self.find(uuid) else {
            return Err(DestinationError::NotFound);
        };
        self.model.remove(pos);
        self.save()
    }

    /// Re-insert a previously removed destination at `pos`, used to undo a delete.
    pub fn restore(&self, pos: u32, data: DestinationData) -> Result<(), DestinationError> {
        self.model
            .insert(pos, &DestinationObject::from_destination_data(data));
        self.save()
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

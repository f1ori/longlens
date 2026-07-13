/* destination_service.rs
 *
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

//! Coordinates destination persistence with related side effects such as
//! password storage.
//!
//! UI widgets should own presentation concerns (dialogs, rows, toasts), while
//! this type owns the application rules for adding, updating, removing and
//! restoring destinations.

use std::rc::Rc;

use gtk::glib;
use secrecy::{ExposeSecret, SecretString};

use crate::destination_dialog::DestinationFormData;
use crate::model::destination_object::DestinationData;
use crate::model::destinations::{DestinationError, Destinations};
use crate::secrets;

#[derive(Debug, Clone)]
pub struct RemovedDestination {
    pub pos: u32,
    pub data: DestinationData,
    pub uuid: String,
}

#[derive(Debug, Clone)]
pub struct DestinationService {
    destinations: Rc<Destinations>,
}

impl DestinationService {
    pub fn new(destinations: Rc<Destinations>) -> Self {
        Self { destinations }
    }

    pub fn add_from_form(&self, form_data: DestinationFormData) -> Result<String, DestinationError> {
        let DestinationFormData {
            name,
            hostname,
            username,
            password,
            remember_password,
            options,
        } = form_data;

        let uuid = self
            .destinations
            .add(DestinationData::new(name, hostname, username, options))?;
        Self::save_password_choice(uuid.clone(), remember_password, password);
        Ok(uuid)
    }

    pub fn update_from_form(
        &self,
        uuid: &str,
        form_data: DestinationFormData,
    ) -> Result<String, DestinationError> {
        let DestinationFormData {
            name,
            hostname,
            username,
            password,
            remember_password,
            options,
        } = form_data;

        let mut data = DestinationData::new(name, hostname, username, options);
        data.uuid = uuid.to_string();
        self.destinations.update(data)?;
        Self::save_password_choice(uuid.to_string(), remember_password, password);
        Ok(uuid.to_string())
    }

    pub fn remove(&self, uuid: &str) -> Result<RemovedDestination, DestinationError> {
        let Some((pos, object)) = self.destinations.find(uuid) else {
            return Err(DestinationError::NotFound);
        };
        let removed = RemovedDestination {
            pos,
            data: object.destination_data(),
            uuid: uuid.to_string(),
        };
        self.destinations.remove(uuid)?;
        Ok(removed)
    }

    pub fn restore(&self, removed: RemovedDestination) -> Result<(), DestinationError> {
        self.destinations.restore(removed.pos, removed.data)
    }

    pub fn finish_delete(&self, uuid: String) {
        glib::spawn_future_local(async move {
            secrets::delete_password(&uuid).await;
        });
    }

    pub async fn stored_password(uuid: &str) -> Option<SecretString> {
        secrets::get_password(uuid).await
    }

    fn save_password_choice(uuid: String, remember_password: bool, password: SecretString) {
        glib::spawn_future_local(async move {
            if remember_password && !password.expose_secret().is_empty() {
                secrets::store_password(&uuid, &password).await;
            } else {
                secrets::delete_password(&uuid).await;
            }
        });
    }
}

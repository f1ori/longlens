/* connection_controller.rs
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

//! Coordinates the saved-destination connection flow.
//!
//! The window owns presentation/layout. This controller owns the application
//! logic for resolving a destination, retrieving credentials and starting the
//! RDP widget.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use secrecy::SecretString;
use tracing::warn;

use crate::model::destination_object::{ConnectionOptions, DestinationData};
use crate::model::destinations::Destinations;
use crate::password_dialog::LongLensPasswordDialog;
use crate::rdp::RdpWidget;
use crate::window::LongLensWindow;

#[derive(Debug)]
pub struct ConnectionController {
    destinations: Rc<Destinations>,
    rdpwidget: RdpWidget,
    current_destination_uuid: RefCell<Option<String>>,
    current_display_title: RefCell<Option<String>>,
}

impl ConnectionController {
    pub fn new(destinations: Rc<Destinations>, rdpwidget: &RdpWidget) -> Rc<Self> {
        Rc::new(Self {
            destinations,
            rdpwidget: rdpwidget.clone(),
            current_destination_uuid: RefCell::new(None),
            current_display_title: RefCell::new(None),
        })
    }

    pub fn connect_by_uuid(
        self: &Rc<Self>,
        window: &LongLensWindow,
        uuid: String,
        width: u16,
        height: u16,
    ) {
        let Some(dest) = self.destinations.get(&uuid) else {
            return;
        };

        let display_title = dest.display_title();
        let options = dest.connection_options();
        let hostname = dest.hostname;
        let username = dest.username;
        let controller = self.clone();
        let window_weak = window.downgrade();

        glib::spawn_future_local(async move {
            match crate::secrets::get_password(&uuid).await {
                Some(password) => {
                    controller.start_connection(
                        uuid,
                        hostname,
                        username,
                        password,
                        display_title,
                        options,
                        width,
                        height,
                    );
                }
                None => {
                    let Some(window) = window_weak.upgrade() else {
                        return;
                    };
                    let dialog = LongLensPasswordDialog::new();
                    dialog.set_hostname(&hostname);
                    dialog.set_username(&username);

                    let controller = controller.clone();
                    dialog.set_on_connect(move |password| {
                        controller.start_connection(
                            uuid.clone(),
                            hostname.clone(),
                            username.clone(),
                            password,
                            display_title.clone(),
                            options,
                            width,
                            height,
                        );
                    });
                    dialog.present(Some(&window));
                }
            }
        });
    }

    pub fn disconnect(&self) {
        self.rdpwidget.disconnect();
    }

    pub fn clear_current(&self) {
        self.current_destination_uuid.borrow_mut().take();
        self.current_display_title.borrow_mut().take();
    }

    pub fn current_display_title(&self) -> Option<String> {
        self.current_display_title.borrow().clone()
    }

    pub fn current_destination(&self) -> Option<DestinationData> {
        let uuid = self.current_destination_uuid.borrow().clone()?;
        self.destinations.get(&uuid)
    }

    pub fn apply_runtime_connection_options(&self, options: ConnectionOptions) {
        let Some(dest) = self.current_destination() else {
            return;
        };

        self.rdpwidget.set_connection_options(options);

        let mut data = DestinationData::new(dest.name, dest.hostname, dest.username, options);
        data.uuid = dest.uuid;
        if let Err(error) = self.destinations.update(data) {
            warn!(?error, "Could not update connection options");
        }
    }

    fn start_connection(
        &self,
        uuid: String,
        hostname: String,
        username: String,
        password: SecretString,
        display_title: String,
        options: ConnectionOptions,
        width: u16,
        height: u16,
    ) {
        *self.current_destination_uuid.borrow_mut() = Some(uuid);
        *self.current_display_title.borrow_mut() = Some(display_title);

        let (server, port) = crate::rdp::parse_hostname_port(&hostname);
        self.rdpwidget
            .connect_to_server(server, port, username, password, width, height, options);
    }
}

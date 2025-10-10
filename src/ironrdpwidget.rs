/* ironrdpwidget.rs
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

use crate::rdpclient::{RdpInputEvent, RdpOutputEvent, start_rdp};
use gtk::glib;
use gtk::subclass::prelude::*;
use std::cell::OnceCell;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("Setting up tokio runtime needs to succeed."))
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct IronRdpWidget {
        input_sender: OnceCell<async_channel::Sender<RdpInputEvent>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IronRdpWidget {
        const NAME: &'static str = "IronRdpWidget";
        type Type = super::IronRdpWidget;
        type ParentType = gtk::Widget;
    }

    impl IronRdpWidget {
        pub fn connect_to_server(&self, hostname: String, username: String, password: String) {
            let input_sender = self.input_sender.clone();
            glib::spawn_future_local(glib::clone!(
                #[strong]
                input_sender,
                async move {
                    input_sender
                        .get()
                        .unwrap()
                        .send(RdpInputEvent::Connect {
                            hostname,
                            username,
                            password,
                            width: 768,
                            height: 568,
                        })
                        .await
                        .expect("The channel needs to be open.");
                }
            ));
        }
    }
    impl ObjectImpl for IronRdpWidget {
        fn constructed(&self) {
            self.parent_constructed();

            let (input_sender, input_receiver) = async_channel::bounded::<RdpInputEvent>(10);
            let (output_sender, output_receiver) = async_channel::bounded::<RdpOutputEvent>(10);
            self.input_sender.set(input_sender).unwrap();
            runtime().spawn(glib::clone!(
                #[strong]
                input_receiver,
                #[strong]
                output_sender,
                async move {
                    start_rdp(input_receiver, output_sender).await;
                }
            ));

            glib::spawn_future_local(async move {
                while let Ok(output_event) = output_receiver.recv().await {
                    match output_event {
                        RdpOutputEvent::Connected => {
                            println!("Connected!");
                        }
                        RdpOutputEvent::ConnectionFailure(_error_message) => {
                            println!("Connection failed");
                        }
                    };
                }
            });
        }
    }

    impl WidgetImpl for IronRdpWidget {}
}

glib::wrapper! {
    pub struct IronRdpWidget(ObjectSubclass<imp::IronRdpWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl IronRdpWidget {
    pub fn connect_to_server(&self, hostname: String, username: String, password: String) {
        self.imp().connect_to_server(hostname, username, password);
    }
}


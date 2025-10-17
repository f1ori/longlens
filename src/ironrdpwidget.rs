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
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::OnceCell;


mod imp {
    use super::*;

    #[derive(Default)]
    pub struct IronRdpWidget {
        input_sender: OnceCell<async_channel::Sender<RdpInputEvent>>,
        texture: std::cell::RefCell<Option<gdk::MemoryTexture>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IronRdpWidget {
        const NAME: &'static str = "IronRdpWidget";
        type Type = super::IronRdpWidget;
        type ParentType = gtk::Widget;
    }

    impl IronRdpWidget {
        pub fn connect_to_server(
            &self,
            hostname: String,
            port: u16,
            username: String,
            password: String,
        ) {
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
                            port,
                            password,
                            width: 768,
                            height: 568,
                        })
                        .await
                        .expect("The channel needs to be open.");
                }
            ));
        }

        fn process_message(&self, output_event: RdpOutputEvent) -> glib::ControlFlow {
            match output_event {
                RdpOutputEvent::Connected => {
                    println!("Connected!");
                }
                RdpOutputEvent::ConnectionFailure(_error_message) => {
                    println!("Connection failed");
                }
                RdpOutputEvent::Image {
                    buffer,
                    width,
                    height,
                } => {
                    let bytes = glib::Bytes::from_owned(buffer);
                    *self.texture.borrow_mut() = Some(gdk::MemoryTexture::new(
                        width.get().into(),
                        height.get().into(),
                        gdk::MemoryFormat::R8g8b8x8,
                        &bytes,
                        (width.get() * 4).into(),
                    ));
                    self.obj().queue_draw();
                }
            }

            glib::ControlFlow::Continue
        }
    }
    impl ObjectImpl for IronRdpWidget {
        fn constructed(&self) {
            self.parent_constructed();

            let (input_sender, input_receiver) = async_channel::bounded::<RdpInputEvent>(10);
            let (output_sender, output_receiver) = async_channel::bounded::<RdpOutputEvent>(10);
            self.input_sender.set(input_sender).unwrap();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();

                rt.block_on(async {
                    start_rdp(input_receiver, output_sender).await;
                });
            });

            glib::spawn_future_local(glib::clone!(
                #[strong]
                output_receiver,
                #[weak(rename_to = imp)]
                self,
                async move {
                    while let Ok(output_event) = output_receiver.recv().await {
                        imp.process_message(output_event);
                    }
                }
            ));
        }
    }

    impl WidgetImpl for IronRdpWidget {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(texture) = self.texture.borrow().as_ref() {
                // Render texture at 0,0
                snapshot.append_texture(
                    texture,
                    &gtk::graphene::Rect::new(
                        0.0,
                        0.0,
                        texture.width() as f32,
                        texture.height() as f32,
                    ),
                );
            } else {
                // Draw a fallback background
                snapshot.append_color(
                    &gdk::RGBA::BLACK,
                    &gtk::graphene::Rect::new(0.0, 0.0, 100.0, 100.0),
                );
            }

            self.parent_snapshot(snapshot)
        }
    }
}

glib::wrapper! {
    pub struct IronRdpWidget(ObjectSubclass<imp::IronRdpWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl IronRdpWidget {
    pub fn connect_to_server(
        &self,
        hostname: String,
        port: u16,
        username: String,
        password: String,
    ) {
        self.imp()
            .connect_to_server(hostname, port, username, password);
    }
}


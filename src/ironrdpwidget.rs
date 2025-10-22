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
use gtk::glib::prelude::*;
use gtk::glib::{self, Properties};
use gtk::glib::subclass::Signal;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, OnceCell, RefCell};
use std::sync::OnceLock;
use tracing::{debug, info};


#[derive(Debug, Clone, Copy, PartialEq, Eq, glib::Enum, Default)]
#[enum_type(name = "RdpState")]
pub enum RdpState {
    #[default]
    Disconnected = 0,
    Connecting = 1,
    Connected = 2,
}

mod imp {
    use super::*;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::IronRdpWidget)]
    pub struct IronRdpWidget {
        #[property(get, set, builder(RdpState::Disconnected))]
        state: Cell<RdpState>,
        input_sender: OnceCell<async_channel::Sender<RdpInputEvent>>,
        texture: RefCell<Option<gdk::MemoryTexture>>,
        input_database: OnceCell<RefCell<ironrdp::input::Database>>,
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
            width: u16,
            height: u16
        ) {
            info!("widthxheight {} {}", width, height);
            self.obj().set_state(RdpState::Connecting);
            glib::spawn_future_local(glib::clone!(
                #[strong(rename_to=sender)]
                self.input_sender.clone(),
                async move {
                    sender
                        .get()
                        .unwrap()
                        .send(RdpInputEvent::Connect {
                            hostname,
                            username,
                            port,
                            password,
                            width,
                            height,
                        })
                        .await
                        .expect("The channel needs to be open.");
                }
            ));
        }

        pub fn disconnect(&self) {
            glib::spawn_future_local(glib::clone!(
                #[strong(rename_to=sender)]
                self.input_sender.clone(),
                async move {
                    sender
                        .get()
                        .unwrap()
                        .send(RdpInputEvent::Close {})
                        .await
                        .expect("The channel needs to be open.");
                }
            ));
        }

        fn process_message(&self, output_event: RdpOutputEvent) -> glib::ControlFlow {
            match output_event {
                RdpOutputEvent::Connected => {
                    info!("State connected");
                    self.obj().set_state(RdpState::Connected);
                }
                RdpOutputEvent::Terminated(Ok(reason)) => {
                    info!("State disconnected {}", reason);
                    self.obj().set_state(RdpState::Disconnected);
                }
                RdpOutputEvent::Terminated(Err(reason)) => {
                    self.obj().set_state(RdpState::Disconnected);
                    println!("Error {}", reason);
                }
                RdpOutputEvent::ConnectionFailure(reason) => {
                    self.obj().set_state(RdpState::Disconnected);
                    let error_string = reason.to_string();
                    self.obj().emit_by_name::<()>("connection-failed", &[&error_string]);
                    println!("Connection error {}", reason);
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
                RdpOutputEvent::PointerDefault => {
                    let cursor = gdk::Cursor::from_name("default", None);
                    self.obj().set_cursor(cursor.as_ref());
                }
                RdpOutputEvent::PointerHidden => {
                    let cursor = gdk::Cursor::from_name("none", None);
                    self.obj().set_cursor(cursor.as_ref());
                }
                RdpOutputEvent::PointerPosition { x, y } => {
                    debug!("PointerPosition {} {}", x, y);
                }
                RdpOutputEvent::PointerBitmap(pointer) => {
                    debug!(width = ?pointer.width, height = ?pointer.height, "Received pointer bitmap");
                    let bitmap_bytes = glib::Bytes::from_owned(pointer.bitmap_data.clone());
                    let texture = gdk::MemoryTexture::new(
                        pointer.width.into(),
                        pointer.height.into(),
                        gdk::MemoryFormat::R8g8b8a8,
                        &bitmap_bytes,
                        (pointer.width * 4).into());

                    let cursor = gdk::Cursor::from_texture(
                        &texture,
                        pointer.hotspot_x.into(),
                        pointer.hotspot_x.into(),
                        None);
                    self.obj().set_cursor(Some(&cursor));
                }
            }

            glib::ControlFlow::Continue
        }

        fn send_fast_path_events(
            &self,
            input_events: smallvec::SmallVec<
                [ironrdp::pdu::input::fast_path::FastPathInputEvent; 2],
            >,
        ) {
            if !input_events.is_empty() {
                let _ = self
                    .input_sender
                    .get()
                    .unwrap()
                    .send_blocking(RdpInputEvent::FastPath(input_events));
            }
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for IronRdpWidget {
        fn constructed(&self) {
            self.parent_constructed();

            assert!(
                self.input_database
                    .set(RefCell::new(ironrdp::input::Database::new()))
                    .is_ok()
            );

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

            let event_controller_motion = gtk::EventControllerMotion::new();
            event_controller_motion.connect_motion(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                move |_controller, x, y| {
                    if imp.state.get() != RdpState::Connected {
                        return;
                    }
                    let operation = ironrdp::input::Operation::MouseMove(
                        ironrdp::input::MousePosition {
                            x: x as u16,
                            y: y as u16,
                        },
                    );
                    let input_events = imp
                        .input_database
                        .get()
                        .unwrap()
                        .borrow_mut()
                        .apply(core::iter::once(operation));
                    imp.send_fast_path_events(input_events);
            }));
            self.obj().add_controller(event_controller_motion);

            let event_controller = gtk::EventControllerLegacy::new();
            event_controller.connect_event(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_controller, event: &gdk::Event| -> glib::Propagation {
                    match event.event_type() {
                        gdk::EventType::ButtonRelease | gdk::EventType::ButtonPress => {
                            let button_event =
                                event.clone().downcast::<gdk::ButtonEvent>().unwrap();
                            let mouse_button = match button_event.button() {
                                gdk::BUTTON_PRIMARY => ironrdp::input::MouseButton::Left,
                                gdk::BUTTON_SECONDARY => ironrdp::input::MouseButton::Right,
                                gdk::BUTTON_MIDDLE => ironrdp::input::MouseButton::Middle,
                                _ => {
                                    return glib::Propagation::Proceed;
                                }
                            };
                            let operation = match event.event_type() {
                                gdk::EventType::ButtonPress => {
                                    ironrdp::input::Operation::MouseButtonPressed(mouse_button)
                                }
                                gdk::EventType::ButtonRelease => {
                                    ironrdp::input::Operation::MouseButtonReleased(mouse_button)
                                }
                                _ => {
                                    return glib::Propagation::Proceed;
                                }
                            };
                            let input_events = imp
                                .input_database
                                .get()
                                .unwrap()
                                .borrow_mut()
                                .apply(core::iter::once(operation));
                            imp.send_fast_path_events(input_events);
                            glib::Propagation::Stop
                        }
                        _ => glib::Propagation::Proceed,
                    }
                }
            ));
            self.obj().add_controller(event_controller);
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![Signal::builder("connection-failed")
                    .param_types([String::static_type()])
                    .build()]
            })
        }
    }

    impl WidgetImpl for IronRdpWidget {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(texture) = self.texture.borrow().as_ref() {
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
        width: u16,
        height: u16
    ) {
        self.imp()
            .connect_to_server(hostname, port, username, password, width, height);
    }

    pub fn disconnect(&self) {
        self.imp().disconnect();
    }
}


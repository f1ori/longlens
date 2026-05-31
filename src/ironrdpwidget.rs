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

use ironrdp_client::config::{ClipboardType, Config, Destination};
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use ironrdp::connector::{self, Credentials};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use gtk::glib::prelude::*;
use gtk::glib::subclass::Signal;
use gtk::glib::{self, Properties};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, OnceCell, RefCell};
use std::sync::OnceLock;
use tracing::{debug, info, warn};

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
        input_sender: RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpInputEvent>>>,
        texture: RefCell<Option<gdk::MemoryTexture>>,
        input_database: OnceCell<RefCell<ironrdp::input::Database>>,
        output_relay: OnceCell<async_channel::Sender<RdpOutputEvent>>,
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
            height: u16,
        ) {
            info!("Connecting to {}:{} {}x{}", hostname, port, width, height);

            // Gracefully close any existing connection
            if let Some(sender) = self.input_sender.borrow().as_ref() {
                let _ = sender.send(RdpInputEvent::Close);
            }

            self.obj().set_state(RdpState::Connecting);

            let codecs: Vec<&str> = vec![];
            let codecs = match client_codecs_capabilities(&codecs) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Could not build codec capabilities: {}", e);
                    return;
                }
            };

            let connector_config = connector::Config {
                credentials: Credentials::UsernamePassword { username, password },
                domain: None,
                enable_tls: true,
                enable_credssp: true,
                keyboard_type: KeyboardType::IbmEnhanced,
                keyboard_subtype: 0,
                keyboard_layout: 0,
                keyboard_functional_keys_count: 12,
                ime_file_name: String::new(),
                dig_product_id: String::new(),
                desktop_size: connector::DesktopSize { width, height },
                desktop_scale_factor: 0,
                bitmap: Some(connector::BitmapConfig {
                    color_depth: 32,
                    lossy_compression: true,
                    codecs,
                }),
                client_build: 42,
                client_name: String::from("longlens"),
                client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
                alternate_shell: String::new(),
                work_dir: String::new(),
                compression_type: None,
                multitransport_flags: None,
                platform: match whoami::platform() {
                    whoami::Platform::Windows => MajorPlatformType::WINDOWS,
                    whoami::Platform::Linux => MajorPlatformType::UNIX,
                    whoami::Platform::MacOS => MajorPlatformType::MACINTOSH,
                    whoami::Platform::Ios => MajorPlatformType::IOS,
                    whoami::Platform::Android => MajorPlatformType::ANDROID,
                    _ => MajorPlatformType::UNSPECIFIED,
                },
                hardware_id: None,
                license_cache: None,
                enable_server_pointer: true,
                autologon: false,
                enable_audio_playback: true,
                request_data: None,
                pointer_software_rendering: false,
                performance_flags: PerformanceFlags::default(),
                timezone_info: TimezoneInfo::default(),
            };

            let config = Config {
                destination: Destination::from_parts(hostname, port),
                connector: connector_config,
                clipboard_type: ClipboardType::Disable,
                log_file: None,
                gw: None,
                kerberos_config: None,
                rdcleanpath: None,
                fake_events_interval: None,
                dvc_pipe_proxies: vec![],
            };

            let (input_tx, input_rx) = RdpInputEvent::create_channel();
            let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<RdpOutputEvent>(64);
            let dvc_factory = DvcPipeProxyFactory::new(input_tx.clone());

            *self.input_sender.borrow_mut() = Some(input_tx);

            let relay_tx = self.output_relay.get().unwrap().clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .unwrap();

                let client = RdpClient {
                    config,
                    output_event_sender: output_tx,
                    input_event_receiver: input_rx,
                    cliprdr_factory: None,
                    dvc_pipe_proxy_factory: dvc_factory,
                };

                rt.block_on(async move {
                    tokio::spawn(async move {
                        while let Some(event) = output_rx.recv().await {
                            if relay_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    });

                    client.run().await;
                });
            });
        }

        pub fn disconnect(&self) {
            if let Some(sender) = self.input_sender.borrow().as_ref() {
                let _ = sender.send(RdpInputEvent::Close);
            }
        }

        fn process_message(&self, output_event: RdpOutputEvent) {
            match output_event {
                RdpOutputEvent::ConnectionFailure(reason) => {
                    self.obj().set_state(RdpState::Disconnected);
                    let mut error_string = reason.to_string();
                    let mut source = std::error::Error::source(&reason);
                    while let Some(e) = source {
                        error_string.push_str(&format!(": {e}"));
                        source = e.source();
                    }
                    self.obj()
                        .emit_by_name::<()>("connection-failed", &[&error_string]);
                }
                RdpOutputEvent::Terminated(Ok(reason)) => {
                    info!("Session terminated: {}", reason);
                    self.obj().set_state(RdpState::Disconnected);
                }
                RdpOutputEvent::Terminated(Err(e)) => {
                    warn!("Session error: {}", e);
                    self.obj().set_state(RdpState::Disconnected);
                }
                RdpOutputEvent::Image { buffer, width, height } => {
                    if self.state.get() == RdpState::Connecting {
                        info!("State connected (first frame received)");
                        self.obj().set_state(RdpState::Connected);
                    }
                    // ironrdp encodes pixels as 0x00RRGGBB (big-endian [0, r, g, b])
                    let byte_buf: Vec<u8> = buffer
                        .into_iter()
                        .flat_map(|p| {
                            let [_, r, g, b] = p.to_be_bytes();
                            [r, g, b, 0xFF]
                        })
                        .collect();
                    let bytes = glib::Bytes::from_owned(byte_buf);
                    *self.texture.borrow_mut() = Some(gdk::MemoryTexture::new(
                        width.get().into(),
                        height.get().into(),
                        gdk::MemoryFormat::R8g8b8a8,
                        &bytes,
                        (width.get() as usize) * 4,
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
                        (pointer.width as usize) * 4,
                    );
                    let cursor = gdk::Cursor::from_texture(
                        &texture,
                        pointer.hotspot_x.into(),
                        pointer.hotspot_y.into(),
                        None,
                    );
                    self.obj().set_cursor(Some(&cursor));
                }
            }
        }

        fn send_input_operation(&self, operation: ironrdp::input::Operation) {
            let input_events = self
                .input_database
                .get()
                .unwrap()
                .borrow_mut()
                .apply(core::iter::once(operation));
            self.send_fast_path_events(input_events);
        }

        fn send_fast_path_events(
            &self,
            input_events: smallvec::SmallVec<
                [ironrdp::pdu::input::fast_path::FastPathInputEvent; 2],
            >,
        ) {
            if !input_events.is_empty() {
                if let Some(sender) = self.input_sender.borrow().as_ref() {
                    let _ = sender.send(RdpInputEvent::FastPath(input_events));
                }
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

            let (relay_tx, relay_rx) =
                async_channel::bounded::<RdpOutputEvent>(64);
            self.output_relay.set(relay_tx).unwrap();

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to=imp)]
                self,
                async move {
                    while let Ok(event) = relay_rx.recv().await {
                        imp.process_message(event);
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
                    let operation =
                        ironrdp::input::Operation::MouseMove(ironrdp::input::MousePosition {
                            x: x as u16,
                            y: y as u16,
                        });
                    imp.send_input_operation(operation);
                }
            ));
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
                            imp.send_input_operation(operation);
                            glib::Propagation::Stop
                        }
                        gdk::EventType::KeyPress | gdk::EventType::KeyRelease => {
                            let key_event = event.clone().downcast::<gdk::KeyEvent>().unwrap();
                            let keycode: u16 = key_event.keycode().try_into().unwrap();
                            let map = keycode::KeyMap::from_key_mapping(
                                keycode::KeyMapping::Xkb(keycode),
                            );
                            let scancode = match map {
                                Ok(map) => map.win,
                                Err(_) => {
                                    warn!("Unknown keycode {}", keycode);
                                    return glib::Propagation::Proceed;
                                }
                            };
                            let scancode = ironrdp::input::Scancode::from_u16(scancode);
                            let operation = match event.event_type() {
                                gdk::EventType::KeyPress => {
                                    ironrdp::input::Operation::KeyPressed(scancode)
                                }
                                gdk::EventType::KeyRelease => {
                                    ironrdp::input::Operation::KeyReleased(scancode)
                                }
                                _ => {
                                    return glib::Propagation::Proceed;
                                }
                            };
                            imp.send_input_operation(operation);
                            glib::Propagation::Stop
                        }
                        _ => glib::Propagation::Proceed,
                    }
                }
            ));
            self.obj().add_controller(event_controller);
            self.obj().set_focusable(true);
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("connection-failed")
                        .param_types([String::static_type()])
                        .build(),
                ]
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
        height: u16,
    ) {
        self.imp()
            .connect_to_server(hostname, port, username, password, width, height);
    }

    pub fn disconnect(&self) {
        self.imp().disconnect();
    }
}

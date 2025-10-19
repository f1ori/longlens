use core::num::NonZeroU16;
use std::sync::Arc;

use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::{ConnectionResult, ConnectorResult, Credentials};
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::graphics::pointer::DecodedPointer;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{
    ActiveStage, ActiveStageOutput, GracefulDisconnectReason, SessionResult, fast_path,
};
use ironrdp::{connector, session};
use ironrdp_core::WriteBuf;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite, single_sequence_step_read, split_tokio_framed};
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tracing::{debug, trace};

#[derive(Debug)]
pub enum RdpInputEvent {
    Connect {
        hostname: String,
        port: u16,
        username: String,
        password: String,
        width: u16,
        height: u16,
    },
    Resize {
        width: u16,
        height: u16,
        scale_factor: u32,
        /// The physical size of the display in millimeters (width, height).
        physical_size: Option<(u32, u32)>,
    },
    FastPath(SmallVec<[FastPathInputEvent; 2]>),
    Close,
    //Clipboard(ClipboardMessage),
}

#[derive(Debug)]
pub enum RdpOutputEvent {
    Connected,
    Image {
        buffer: Vec<u8>,
        width: NonZeroU16,
        height: NonZeroU16,
    },
    PointerDefault,
    PointerHidden,
    PointerPosition {
        x: u16,
        y: u16,
    },
    PointerBitmap(Arc<DecodedPointer>),
    Terminated(SessionResult<GracefulDisconnectReason>),
}

enum RdpControlFlow {
    //ReconnectWithNewSize { width: u16, height: u16 },
    TerminatedGracefully(GracefulDisconnectReason),
}

trait AsyncReadWrite: AsyncRead + AsyncWrite {}

impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite {}

type UpgradedFramed = ironrdp_tokio::TokioFramed<Box<dyn AsyncReadWrite + Unpin + Send + Sync>>;

pub struct RdpClient {
    input_receiver: async_channel::Receiver<RdpInputEvent>,
    output_sender: async_channel::Sender<RdpOutputEvent>,
}

impl RdpClient {
    async fn unconnected_loop(&self) {
        while let Ok(event) = self.input_receiver.recv().await {
            match event {
                RdpInputEvent::Connect {
                    hostname,
                    port,
                    username,
                    password,
                    width,
                    height,
                } => {
                    let (connection_result, framed) = match self
                        .connect(hostname, port, username, password, width, height)
                        .await
                    {
                        Ok(result) => result,
                        Err(e) => {
                            println!("Failed to connect: {}", e);
                            continue;
                        }
                    };
                    self.output_sender
                        .send(RdpOutputEvent::Connected {})
                        .await
                        .expect("Channel broken");
                    match self.active_session(framed, connection_result).await {
                        Ok(RdpControlFlow::TerminatedGracefully(reason)) => {
                            self.output_sender
                                .send(RdpOutputEvent::Terminated(Ok(reason)))
                                .await
                                .expect("Channel broken");
                        }
                        Err(e) => {
                            self.output_sender
                                .send(RdpOutputEvent::Terminated(Err(e)))
                                .await
                                .expect("Channel broken");
                        }
                    }
                }
                _ => {
                    println!("Unexpected event");
                }
            }
        }
        println!("Input loop ended");
    }

    async fn active_session(
        &self,
        framed: UpgradedFramed,
        connection_result: ConnectionResult,
    ) -> SessionResult<RdpControlFlow> {
        let (mut reader, mut writer) = split_tokio_framed(framed);
        let mut image = DecodedImage::new(
            PixelFormat::RgbA32,
            connection_result.desktop_size.width,
            connection_result.desktop_size.height,
        );

        let mut active_stage = ActiveStage::new(connection_result);

        let disconnect_reason = 'outer: loop {
            let outputs = tokio::select! {
                frame = reader.read_pdu() => {
                    let (action, payload) = frame.map_err(|e| session::custom_err!("read frame", e))?;
                    trace!(?action, frame_length = payload.len(), "Frame received");

                    active_stage.process(&mut image, action, &payload)?
                }
                input_event = self.input_receiver.recv() => {
                    let input_event = input_event.map_err(|_| session::general_err!("GUI is stopped"))?;

                    match input_event {
                        RdpInputEvent::Close => {
                            active_stage.graceful_shutdown()?
                        }
                        RdpInputEvent::FastPath(events) => {
                            trace!(?events);
                            active_stage.process_fastpath_input(&mut image, &events)?
                        }
                        _ => {
                            println! ("inner loop unhandled event");
                            Vec::new()
                        }
                    }
                }
            };
            for out in outputs {
                match out {
                    ActiveStageOutput::ResponseFrame(frame) => writer
                        .write_all(&frame)
                        .await
                        .map_err(|e| session::custom_err!("write response", e))?,
                    ActiveStageOutput::GraphicsUpdate(_region) => {
                        let buffer: Vec<u8> = image.data().to_vec();

                        self.output_sender
                            .send(RdpOutputEvent::Image {
                                buffer,
                                width: NonZeroU16::new(image.width())
                                    .ok_or_else(|| session::general_err!("width is zero"))?,
                                height: NonZeroU16::new(image.height())
                                    .ok_or_else(|| session::general_err!("height is zero"))?,
                            })
                            .await
                            .expect("Could not send Image event");
                    }
                    ActiveStageOutput::PointerDefault => {
                        self.output_sender
                            .send(RdpOutputEvent::PointerDefault)
                            .await
                            .expect("Could not send pointer default");
                    }
                    ActiveStageOutput::PointerHidden => {
                        self.output_sender
                            .send(RdpOutputEvent::PointerHidden)
                            .await
                            .expect("Could not send pointer hidden");
                    }
                    ActiveStageOutput::PointerPosition { x, y } => {
                        self.output_sender
                            .send(RdpOutputEvent::PointerPosition { x, y })
                            .await
                            .expect("Could not send pointer event");
                    }
                    ActiveStageOutput::PointerBitmap(pointer) => {
                        self.output_sender
                            .send(RdpOutputEvent::PointerBitmap(pointer))
                            .await
                            .expect("Could not send pointer bitmap");
                    }
                    ActiveStageOutput::DeactivateAll(mut connection_activation) => {
                        // Execute the Deactivation-Reactivation Sequence:
                        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
                        debug!(
                            "Received Server Deactivate All PDU, executing Deactivation-Reactivation Sequence"
                        );
                        let mut buf = WriteBuf::new();
                        'activation_seq: loop {
                            let written = single_sequence_step_read(
                                &mut reader,
                                &mut *connection_activation,
                                &mut buf,
                            )
                            .await
                            .map_err(|e| {
                                session::custom_err!(
                                    "read deactivation-reactivation sequence step",
                                    e
                                )
                            })?;

                            if written.size().is_some() {
                                writer.write_all(buf.filled()).await.map_err(|e| {
                                    session::custom_err!(
                                        "write deactivation-reactivation sequence step",
                                        e
                                    )
                                })?;
                            }

                            if let ConnectionActivationState::Finalized {
                                io_channel_id,
                                user_channel_id,
                                desktop_size,
                                enable_server_pointer,
                                pointer_software_rendering,
                            } = connection_activation.state
                            {
                                debug!(
                                    ?desktop_size,
                                    "Deactivation-Reactivation Sequence completed"
                                );
                                // Update image size with the new desktop size.
                                image = DecodedImage::new(
                                    PixelFormat::RgbA32,
                                    desktop_size.width,
                                    desktop_size.height,
                                );
                                // Update the active stage with the new channel IDs and pointer settings.
                                active_stage.set_fastpath_processor(
                                    fast_path::ProcessorBuilder {
                                        io_channel_id,
                                        user_channel_id,
                                        enable_server_pointer,
                                        pointer_software_rendering,
                                    }
                                    .build(),
                                );
                                active_stage.set_enable_server_pointer(enable_server_pointer);
                                break 'activation_seq;
                            }
                        }
                    }
                    ActiveStageOutput::Terminate(reason) => break 'outer reason,
                }
            }
        };
        Ok(RdpControlFlow::TerminatedGracefully(disconnect_reason))
    }

    async fn connect(
        &self,
        hostname: String,
        port: u16,
        username: String,
        password: String,
        width: u16,
        height: u16,
    ) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
        let codecs: Vec<&str> = vec![];
        let codecs = match client_codecs_capabilities(&codecs) {
            Ok(codecs) => codecs,
            Err(help) => {
                panic!("Could not parse codecs {}", help);
            }
        };
        let bitmap = connector::BitmapConfig {
            color_depth: 32,
            lossy_compression: true,
            codecs,
        };
        let config = connector::Config {
            credentials: Credentials::UsernamePassword { username, password },
            domain: None,
            enable_tls: true,
            enable_credssp: true,
            keyboard_type: ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
            keyboard_subtype: 0,
            keyboard_layout: 0, // the server SHOULD use the default active input locale identifier
            keyboard_functional_keys_count: 12,
            ime_file_name: String::from(""),
            dig_product_id: String::from(""),
            desktop_size: connector::DesktopSize { width, height },
            desktop_scale_factor: 0, // Default to 0 per FreeRDP
            bitmap: Some(bitmap),
            client_build: 42,
            client_name: String::from("fernsichtrdp"),
            // NOTE: hardcode this value like in freerdp
            // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
            client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
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
        let destination = format!("{}:{}", hostname, port);
        let stream = TcpStream::connect(destination)
            .await
            .map_err(|e| connector::custom_err!("TCP connect", e))?;
        let client_addr = stream
            .local_addr()
            .map_err(|e| connector::custom_err!("get socket local address", e))?;
        let mut framed = ironrdp_tokio::TokioFramed::new(stream);
        let mut connector = connector::ClientConnector::new(config, client_addr);
        // TODO add additional channels for clipboard, sound, filesystem, ...

        let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await?;
        debug!("TLS upgrade");
        // Ensure there is no leftover
        let (initial_stream, leftover_bytes) = framed.into_inner();

        let (upgraded_stream, server_public_key) = ironrdp_tls::upgrade(initial_stream, &hostname)
            .await
            .map_err(|e| connector::custom_err!("TLS upgrade", e))?;

        let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

        let erased_stream =
            Box::new(upgraded_stream) as Box<dyn AsyncReadWrite + Unpin + Send + Sync>;
        let mut upgraded_framed =
            ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

        let connection_result = ironrdp_tokio::connect_finalize(
            upgraded,
            &mut upgraded_framed,
            connector,
            hostname.into(),
            server_public_key,
            Some(&mut ReqwestNetworkClient::new()),
            None,
        )
        .await?;

        debug!(?connection_result);

        Ok((connection_result, upgraded_framed))
    }
}

pub async fn start_rdp(
    input_receiver: async_channel::Receiver<RdpInputEvent>,
    output_sender: async_channel::Sender<RdpOutputEvent>,
) {
    let rdp_client = RdpClient {
        input_receiver,
        output_sender,
    };
    rdp_client.unconnected_loop().await;
}


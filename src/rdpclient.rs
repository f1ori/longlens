use ironrdp::connector::{self, Credentials};
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};

#[derive(Debug)]
pub enum RdpInputEvent {
    Connect {
        hostname: String,
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
    // FastPath(SmallVec<[FastPathInputEvent; 2]>),
    Close,
    //Clipboard(ClipboardMessage),
}

#[derive(Debug)]
pub enum RdpOutputEvent {
    Connected,
    ConnectionFailure(String),
    // PointerDefault,
    // PointerHidden,
    // PointerPosition {
    //     x: u16,
    //     y: u16,
    // },
    // PointerBitmap(Arc<DecodedPointer>),
    // Terminated(SessionResult<GracefulDisconnectReason>),
}

pub struct RdpClient {}

impl RdpClient {
    fn connect(&self, host: String, username: String, password: String) {
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
            enable_credssp: false,
            keyboard_type: ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
            keyboard_subtype: 0,
            keyboard_layout: 0, // the server SHOULD use the default active input locale identifier
            keyboard_functional_keys_count: 12,
            ime_file_name: String::from(""),
            dig_product_id: String::from(""),
            desktop_size: connector::DesktopSize {
                width: 1024,
                height: 768,
            },
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
    }
}

pub async fn start_rdp(
    input_receiver: async_channel::Receiver<RdpInputEvent>,
    _output_sender: async_channel::Sender<RdpOutputEvent>,
) {
    let rdp_client = RdpClient {};
    while let Ok(event) = input_receiver.recv().await {
        match event {
            RdpInputEvent::Connect {hostname, username, password, width: _, height: _} => {
                rdp_client.connect(hostname, username, password);
            }
            RdpInputEvent::Resize { width: _, height: _, scale_factor: _, physical_size: _ } => {
                println!("Resize event");
            }
            RdpInputEvent::Close => {
                println!("Close event")

            }
        }
    }
    println!("Input loop ended");
}



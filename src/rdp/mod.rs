/* rdp/mod.rs
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

//! GTK widget and FreeRDP session integration.

mod clipboard_codec;
mod config;
pub(crate) mod ffi;
mod input;
mod render;
mod session;

pub use config::parse_hostname_port;

use gettextrs::gettext;
use adw::prelude::*;
use gtk::glib::subclass::Signal;
use gtk::glib::{self, Properties};
use gtk::gio;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{OnceLock, mpsc};
use tracing::{info, warn};

use crate::model::destination_object::ConnectionOptions;

use session::{
    CertificateDecision, CertificateDetails, ConnectionError, LocalClipboardFile,
    RemoteClipboardFile, Session, SessionEvent,
};

const GRACEFUL_DISCONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, glib::Enum, Default)]
#[enum_type(name = "RdpState")]
pub enum RdpState {
    #[default]
    Disconnected = 0,
    Connecting = 1,
    Connected = 2,
}

const PORTAL_FILETRANSFER_MIME: &str = "application/vnd.portal.filetransfer";
const REMOTE_FILE_CHUNK_SIZE: u32 = 32 * 1024;
const REMOTE_FILE_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

glib::wrapper! {
    pub struct PortalFileTransferProvider(ObjectSubclass<portal_provider::PortalFileTransferProvider>)
        @extends gdk::ContentProvider;
}

impl PortalFileTransferProvider {
    fn new(widget: &RdpWidget, session: Session, files: Vec<RemoteClipboardFile>) -> Self {
        info!(count = files.len(), ?files, "Creating portal file-transfer clipboard provider");
        let provider: Self = glib::Object::new();
        provider.imp().state.replace(Some(portal_provider::TransferState {
            widget: widget.downgrade(),
            session,
            files,
        }));
        provider
    }
}

mod portal_provider {
    use super::*;

    #[derive(Clone)]
    pub struct TransferState {
        pub widget: glib::WeakRef<RdpWidget>,
        pub session: Session,
        pub files: Vec<RemoteClipboardFile>,
    }

    #[derive(Default)]
    pub struct PortalFileTransferProvider {
        pub state: RefCell<Option<TransferState>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PortalFileTransferProvider {
        const NAME: &'static str = "LonglensPortalFileTransferProvider";
        type Type = super::PortalFileTransferProvider;
        type ParentType = gdk::ContentProvider;
    }

    impl ObjectImpl for PortalFileTransferProvider {}

    impl gdk::subclass::prelude::ContentProviderImpl for PortalFileTransferProvider {
        fn attach_clipboard(&self, _clipboard: &gdk::Clipboard) {
            info!("Portal file-transfer provider attached to clipboard");
        }

        fn detach_clipboard(&self, _clipboard: &gdk::Clipboard) {
            info!("Portal file-transfer provider detached from clipboard");
        }

        fn formats(&self) -> gdk::ContentFormats {
            info!("Portal file-transfer provider formats requested");
            gdk::ContentFormats::builder()
                .add_mime_type(PORTAL_FILETRANSFER_MIME)
                .build()
        }

        fn storable_formats(&self) -> gdk::ContentFormats {
            self.formats()
        }

        fn write_mime_type_future(
            &self,
            mime_type: &str,
            stream: &gio::OutputStream,
            _io_priority: glib::Priority,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), glib::Error>> + 'static>>
        {
            info!(mime_type, "Portal file-transfer provider write requested");
            let state = self.state.borrow().clone();
            let stream = stream.clone();
            let mime_type = mime_type.to_owned();
            Box::pin(async move {
                if mime_type != PORTAL_FILETRANSFER_MIME {
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::NotSupported,
                        "Unsupported clipboard MIME type",
                    ));
                }
                let Some(state) = state else {
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::Failed,
                        "Clipboard file transfer is no longer available",
                    ));
                };
                let Some(widget) = state.widget.upgrade() else {
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::Failed,
                        "RDP widget is no longer available",
                    ));
                };
                info!(mime_type, count = state.files.len(), "Starting on-demand remote clipboard download");
                let files = download_remote_files_for_portal(&widget, &state.session, &state.files)
                    .await
                    .ok_or_else(|| {
                        glib::Error::new(
                            gio::IOErrorEnum::Failed,
                            "Could not download remote clipboard files",
                        )
                    })?;
                let data = start_portal_file_transfer(files).await.map_err(|error| {
                    glib::Error::new(gio::IOErrorEnum::Failed, &error)
                })?;
                info!(bytes = data.len(), mime_type, "Writing clipboard file-transfer response data");
                stream
                    .write_all_future(data, glib::Priority::DEFAULT)
                    .await
                    .map(|_| ())
                    .map_err(|(_, error)| error)
            })
        }
    }
}

async fn download_remote_files_for_portal(
    widget: &RdpWidget,
    session: &Session,
    files: &[RemoteClipboardFile],
) -> Option<Vec<std::fs::File>> {
    info!(count = files.len(), "Preparing on-demand remote clipboard download");
    let dir = glib::user_cache_dir()
        .join("longlens")
        .join("clipboard")
        .join(uuid::Uuid::new_v4().to_string());
    if let Err(error) = std::fs::create_dir_all(&dir) {
        warn!("Could not create clipboard temp directory: {error}");
        return None;
    }

    info!(path = %dir.display(), "Created remote clipboard temp directory");
    let mut out_files = Vec::new();
    let mut used_names = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        info!(index, name = %file.name, size = file.size, is_directory = file.is_directory, "Downloading remote clipboard file");
        if file.is_directory {
            warn!("Skipping remote clipboard directory {}; portal file transfer only supports regular files", file.name);
            continue;
        }

        let unique_name = unique_clipboard_file_name(&file.name, &mut used_names);
        let path = dir.join(&unique_name);
        if unique_name != file.name {
            info!(index, remote_name = %file.name, local_name = %unique_name, "Renamed duplicate remote clipboard file");
        }
        let mut out = match std::fs::File::create(&path) {
            Ok(out) => out,
            Err(error) => {
                warn!("Could not create clipboard file {}: {error}", path.display());
                continue;
            }
        };

        let size_stream_id = session.next_stream_id();
        let (size_sender, size_receiver) = async_channel::bounded(1);
        register_pending_file_contents(widget, size_stream_id, size_sender);
        session.request_clipboard_file_size(size_stream_id, index as u32);
        match size_receiver.recv().await {
            Ok(data) if data.len() == 8 => {
                let remote_size = u64::from_le_bytes(data.as_slice().try_into().ok()?);
                info!(index, descriptor_size = file.size, remote_size, "Received remote clipboard file size");
            }
            Ok(data) => warn!(index, len = data.len(), "Unexpected remote clipboard file size response"),
            Err(error) => warn!(index, error = %error, "Could not receive remote clipboard file size"),
        }

        let mut offset = 0u64;
        while offset < file.size {
            let size = REMOTE_FILE_CHUNK_SIZE.min((file.size - offset) as u32);
            let stream_id = session.next_stream_id();
            let (sender, receiver) = async_channel::bounded(1);
            register_pending_file_contents(widget, stream_id, sender);
            session.request_clipboard_file_contents(stream_id, index as u32, offset, size);
            let data = match receiver.recv().await {
                Ok(data) => data,
                Err(error) => {
                    warn!("Could not receive remote clipboard file data: {error}");
                    break;
                }
            };
            if data.is_empty() {
                warn!(index, offset, "Remote clipboard file response was empty");
                break;
            }
            if let Err(error) = out.write_all(&data) {
                warn!(index, offset, error = %error, "Could not write remote clipboard file data");
                break;
            }
            offset += data.len() as u64;
            info!(index, offset, total = file.size, "Wrote remote clipboard file chunk");
        }

        if offset == file.size {
            info!(index, path = %path.display(), "Completed remote clipboard file download");
            drop(out);
            match std::fs::File::open(&path) {
                Ok(file) => out_files.push(file),
                Err(error) => warn!("Could not reopen clipboard file {}: {error}", path.display()),
            }
        } else {
            warn!(index, received = offset, expected = file.size, "Remote clipboard file download incomplete");
        }
    }

    info!(count = out_files.len(), "Finished on-demand remote clipboard download");
    if !out_files.is_empty() {
        session.unlock_remote_clipboard_files();
        Some(out_files)
    } else {
        None
    }
}

fn register_pending_file_contents(
    widget: &RdpWidget,
    stream_id: u32,
    sender: async_channel::Sender<Vec<u8>>,
) {
    widget
        .imp()
        .pending_file_contents
        .borrow_mut()
        .insert(stream_id, sender);

    let widget = widget.downgrade();
    glib::timeout_add_local_once(REMOTE_FILE_RESPONSE_TIMEOUT, move || {
        let Some(widget) = widget.upgrade() else {
            return;
        };
        if widget
            .imp()
            .pending_file_contents
            .borrow_mut()
            .remove(&stream_id)
            .is_some()
        {
            warn!(stream_id, "Timed out waiting for remote clipboard file response");
        }
    });
}

fn unique_clipboard_file_name(name: &str, used_names: &mut HashMap<String, u32>) -> String {
    let mut next_suffix = used_names.get(name).copied().unwrap_or(0);
    if next_suffix == 0 {
        used_names.insert(name.to_owned(), 1);
        return name.to_owned();
    }

    loop {
        next_suffix += 1;
        let candidate = numbered_file_name(name, next_suffix);
        if !used_names.contains_key(&candidate) {
            used_names.insert(name.to_owned(), next_suffix);
            used_names.insert(candidate.clone(), 1);
            return candidate;
        }
    }
}

fn numbered_file_name(name: &str, number: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({number}).{extension}"),
        _ => format!("{name} ({number})"),
    }
}

struct PortalTransferRequest {
    files: Vec<std::fs::File>,
    response: async_channel::Sender<Result<Vec<u8>, String>>,
}

fn portal_transfer_sender() -> &'static async_channel::Sender<PortalTransferRequest> {
    static SENDER: OnceLock<async_channel::Sender<PortalTransferRequest>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("longlens-portal-file-transfer".into())
            .spawn(move || portal_transfer_worker(receiver))
            .expect("failed to start portal file-transfer worker");
        sender
    })
}

fn portal_transfer_worker(receiver: async_channel::Receiver<PortalTransferRequest>) {
    async_io::block_on(async move {
        info!("Opening shared xdg-desktop-portal D-Bus connection");
        let connection = match ashpd::zbus::Connection::session().await {
            Ok(connection) => connection,
            Err(error) => {
                warn!(error = %error, "Could not open shared xdg-desktop-portal D-Bus connection");
                while let Ok(request) = receiver.recv().await {
                    let _ = request.response.send(Err(error.to_string())).await;
                }
                return;
            }
        };

        while let Ok(request) = receiver.recv().await {
            let connection = connection.clone();
            let result = start_portal_file_transfer_on_connection(connection.clone(), request.files)
                .await;
            match result {
                Ok((key, data)) => {
                    let _ = request.response.send(Ok(data)).await;
                    keep_portal_transfer_alive(connection, key);
                }
                Err(error) => {
                    let _ = request.response.send(Err(error)).await;
                }
            }
        }
    });
}

async fn start_portal_file_transfer_on_connection(
    connection: ashpd::zbus::Connection,
    files: Vec<std::fs::File>,
) -> Result<(String, Vec<u8>), String> {
    info!(count = files.len(), "Opening xdg-desktop-portal FileTransfer proxy");
    let portal = ashpd::documents::file_transfer::FileTransfer::with_connection(connection)
        .await
        .map_err(|error| error.to_string())?;
    info!("Starting xdg-desktop-portal FileTransfer session");
    let key = portal
        .start_transfer(
            ashpd::documents::file_transfer::StartTransferOptions::default()
                .set_writeable(false)
                .set_auto_stop(true),
        )
        .await
        .map_err(|error| error.to_string())?;
    info!(%key, count = files.len(), "Adding files to xdg-desktop-portal FileTransfer session");
    portal
        .add_files(&key, &files, Default::default())
        .await
        .map_err(|error| error.to_string())?;
    info!(%key, "Added files to xdg-desktop-portal FileTransfer session");
    info!(%key, "Writing portal file-transfer key to clipboard request stream");
    Ok((key.clone(), key.into_bytes()))
}

fn keep_portal_transfer_alive(connection: ashpd::zbus::Connection, key: String) {
    std::thread::Builder::new()
        .name("longlens-portal-transfer-keepalive".into())
        .spawn(move || {
            async_io::block_on(async move {
                info!(%key, "Keeping xdg-desktop-portal FileTransfer session alive");
                async_io::Timer::after(std::time::Duration::from_secs(300)).await;
                match ashpd::documents::file_transfer::FileTransfer::with_connection(connection).await {
                    Ok(portal) => {
                        let _ = portal.stop_transfer(&key).await;
                    }
                    Err(error) => warn!(%key, error = %error, "Could not open FileTransfer proxy to stop transfer"),
                }
                info!(%key, "Finished keeping xdg-desktop-portal FileTransfer session alive");
            });
        })
        .expect("failed to start portal transfer keepalive");
}

async fn start_portal_file_transfer(files: Vec<std::fs::File>) -> Result<Vec<u8>, String> {
    let (response, receiver) = async_channel::bounded(1);
    portal_transfer_sender()
        .send(PortalTransferRequest { files, response })
        .await
        .map_err(|error| error.to_string())?;
    receiver.recv().await.map_err(|error| error.to_string())?
}

async fn read_clipboard_files(clipboard: &gdk::Clipboard) -> Option<Vec<LocalClipboardFile>> {
    let value = clipboard
        .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
        .await
        .ok()?;
    let file_list = value.get::<gdk::FileList>().ok()?;
    let mut files = Vec::new();
    for file in file_list.files() {
        let path = file.path()?;
        let info = file
            .query_info(
                "standard::name,standard::size,standard::type",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .ok()?;
        let name = info
            .name()
            .file_name()
            .and_then(|name| name.to_str().map(ToOwned::to_owned))
            .or_else(|| path.file_name().and_then(|name| name.to_str()).map(ToOwned::to_owned))?;
        files.push(LocalClipboardFile {
            path,
            name,
            size: info.size().max(0) as u64,
            is_directory: info.file_type() == gio::FileType::Directory,
        });
    }
    (!files.is_empty()).then_some(files)
}

fn friendly_connection_error(error: &ConnectionError) -> String {
    match error.class {
        1 => gettext(
            "The username or password is incorrect. Please check your credentials and try again.",
        ),
        2 => gettext(
            "Access was denied. The username or password may be incorrect, or this account is not allowed to connect.",
        ),
        _ => {
            let detail = if error.message.is_empty() {
                if error.name.is_empty() {
                    format!("FreeRDP error 0x{:08x}", error.code)
                } else {
                    error.name.clone()
                }
            } else {
                error.message.clone()
            };
            format!("{}\n\n{}", gettext("Could not connect to the server."), detail)
        }
    }
}

mod imp {
    use super::*;

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::RdpWidget)]
    pub struct RdpWidget {
        #[property(get, set, builder(RdpState::Disconnected))]
        state: Cell<RdpState>,
        session: RefCell<Option<Session>>,
        texture: RefCell<Option<gdk::MemoryTexture>>,
        resize_timeout: RefCell<Option<glib::SourceId>>,
        disconnect_timeout: RefCell<Option<glib::SourceId>>,
        pending_certificate: RefCell<Option<mpsc::SyncSender<CertificateDecision>>>,
        last_remote_clipboard: RefCell<Option<String>>,
        pub(super) pending_file_contents: RefCell<HashMap<u32, async_channel::Sender<Vec<u8>>>>,
        suppress_next_clipboard_announce: Cell<bool>,
        pub(super) clipboard_enabled: Cell<bool>,
        pub(super) forward_unicode: Cell<bool>,
        pub(super) inhibit_system_shortcuts: Cell<bool>,
        generation: Cell<u64>,
        pointer_x: Cell<u16>,
        pointer_y: Cell<u16>,
        connection_scale: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RdpWidget {
        const NAME: &'static str = "RdpWidget";
        type Type = super::RdpWidget;
        type ParentType = gtk::Widget;
    }

    impl RdpWidget {
        fn surface_scale(&self) -> f64 {
            self.obj()
                .native()
                .and_then(|native| native.surface())
                .map(|surface| surface.scale())
                .unwrap_or_else(|| self.obj().scale_factor() as f64)
        }

        fn physical_size(
            &self,
            logical_width: f64,
            logical_height: f64,
        ) -> Option<(u16, u16, u32)> {
            let scale = self.surface_scale();
            let width = u16::try_from((logical_width * scale).round() as i64).ok()?;
            let height = u16::try_from((logical_height * scale).round() as i64).ok()?;
            Some((width, height, (scale * 100.0).round() as u32))
        }

        pub fn connect_to_server(
            &self,
            hostname: String,
            port: u16,
            username: String,
            password: secrecy::SecretString,
            width: u16,
            height: u16,
            options: ConnectionOptions,
        ) {
            let Some((width, height, desktop_scale)) =
                self.physical_size(width.into(), height.into())
            else {
                return;
            };
            info!("Connecting to {hostname}:{port} {width}x{height}");

            self.disconnect();
            self.clear_disconnect_watchdog();
            *self.texture.borrow_mut() = None;
            self.obj().queue_draw();

            let generation = self.generation.get().wrapping_add(1);
            self.generation.set(generation);
            self.connection_scale.set(self.surface_scale());
            self.clipboard_enabled.set(options.clipboard_enabled);
            self.obj().set_state(RdpState::Connecting);

            let config = config::build_config(
                hostname,
                port,
                username,
                password,
                width,
                height,
                desktop_scale,
                options.sound_enabled,
            );
            let (output, receiver) = async_channel::bounded(64);
            let Some(session) = Session::spawn(config, output) else {
                self.obj().set_state(RdpState::Disconnected);
                self.obj().emit_by_name::<()>(
                    "connection-failed",
                    &[&gettext("Could not initialize FreeRDP.")],
                );
                return;
            };
            *self.session.borrow_mut() = Some(session);

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    while let Ok(event) = receiver.recv().await {
                        if imp.generation.get() != generation {
                            break;
                        }
                        imp.process_event(event);
                    }
                }
            ));
        }

        pub fn disconnect(&self) {
            if let Some(response) = self.pending_certificate.borrow_mut().take() {
                let _ = response.send(CertificateDecision::Reject);
            }
            let Some(session) = self.session.borrow().as_ref().cloned() else {
                return;
            };
            if self.state.get() == RdpState::Connecting {
                session.abort();
            } else {
                session.disconnect();
                self.arm_disconnect_watchdog(session);
            }
        }

        fn arm_disconnect_watchdog(&self, session: Session) {
            self.clear_disconnect_watchdog();
            let source_id = glib::timeout_add_local_once(
                GRACEFUL_DISCONNECT_TIMEOUT,
                glib::clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        *imp.disconnect_timeout.borrow_mut() = None;
                        if imp.state.get() != RdpState::Disconnected {
                            warn!("Graceful disconnect timed out; forcing connection drop");
                            session.abort();
                        }
                    }
                ),
            );
            *self.disconnect_timeout.borrow_mut() = Some(source_id);
        }

        fn clear_disconnect_watchdog(&self) {
            if let Some(source_id) = self.disconnect_timeout.borrow_mut().take() {
                source_id.remove();
            }
        }

        fn process_event(&self, event: SessionEvent) {
            match event {
                SessionEvent::Frame {
                    buffer,
                    width,
                    height,
                    stride,
                } => {
                    if self.state.get() == RdpState::Connecting {
                        info!("State connected (first frame received)");
                        self.obj().set_state(RdpState::Connected);
                        self.announce_local_clipboard();
                    }
                    if let Some(texture) = render::image_texture(buffer, width, height, stride) {
                        *self.texture.borrow_mut() = Some(texture);
                        self.obj().queue_draw();
                    }
                }
                SessionEvent::Cursor {
                    data,
                    width,
                    height,
                    hotspot_x,
                    hotspot_y,
                } => {
                    if let Some(cursor) = render::pointer_cursor(
                        data,
                        width,
                        height,
                        hotspot_x,
                        hotspot_y,
                        self.connection_scale.get(),
                    ) {
                        self.obj().set_cursor(Some(&cursor));
                    }
                }
                SessionEvent::CursorHidden => {
                    self.obj()
                        .set_cursor(gdk::Cursor::from_name("none", None).as_ref());
                }
                SessionEvent::CursorDefault => {
                    self.obj()
                        .set_cursor(gdk::Cursor::from_name("default", None).as_ref());
                }
                SessionEvent::ClipboardRemoteTextAvailable => {
                    if !self.clipboard_enabled.get() {
                        return;
                    }
                    if let Some(session) = self.session.borrow().as_ref() {
                        session.request_clipboard_text();
                    }
                }
                SessionEvent::ClipboardRemoteFilesAvailable => {
                    if !self.clipboard_enabled.get() {
                        return;
                    }
                    if let Some(session) = self.session.borrow().as_ref() {
                        session.request_clipboard_files();
                    }
                }
                SessionEvent::ClipboardRemoteFiles(files) => {
                    if self.clipboard_enabled.get() {
                        self.set_remote_file_transfer_portal(files);
                    }
                }
                SessionEvent::ClipboardRemoteFileContents { stream_id, data } => {
                    if let Some(sender) = self.pending_file_contents.borrow_mut().remove(&stream_id)
                    {
                        let _ = sender.try_send(data);
                    }
                }
                SessionEvent::ClipboardText(text) => {
                    if !self.clipboard_enabled.get() {
                        return;
                    }
                    info!(chars = text.chars().count(), "Setting local clipboard from remote text");
                    *self.last_remote_clipboard.borrow_mut() = Some(text.clone());
                    self.obj().display().clipboard().set_text(&text);
                }
                SessionEvent::CertificateRequest { details, response } => {
                    self.present_certificate_dialog(details, response);
                }
                SessionEvent::ConnectionFailure(error) => {
                    self.finish_session();
                    let message = friendly_connection_error(&error);
                    self.obj()
                        .emit_by_name::<()>("connection-failed", &[&message]);
                }
                SessionEvent::Terminated(detail) => {
                    if let Some(detail) = detail {
                        warn!("RDP session terminated: {detail}");
                    }
                    self.finish_session();
                }
            }
        }

        fn finish_session(&self) {
            self.clear_disconnect_watchdog();
            self.pending_file_contents.borrow_mut().clear();
            self.session.borrow_mut().take();
            self.obj().set_state(RdpState::Disconnected);
        }

        fn present_certificate_dialog(
            &self,
            details: CertificateDetails,
            response: mpsc::SyncSender<CertificateDecision>,
        ) {
            if let Some(previous) = self.pending_certificate.borrow_mut().replace(response) {
                let _ = previous.send(CertificateDecision::Reject);
            }

            let heading = if details.changed() {
                gettext("The server certificate has changed")
            } else {
                gettext("Untrusted server certificate")
            };
            let mut body = format!(
                "{}: {}:{}\n{}: {}\n{}: {}\n{}: {}",
                gettext("Server"),
                details.host,
                details.port,
                gettext("Subject"),
                details.subject,
                gettext("Issuer"),
                details.issuer,
                gettext("Fingerprint"),
                details.fingerprint
            );
            if !details.common_name.is_empty() {
                body.push_str(&format!(
                    "\n{}: {}",
                    gettext("Common name"),
                    details.common_name
                ));
            }
            if details.host_mismatch {
                body.push_str(&format!(
                    "\n\n{}",
                    gettext("The certificate name does not match this server.")
                ));
            }
            if let Some(old) = details.old_fingerprint.as_deref() {
                body.push_str(&format!(
                    "\n{}: {}",
                    gettext("Previous fingerprint"),
                    old
                ));
            }
            if let Some(old_subject) = details.old_subject.as_deref() {
                body.push_str(&format!(
                    "\n{}: {}",
                    gettext("Previous subject"),
                    old_subject
                ));
            }
            if let Some(old_issuer) = details.old_issuer.as_deref() {
                body.push_str(&format!(
                    "\n{}: {}",
                    gettext("Previous issuer"),
                    old_issuer
                ));
            }

            let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
            dialog.add_response("cancel", &gettext("Cancel"));
            dialog.add_response("once", &gettext("Trust Once"));
            dialog.add_response("always", &gettext("Trust and Remember"));
            dialog.set_response_appearance("always", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let parent = self.obj().root().and_downcast::<gtk::Window>();

            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    let choice = dialog.choose_future(parent.as_ref()).await;
                    let decision = match choice.as_str() {
                        "once" => CertificateDecision::TrustOnce,
                        "always" => CertificateDecision::TrustPermanently,
                        _ => CertificateDecision::Reject,
                    };
                    if let Some(response) = imp.pending_certificate.borrow_mut().take() {
                        let _ = response.send(decision);
                    }
                }
            ));
        }

        fn set_remote_file_transfer_portal(&self, files: Vec<session::RemoteClipboardFile>) {
            let Some(session) = self.session.borrow().as_ref().cloned() else {
                return;
            };
            warn!(count = files.len(), "Setting local clipboard to remote file-transfer provider");
            let provider = PortalFileTransferProvider::new(&self.obj(), session, files);
            self.suppress_next_clipboard_announce.set(true);
            if let Err(error) = self
                .obj()
                .display()
                .clipboard()
                .set_content(Some(&provider))
            {
                self.suppress_next_clipboard_announce.set(false);
                warn!("Could not set portal file transfer clipboard content: {error}");
            }
        }

        pub fn send_key(&self, keycode: u16, pressed: bool) {
            if self.state.get() != RdpState::Connected {
                return;
            }
            if let (Some(scancode), Some(session)) = (
                input::key_scancode(keycode),
                self.session.borrow().as_ref(),
            ) {
                session.send_key(scancode, pressed);
            }
        }

        pub fn send_unicode_char(&self, ch: char, pressed: bool) {
            if self.state.get() != RdpState::Connected {
                return;
            }
            if let Some(session) = self.session.borrow().as_ref() {
                let mut buffer = [0; 2];
                for code in ch.encode_utf16(&mut buffer) {
                    session.send_unicode(*code, pressed);
                }
            }
        }

        fn send_mouse(&self, flags: u16, x: f64, y: f64) {
            if self.state.get() != RdpState::Connected {
                return;
            }
            let scale = self.surface_scale();
            let x = (x * scale).round().clamp(0.0, u16::MAX as f64) as u16;
            let y = (y * scale).round().clamp(0.0, u16::MAX as f64) as u16;
            self.pointer_x.set(x);
            self.pointer_y.set(y);
            if let Some(session) = self.session.borrow().as_ref() {
                session.send_mouse(flags, x, y);
            }
        }

        fn setup_motion_controller(&self) {
            let controller = gtk::EventControllerMotion::new();
            controller.connect_motion(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller, x, y| imp.send_mouse(input::PTR_FLAGS_MOVE, x, y)
            ));
            controller.connect_enter(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller, _x, _y| {
                    if imp.state.get() == RdpState::Connected {
                        let obj = imp.obj();
                        obj.grab_focus();
                        imp.announce_local_clipboard();
                        crate::utils::set_shortcuts_inhibited(&*obj, imp.inhibit_system_shortcuts.get());
                    }
                }
            ));
            controller.connect_leave(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_controller| {
                    let obj = imp.obj();
                    crate::utils::set_shortcuts_inhibited(&*obj, false);
                    if let Some(root) = obj.root() {
                        root.set_focus(None::<&gtk::Widget>);
                    }
                }
            ));
            self.obj().add_controller(controller);
        }

        pub(super) fn announce_local_clipboard(&self) {
            if self.state.get() != RdpState::Connected || !self.clipboard_enabled.get() {
                return;
            }
            let clipboard = self.obj().display().clipboard();
            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    if let Some(files) = read_clipboard_files(&clipboard).await {
                        if let Some(session) = imp.session.borrow().as_ref() {
                            session.set_clipboard_files(files);
                        }
                        return;
                    }

                    let text = match clipboard.read_text_future().await {
                        Ok(Some(text)) => text.to_string(),
                        Ok(None) => return,
                        Err(error) => {
                            warn!("Could not read local clipboard text: {error}");
                            return;
                        }
                    };
                    if imp.last_remote_clipboard.borrow().as_deref() == Some(text.as_str()) {
                        imp.last_remote_clipboard.borrow_mut().take();
                        return;
                    }
                    if let Some(session) = imp.session.borrow().as_ref() {
                        session.set_clipboard_text(text);
                    }
                }
            ));
        }

        fn setup_clipboard(&self) {
            self.obj().display().clipboard().connect_changed(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_clipboard| {
                    if imp.suppress_next_clipboard_announce.replace(false) {
                        warn!("Ignoring clipboard change caused by remote clipboard update");
                        return;
                    }
                    imp.announce_local_clipboard();
                }
            ));
        }

        fn setup_input_controller(&self) {
            let click = gtk::GestureClick::new();
            click.set_button(0);
            click.connect_pressed(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |gesture, _count, x, y| {
                    if let Some(button) = input::mouse_button(gesture.current_button()) {
                        imp.send_mouse(button | input::PTR_FLAGS_DOWN, x, y);
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            ));
            click.connect_released(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |gesture, _count, x, y| {
                    if let Some(button) = input::mouse_button(gesture.current_button()) {
                        imp.send_mouse(button, x, y);
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            ));
            self.obj().add_controller(click);

            let scroll =
                gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
            scroll.connect_scroll(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |controller, dx, dy| {
                    if imp.state.get() != RdpState::Connected {
                        return glib::Propagation::Proceed;
                    }
                    if let Some(session) = imp.session.borrow().as_ref() {
                        for flags in input::scroll_flags(dx, dy, controller.unit()) {
                            session.send_mouse(flags, imp.pointer_x.get(), imp.pointer_y.get());
                        }
                    }
                    glib::Propagation::Stop
                }
            ));
            self.obj().add_controller(scroll);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RdpWidget {
        fn constructed(&self) {
            self.parent_constructed();
            self.setup_motion_controller();
            self.setup_input_controller();
            self.setup_clipboard();
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

        fn dispose(&self) {
            self.disconnect();
        }
    }

    impl WidgetImpl for RdpWidget {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if self.state.get() != RdpState::Connected {
                return;
            }
            if let Some(source_id) = self.resize_timeout.borrow_mut().take() {
                source_id.remove();
            }
            let source_id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(500),
                glib::clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        *imp.resize_timeout.borrow_mut() = None;
                        let Some((width, height, scale)) =
                            imp.physical_size(width.into(), height.into())
                        else {
                            return;
                        };
                        imp.connection_scale.set(imp.surface_scale());
                        if let Some(session) = imp.session.borrow().as_ref() {
                            session.resize(width.into(), height.into(), scale);
                        }
                    }
                ),
            );
            *self.resize_timeout.borrow_mut() = Some(source_id);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let width = self.obj().width() as f32;
            let height = self.obj().height() as f32;
            if let Some(texture) = self.texture.borrow().as_ref() {
                snapshot.append_texture(
                    texture,
                    &gtk::graphene::Rect::new(0.0, 0.0, width, height),
                );
            } else {
                snapshot.append_color(
                    &gdk::RGBA::BLACK,
                    &gtk::graphene::Rect::new(0.0, 0.0, width, height),
                );
            }
            self.parent_snapshot(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct RdpWidget(ObjectSubclass<imp::RdpWidget>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RdpWidget {
    pub fn connect_to_server(
        &self,
        hostname: String,
        port: u16,
        username: String,
        password: secrecy::SecretString,
        width: u16,
        height: u16,
        options: ConnectionOptions,
    ) {
        self.imp().connect_to_server(
            hostname,
            port,
            username,
            password,
            width,
            height,
            options,
        );
    }

    pub fn disconnect(&self) {
        self.imp().disconnect();
    }

    pub fn set_clipboard_enabled(&self, enabled: bool) {
        let imp = self.imp();
        imp.clipboard_enabled.set(enabled);
        if enabled {
            imp.announce_local_clipboard();
        }
    }

    pub fn set_forward_unicode(&self, enabled: bool) {
        self.imp().forward_unicode.set(enabled);
    }

    pub fn set_inhibit_system_shortcuts(&self, enabled: bool) {
        self.imp().inhibit_system_shortcuts.set(enabled);
        crate::utils::set_shortcuts_inhibited(self, self.state() == RdpState::Connected && enabled);
    }

    pub fn send_key(&self, keycode: u16, pressed: bool) {
        self.imp().send_key(keycode, pressed);
    }

    pub fn send_unicode_char(&self, ch: char, pressed: bool) {
        self.imp().send_unicode_char(ch, pressed);
    }

    pub fn forward_unicode(&self) -> bool {
        self.imp().forward_unicode.get()
    }
}

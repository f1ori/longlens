/* rdp/portal_transfer.rs
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

 const PORTAL_FILETRANSFER_MIME: &str = "application/vnd.portal.filetransfer";
const REMOTE_FILE_CHUNK_SIZE: u32 = 32 * 1024;
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::sync::OnceLock;
use tracing::{info, warn};

use super::session::{RemoteClipboardFile, Session};
use super::RdpWidget;

glib::wrapper! {
    pub struct PortalFileTransferProvider(ObjectSubclass<portal_provider::PortalFileTransferProvider>)
        @extends gdk::ContentProvider;
}

impl PortalFileTransferProvider {
    pub(super) fn new(widget: &RdpWidget, session: Session, files: Vec<RemoteClipboardFile>) -> Self {
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
        .clipboard
        .register_pending_file_contents(widget, stream_id, sender);
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

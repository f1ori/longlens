/* clipboard.rs
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

use std::sync::{Arc, Mutex};

use ironrdp::cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory, ClipboardMessage, ClipboardMessageProxy};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId, OwnedFormatDataResponse,
};
use ironrdp_client::rdp::RdpInputEvent;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

/// Forwards clipboard messages from the OS clipboard backend into the RDP input channel.
#[derive(Clone, Debug)]
pub struct ClientClipboardMessageProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl ClientClipboardMessageProxy {
    pub fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self { tx }
    }
}

impl ClipboardMessageProxy for ClientClipboardMessageProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        if self.tx.send(RdpInputEvent::Clipboard(message)).is_err() {
            error!("Failed to send clipboard message, receiver is closed");
        }
    }
}

/// State shared between the GTK main thread (clipboard monitoring) and the RDP backend task.
#[derive(Debug)]
pub struct ClipboardContext {
    /// The proxy for the current active connection (None when disconnected).
    pub proxy: Option<ClientClipboardMessageProxy>,
    /// Cached UTF-16 LE text from the local clipboard (with null terminator), for serving remote requests.
    pub local_text_utf16: Option<Vec<u8>>,
    /// Text most recently received from the remote and written to local clipboard,
    /// used to avoid re-broadcasting it back to the remote.
    pub last_remote_text: Option<String>,
}

pub type SharedClipboardContext = Arc<Mutex<ClipboardContext>>;

pub fn new_shared_context() -> SharedClipboardContext {
    Arc::new(Mutex::new(ClipboardContext {
        proxy: None,
        local_text_utf16: None,
        last_remote_text: None,
    }))
}

pub struct GtkCliprdrBackendFactory {
    context: SharedClipboardContext,
    /// Channel to send text received from remote to the GTK main thread for writing to clipboard.
    gtk_tx: async_channel::Sender<String>,
}

impl GtkCliprdrBackendFactory {
    pub fn new(context: SharedClipboardContext, gtk_tx: async_channel::Sender<String>) -> Self {
        Self { context, gtk_tx }
    }
}

impl CliprdrBackendFactory for GtkCliprdrBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(GtkCliprdrBackend {
            context: self.context.clone(),
            gtk_tx: self.gtk_tx.clone(),
        })
    }
}

#[derive(Debug)]
pub struct GtkCliprdrBackend {
    context: SharedClipboardContext,
    gtk_tx: async_channel::Sender<String>,
}

impl ironrdp::core::AsAny for GtkCliprdrBackend {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl CliprdrBackend for GtkCliprdrBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        debug!("Clipboard channel ready");
        // Advertise local clipboard contents to remote if we have any cached text.
        let ctx = self.context.lock().unwrap();
        if ctx.local_text_utf16.is_some() {
            if let Some(proxy) = &ctx.proxy {
                proxy.send_clipboard_message(ClipboardMessage::SendInitiateCopy(vec![
                    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
                ]));
            }
        }
    }

    fn on_request_format_list(&mut self) {
        debug!("Remote requested our format list");
        let ctx = self.context.lock().unwrap();
        if ctx.local_text_utf16.is_some() {
            if let Some(proxy) = &ctx.proxy {
                proxy.send_clipboard_message(ClipboardMessage::SendInitiateCopy(vec![
                    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
                ]));
            }
        }
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        debug!(?capabilities, "Negotiated clipboard capabilities");
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        debug!(?available_formats, "Remote copied data");
        let has_unicode = available_formats
            .iter()
            .any(|f| f.id == ClipboardFormatId::CF_UNICODETEXT);
        if has_unicode {
            let ctx = self.context.lock().unwrap();
            if let Some(proxy) = &ctx.proxy {
                proxy.send_clipboard_message(ClipboardMessage::SendInitiatePaste(
                    ClipboardFormatId::CF_UNICODETEXT,
                ));
            }
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        debug!(?request, "Remote requested format data");
        let ctx = self.context.lock().unwrap();
        let response = if request.format == ClipboardFormatId::CF_UNICODETEXT {
            if let Some(data) = &ctx.local_text_utf16 {
                OwnedFormatDataResponse::new_data(data.clone())
            } else {
                OwnedFormatDataResponse::new_error()
            }
        } else {
            OwnedFormatDataResponse::new_error()
        };
        if let Some(proxy) = &ctx.proxy {
            proxy.send_clipboard_message(ClipboardMessage::SendFormatData(response));
        }
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            warn!("Remote clipboard data response was an error");
            return;
        }
        let data = response.data();
        // Decode UTF-16 LE, stopping at null terminator.
        let u16_chars: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        match String::from_utf16(&u16_chars) {
            Ok(text) => {
                debug!("Received {} chars from remote clipboard", text.len());
                self.context.lock().unwrap().last_remote_text = Some(text.clone());
                if let Err(e) = self.gtk_tx.try_send(text) {
                    error!("Failed to forward remote clipboard text to GTK thread: {e}");
                }
            }
            Err(e) => {
                warn!("Failed to decode remote clipboard text as UTF-16: {e}");
            }
        }
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _data_id: LockDataId) {}
    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

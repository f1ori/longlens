/* rdp/clipboard.rs
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

//! Clipboard state and GTK clipboard integration for an RDP widget.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use adw::prelude::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;
use gtk::{gdk, gio, glib};
use tracing::{info, warn};

use super::portal_transfer::PortalFileTransferProvider;
use super::session::{LocalClipboardFile, RemoteClipboardFile, Session};
use super::{RdpState, RdpWidget};

const REMOTE_FILE_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Default)]
pub(in crate::rdp) struct Clipboard {
    pending_file_contents: RefCell<HashMap<u32, async_channel::Sender<Vec<u8>>>>,
    last_remote_text: RefCell<Option<String>>,
    suppress_next_announce: Cell<bool>,
    enabled: Cell<bool>,
}

impl Clipboard {
    pub(in crate::rdp) fn enabled(&self) -> bool {
        self.enabled.get()
    }

    pub(in crate::rdp) fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }

    pub(in crate::rdp) fn clear_pending(&self) {
        self.pending_file_contents.borrow_mut().clear();
    }

    pub(in crate::rdp) fn complete_file_contents(&self, stream_id: u32, data: Vec<u8>) {
        if let Some(sender) = self.pending_file_contents.borrow_mut().remove(&stream_id) {
            let _ = sender.try_send(data);
        }
    }

    pub(in crate::rdp) fn register_pending_file_contents(
        &self,
        widget: &RdpWidget,
        stream_id: u32,
        sender: async_channel::Sender<Vec<u8>>,
    ) {
        self.pending_file_contents
            .borrow_mut()
            .insert(stream_id, sender);

        let widget = widget.downgrade();
        glib::timeout_add_local_once(REMOTE_FILE_RESPONSE_TIMEOUT, move || {
            let Some(widget) = widget.upgrade() else {
                return;
            };
            if widget
                .imp()
                .clipboard
                .pending_file_contents
                .borrow_mut()
                .remove(&stream_id)
                .is_some()
            {
                warn!(stream_id, "Timed out waiting for remote clipboard file response");
            }
        });
    }

    pub(in crate::rdp) fn set_remote_text(&self, widget: &RdpWidget, text: String) {
        if !self.enabled() {
            return;
        }
        info!(chars = text.chars().count(), "Setting local clipboard from remote text");
        *self.last_remote_text.borrow_mut() = Some(text.clone());
        widget.display().clipboard().set_text(&text);
    }

    pub(in crate::rdp) fn set_remote_file_transfer_portal(
        &self,
        widget: &RdpWidget,
        session: Session,
        files: Vec<RemoteClipboardFile>,
    ) {
        if !self.enabled() {
            return;
        }
        warn!(count = files.len(), "Setting local clipboard to remote file-transfer provider");
        let provider = PortalFileTransferProvider::new(widget, session, files);
        self.suppress_next_announce.set(true);
        if let Err(error) = widget.display().clipboard().set_content(Some(&provider)) {
            self.suppress_next_announce.set(false);
            warn!("Could not set portal file transfer clipboard content: {error}");
        }
    }

    pub(in crate::rdp) fn announce_local(
        &self,
        widget: &RdpWidget,
        session: Option<Session>,
        state: RdpState,
    ) {
        if state != RdpState::Connected || !self.enabled() {
            return;
        }
        let Some(session) = session else {
            return;
        };
        let clipboard = widget.display().clipboard();
        let widget = widget.downgrade();
        glib::spawn_future_local(async move {
            if let Some(files) = read_clipboard_files(&clipboard).await {
                session.set_clipboard_files(files);
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
            let Some(widget) = widget.upgrade() else {
                return;
            };
            let clipboard_state = &widget.imp().clipboard;
            if clipboard_state.last_remote_text.borrow().as_deref() == Some(text.as_str()) {
                clipboard_state.last_remote_text.borrow_mut().take();
                return;
            }
            session.set_clipboard_text(text);
        });
    }

    pub(in crate::rdp) fn setup(&self, widget: &RdpWidget) {
        widget.display().clipboard().connect_changed(glib::clone!(
            #[weak]
            widget,
            move |_clipboard| {
                let clipboard_state = &widget.imp().clipboard;
                if clipboard_state.suppress_next_announce.replace(false) {
                    warn!("Ignoring clipboard change caused by remote clipboard update");
                    return;
                }
                widget.announce_local_clipboard();
            }
        ));
    }
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

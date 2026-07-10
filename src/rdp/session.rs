/* rdp/session.rs
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

//! Safe ownership and worker-thread integration for the FreeRDP C adapter.

use std::ffi::{CStr, CString, c_char, c_void};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use secrecy::{ExposeSecret, SecretString};
use tracing::{info, warn};

use super::ffi;

#[derive(Debug)]
pub struct SessionConfig {
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub domain: String,
    pub password: SecretString,
    pub config_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub desktop_scale: u32,
    pub sound_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct CertificateDetails {
    pub host: String,
    pub port: u16,
    pub common_name: String,
    pub subject: String,
    pub issuer: String,
    pub fingerprint: String,
    pub host_mismatch: bool,
    pub old_subject: Option<String>,
    pub old_issuer: Option<String>,
    pub old_fingerprint: Option<String>,
}

impl CertificateDetails {
    pub fn changed(&self) -> bool {
        self.old_fingerprint.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CertificateDecision {
    Reject,
    TrustPermanently,
    TrustOnce,
}

impl CertificateDecision {
    fn code(self) -> u32 {
        match self {
            Self::Reject => 0,
            Self::TrustPermanently => 1,
            Self::TrustOnce => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalClipboardFile {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_directory: bool,
}

#[derive(Debug, Clone)]
pub struct RemoteClipboardFile {
    pub name: String,
    pub size: u64,
    pub is_directory: bool,
}

#[derive(Debug)]
pub enum SessionEvent {
    Frame {
        buffer: Vec<u8>,
        width: u32,
        height: u32,
        stride: u32,
    },
    Cursor {
        data: Vec<u8>,
        width: u32,
        height: u32,
        hotspot_x: u32,
        hotspot_y: u32,
    },
    CursorHidden,
    CursorDefault,
    ClipboardText(String),
    ClipboardRemoteTextAvailable,
    ClipboardRemoteFilesAvailable,
    ClipboardRemoteFiles(Vec<RemoteClipboardFile>),
    ClipboardRemoteFileContents { stream_id: u32, data: Vec<u8> },
    CertificateRequest {
        details: CertificateDetails,
        response: mpsc::SyncSender<CertificateDecision>,
    },
    ConnectionFailure(ConnectionError),
    Terminated(Option<String>),
}

#[derive(Debug)]
pub struct ConnectionError {
    pub code: u32,
    pub class: u32,
    pub name: String,
    pub message: String,
}

#[derive(Debug)]
enum SessionCommand {
    Key {
        scancode: u32,
        pressed: bool,
    },
    Unicode {
        code: u16,
        pressed: bool,
    },
    Mouse {
        flags: u16,
        x: u16,
        y: u16,
    },
    Resize {
        width: u32,
        height: u32,
        desktop_scale: u32,
    },
    ClipboardSetText(String),
    ClipboardSetFiles(Vec<LocalClipboardFile>),
    ClipboardRequestText,
    ClipboardRequestFiles,
    ClipboardUnlockRemoteFiles,
    ClipboardRequestFileSize {
        stream_id: u32,
        index: u32,
    },
    ClipboardRequestFileContents {
        stream_id: u32,
        index: u32,
        offset: u64,
        size: u32,
    },
    Disconnect,
}

struct CallbackContext {
    output: async_channel::Sender<SessionEvent>,
    local_files: Mutex<Vec<LocalClipboardFile>>,
    next_stream_id: AtomicU32,
}

struct NativeSession {
    raw: NonNull<ffi::LLSession>,
    _callbacks: Box<CallbackContext>,
    aborted: AtomicBool,
}

// The adapter documents abort as cross-thread safe. All other native calls are
// serialized on the worker thread.
unsafe impl Send for NativeSession {}
unsafe impl Sync for NativeSession {}

impl Drop for NativeSession {
    fn drop(&mut self) {
        unsafe { ffi::ll_session_free(self.raw.as_ptr()) };
    }
}

impl NativeSession {
    fn abort(&self) {
        self.aborted.store(true, Ordering::Release);
        unsafe { ffi::ll_session_abort(self.raw.as_ptr()) };
    }

    fn connection_error(&self) -> ConnectionError {
        unsafe {
            let code = ffi::ll_session_last_error(self.raw.as_ptr());
            ConnectionError {
                code,
                class: ffi::ll_error_class(code),
                name: c_string(ffi::ll_error_name(code)),
                message: c_string(ffi::ll_error_string(code)),
            }
        }
    }
}

#[derive(Clone)]
pub struct Session {
    native: Arc<NativeSession>,
    commands: mpsc::Sender<SessionCommand>,
}

impl Session {
    pub fn spawn(
        config: SessionConfig,
        output: async_channel::Sender<SessionEvent>,
    ) -> Option<Self> {
        let hostname = CString::new(config.hostname).ok()?;
        let username = CString::new(config.username).ok()?;
        let domain = CString::new(config.domain).ok()?;
        let password = CString::new(config.password.expose_secret()).ok()?;
        let config_path = CString::new(config.config_path.to_string_lossy().as_bytes()).ok()?;

        let mut callback_context = Box::new(CallbackContext {
            output,
            local_files: Mutex::new(Vec::new()),
            next_stream_id: AtomicU32::new(1),
        });
        let callbacks = ffi::LLSessionCallbacks {
            user_data: (&mut *callback_context as *mut CallbackContext).cast(),
            frame: Some(frame_callback),
            cursor: Some(cursor_callback),
            cursor_system: Some(cursor_system_callback),
            clipboard_offer_text: Some(clipboard_offer_text_callback),
            clipboard_text: Some(clipboard_text_callback),
            clipboard_offer_files: Some(clipboard_offer_files_callback),
            clipboard_files: Some(clipboard_files_callback),
            clipboard_file_contents_response: Some(clipboard_file_contents_response_callback),
            clipboard_file_size: Some(clipboard_file_size_callback),
            clipboard_file_contents: Some(clipboard_file_contents_callback),
            verify_certificate: Some(certificate_callback),
        };
        let native_config = ffi::LLSessionConfig {
            hostname: hostname.as_ptr(),
            port: config.port,
            username: username.as_ptr(),
            domain: domain.as_ptr(),
            password: password.as_ptr(),
            config_path: config_path.as_ptr(),
            width: config.width,
            height: config.height,
            desktop_scale: config.desktop_scale,
            sound_enabled: config.sound_enabled,
        };
        let raw = NonNull::new(unsafe { ffi::ll_session_new(&native_config, &callbacks) })?;
        let native = Arc::new(NativeSession {
            raw,
            _callbacks: callback_context,
            aborted: AtomicBool::new(false),
        });
        let (commands, receiver) = mpsc::channel();
        let worker_native = native.clone();
        std::thread::Builder::new()
            .name("longlens-freerdp".into())
            .spawn(move || run_worker(worker_native, receiver))
            .ok()?;
        Some(Self { native, commands })
    }

    pub fn send_key(&self, scancode: u32, pressed: bool) {
        let _ = self
            .commands
            .send(SessionCommand::Key { scancode, pressed });
    }

    pub fn send_unicode(&self, code: u16, pressed: bool) {
        let _ = self
            .commands
            .send(SessionCommand::Unicode { code, pressed });
    }

    pub fn send_mouse(&self, flags: u16, x: u16, y: u16) {
        let _ = self.commands.send(SessionCommand::Mouse { flags, x, y });
    }

    pub fn resize(&self, width: u32, height: u32, desktop_scale: u32) {
        let _ = self.commands.send(SessionCommand::Resize {
            width,
            height,
            desktop_scale,
        });
    }

    pub fn set_clipboard_text(&self, text: String) {
        let _ = self.commands.send(SessionCommand::ClipboardSetText(text));
    }

    pub fn set_clipboard_files(&self, files: Vec<LocalClipboardFile>) {
        let _ = self.commands.send(SessionCommand::ClipboardSetFiles(files));
    }

    pub fn request_clipboard_text(&self) {
        let _ = self.commands.send(SessionCommand::ClipboardRequestText);
    }

    pub fn request_clipboard_files(&self) {
        info!("Requesting remote clipboard file descriptor list");
        let _ = self.commands.send(SessionCommand::ClipboardRequestFiles);
    }

    pub fn unlock_remote_clipboard_files(&self) {
        info!("Unlocking remote clipboard file data");
        let _ = self.commands.send(SessionCommand::ClipboardUnlockRemoteFiles);
    }

    pub fn request_clipboard_file_size(&self, stream_id: u32, index: u32) {
        info!(stream_id, index, "Requesting remote clipboard file size");
        let _ = self.commands.send(SessionCommand::ClipboardRequestFileSize {
            stream_id,
            index,
        });
    }

    pub fn request_clipboard_file_contents(
        &self,
        stream_id: u32,
        index: u32,
        offset: u64,
        size: u32,
    ) {
        info!(stream_id, index, offset, size, "Requesting remote clipboard file content range");
        let _ = self.commands.send(SessionCommand::ClipboardRequestFileContents {
            stream_id,
            index,
            offset,
            size,
        });
    }

    pub fn next_stream_id(&self) -> u32 {
        self.native._callbacks.next_stream_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn disconnect(&self) {
        let _ = self.commands.send(SessionCommand::Disconnect);
    }

    pub fn abort(&self) {
        self.native.abort();
    }
}

fn run_worker(native: Arc<NativeSession>, commands: mpsc::Receiver<SessionCommand>) {
    let connected = unsafe { ffi::ll_session_connect(native.raw.as_ptr()) } != 0;
    if !connected {
        let event = if native.aborted.load(Ordering::Acquire) {
            SessionEvent::Terminated(None)
        } else {
            SessionEvent::ConnectionFailure(native.connection_error())
        };
        let _ = native._callbacks.output.send_blocking(event);
        return;
    }

    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                SessionCommand::Key { scancode, pressed } => unsafe {
                    ffi::ll_session_send_key(native.raw.as_ptr(), scancode, pressed.into());
                },
                SessionCommand::Unicode { code, pressed } => unsafe {
                    ffi::ll_session_send_unicode(native.raw.as_ptr(), code, pressed.into());
                },
                SessionCommand::Mouse { flags, x, y } => unsafe {
                    ffi::ll_session_send_mouse(native.raw.as_ptr(), flags, x, y);
                },
                SessionCommand::Resize {
                    width,
                    height,
                    desktop_scale,
                } => unsafe {
                    ffi::ll_session_resize(
                        native.raw.as_ptr(),
                        width,
                        height,
                        desktop_scale,
                    );
                },
                SessionCommand::ClipboardSetText(text) => {
                    if let Ok(mut files) = native._callbacks.local_files.lock() {
                        files.clear();
                    }
                    let data = encode_clipboard_text(&text);
                    unsafe {
                        ffi::ll_session_clipboard_set_text(
                            native.raw.as_ptr(),
                            data.as_ptr(),
                            data.len() as u32,
                        );
                    }
                }
                SessionCommand::ClipboardSetFiles(files) => {
                    let descriptor = encode_file_group_descriptor(&files);
                    let count = files.len() as u32;
                    if let Ok(mut stored) = native._callbacks.local_files.lock() {
                        *stored = files;
                    }
                    unsafe {
                        ffi::ll_session_clipboard_set_files(
                            native.raw.as_ptr(),
                            descriptor.as_ptr(),
                            descriptor.len() as u32,
                            count,
                        );
                    }
                }
                SessionCommand::ClipboardRequestText => unsafe {
                    ffi::ll_session_clipboard_request_text(native.raw.as_ptr());
                },
                SessionCommand::ClipboardRequestFiles => unsafe {
                    ffi::ll_session_clipboard_request_files(native.raw.as_ptr());
                },
                SessionCommand::ClipboardUnlockRemoteFiles => unsafe {
                    ffi::ll_session_clipboard_unlock_remote_files(native.raw.as_ptr());
                },
                SessionCommand::ClipboardRequestFileSize { stream_id, index } => unsafe {
                    ffi::ll_session_clipboard_request_file_size(
                        native.raw.as_ptr(),
                        stream_id,
                        index,
                    );
                },
                SessionCommand::ClipboardRequestFileContents {
                    stream_id,
                    index,
                    offset,
                    size,
                } => unsafe {
                    ffi::ll_session_clipboard_request_file_contents(
                        native.raw.as_ptr(),
                        stream_id,
                        index,
                        offset,
                        size,
                    );
                },
                SessionCommand::Disconnect => {
                    unsafe { ffi::ll_session_disconnect(native.raw.as_ptr()) };
                    let _ = native
                        ._callbacks
                        .output
                        .send_blocking(SessionEvent::Terminated(None));
                    return;
                }
            }
        }

        let result = unsafe { ffi::ll_session_poll(native.raw.as_ptr(), 10) };
        if result <= 0 {
            let detail = (result < 0).then(|| native.connection_error().message);
            let _ = native
                ._callbacks
                .output
                .send_blocking(SessionEvent::Terminated(detail));
            return;
        }
    }
}

unsafe extern "C" fn frame_callback(
    user_data: *mut c_void,
    data: *const u8,
    width: u32,
    height: u32,
    stride: u32,
) {
    if user_data.is_null() || data.is_null() {
        return;
    }
    let Some(len) = (stride as usize).checked_mul(height as usize) else {
        return;
    };
    let buffer = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let _ = context.output.send_blocking(SessionEvent::Frame {
        buffer,
        width,
        height,
        stride,
    });
}

unsafe extern "C" fn cursor_callback(
    user_data: *mut c_void,
    data: *const u8,
    width: u32,
    height: u32,
    hotspot_x: u32,
    hotspot_y: u32,
) {
    if user_data.is_null() || data.is_null() {
        return;
    }
    let Some(len) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return;
    };
    let data = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let _ = context.output.send_blocking(SessionEvent::Cursor {
        data,
        width,
        height,
        hotspot_x,
        hotspot_y,
    });
}

unsafe extern "C" fn cursor_system_callback(user_data: *mut c_void, kind: u32) {
    if user_data.is_null() {
        return;
    }
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let event = if kind == 0 {
        SessionEvent::CursorHidden
    } else {
        SessionEvent::CursorDefault
    };
    let _ = context.output.send_blocking(event);
}

unsafe extern "C" fn clipboard_offer_text_callback(user_data: *mut c_void) {
    if user_data.is_null() {
        return;
    }
    info!("Remote clipboard offered text");
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let _ = context
        .output
        .send_blocking(SessionEvent::ClipboardRemoteTextAvailable);
}

unsafe extern "C" fn clipboard_text_callback(user_data: *mut c_void, data: *const u8, size: u32) {
    if user_data.is_null() || data.is_null() {
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, size as usize) };
    if let Some(text) = decode_clipboard_text(bytes) {
        info!(chars = text.chars().count(), "Received remote clipboard text");
        let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
        let _ = context.output.send_blocking(SessionEvent::ClipboardText(text));
    } else {
        warn!(size, "Could not decode remote clipboard text");
    }
}

unsafe extern "C" fn clipboard_offer_files_callback(user_data: *mut c_void) {
    if user_data.is_null() {
        return;
    }
    info!("Remote clipboard offered files");
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let _ = context
        .output
        .send_blocking(SessionEvent::ClipboardRemoteFilesAvailable);
}

unsafe extern "C" fn clipboard_files_callback(user_data: *mut c_void, data: *const u8, size: u32) {
    if user_data.is_null() || data.is_null() {
        return;
    }
    info!(size, "Received remote clipboard file descriptor data");
    let bytes = unsafe { std::slice::from_raw_parts(data, size as usize) };
    if let Some(files) = decode_file_group_descriptor(bytes) {
        info!(count = files.len(), ?files, "Decoded remote clipboard file descriptors");
        let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
        let _ = context
            .output
            .send_blocking(SessionEvent::ClipboardRemoteFiles(files));
    } else {
        let hex = bytes
            .iter()
            .take(64)
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        warn!(size, %hex, "Could not decode remote clipboard file descriptors");
    }
}

unsafe extern "C" fn clipboard_file_contents_response_callback(
    user_data: *mut c_void,
    stream_id: u32,
    data: *const u8,
    size: u32,
) {
    if user_data.is_null() || data.is_null() {
        return;
    }
    info!(stream_id, size, "Received remote clipboard file content response");
    let data = unsafe { std::slice::from_raw_parts(data, size as usize) }.to_vec();
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let _ = context
        .output
        .send_blocking(SessionEvent::ClipboardRemoteFileContents { stream_id, data });
}

unsafe extern "C" fn clipboard_file_size_callback(user_data: *mut c_void, index: u32) -> u64 {
    if user_data.is_null() {
        return 0;
    }
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    context
        .local_files
        .lock()
        .ok()
        .and_then(|files| files.get(index as usize).map(|file| file.size))
        .unwrap_or(0)
}

unsafe extern "C" fn clipboard_file_contents_callback(
    user_data: *mut c_void,
    index: u32,
    offset: u64,
    data: *mut u8,
    size: u32,
) -> u32 {
    if user_data.is_null() || data.is_null() || size == 0 {
        return 0;
    }
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let Some(path) = context
        .local_files
        .lock()
        .ok()
        .and_then(|files| files.get(index as usize).map(|file| file.path.clone()))
    else {
        return 0;
    };
    let Ok(mut file) = File::open(path) else {
        return 0;
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return 0;
    }
    let buffer = unsafe { std::slice::from_raw_parts_mut(data, size as usize) };
    file.read(buffer).unwrap_or_default() as u32
}

fn encode_clipboard_text(text: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity((text.len() + 1) * 2);
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data
}

fn encode_file_group_descriptor(files: &[LocalClipboardFile]) -> Vec<u8> {
    const FILEDESCRIPTORW_SIZE: usize = 592;
    const FD_ATTRIBUTES: u32 = 0x0000_0004;
    const FD_FILESIZE: u32 = 0x0000_0040;
    const FD_UNICODE: u32 = 0x8000_0000;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;

    let mut data = Vec::with_capacity(4 + files.len() * FILEDESCRIPTORW_SIZE);
    data.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for file in files {
        let start = data.len();
        data.resize(start + FILEDESCRIPTORW_SIZE, 0);
        data[start..start + 4]
            .copy_from_slice(&(FD_UNICODE | FD_ATTRIBUTES | FD_FILESIZE).to_le_bytes());
        let attributes = if file.is_directory {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        data[start + 36..start + 40].copy_from_slice(&attributes.to_le_bytes());
        data[start + 64..start + 68].copy_from_slice(&((file.size >> 32) as u32).to_le_bytes());
        data[start + 68..start + 72].copy_from_slice(&(file.size as u32).to_le_bytes());
        for (i, unit) in file.name.encode_utf16().take(259).enumerate() {
            let offset = start + 72 + i * 2;
            data[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    data
}

fn decode_file_group_descriptor(data: &[u8]) -> Option<Vec<RemoteClipboardFile>> {
    const FILEDESCRIPTORW_SIZE: usize = 592;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    if data.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if data.len() < 4 + count.checked_mul(FILEDESCRIPTORW_SIZE)? {
        return None;
    }
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let start = 4 + i * FILEDESCRIPTORW_SIZE;
        let attributes = u32::from_le_bytes(data[start + 36..start + 40].try_into().ok()?);
        let size_high = u32::from_le_bytes(data[start + 64..start + 68].try_into().ok()?);
        let size_low = u32::from_le_bytes(data[start + 68..start + 72].try_into().ok()?);
        let mut units = Vec::new();
        for chunk in data[start + 72..start + 72 + 520].chunks_exact(2) {
            let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        let name = sanitize_remote_file_name(&String::from_utf16(&units).ok()?)?;
        files.push(RemoteClipboardFile {
            name,
            size: ((size_high as u64) << 32) | size_low as u64,
            is_directory: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        });
    }
    Some(files)
}

fn sanitize_remote_file_name(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let path = std::path::Path::new(&name);
    let file_name = path.file_name()?.to_str()?.to_owned();
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        None
    } else {
        Some(file_name)
    }
}

fn decode_clipboard_text(data: &[u8]) -> Option<String> {
    let mut units = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    String::from_utf16(&units).ok()
}

unsafe extern "C" fn certificate_callback(
    user_data: *mut c_void,
    host: *const c_char,
    port: u16,
    common_name: *const c_char,
    subject: *const c_char,
    issuer: *const c_char,
    fingerprint: *const c_char,
    flags: u32,
    old_subject: *const c_char,
    old_issuer: *const c_char,
    old_fingerprint: *const c_char,
) -> u32 {
    if user_data.is_null() {
        return CertificateDecision::Reject.code();
    }
    let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let details = CertificateDetails {
        host: unsafe { c_string(host) },
        port,
        common_name: unsafe { c_string(common_name) },
        subject: unsafe { c_string(subject) },
        issuer: unsafe { c_string(issuer) },
        fingerprint: unsafe { c_string(fingerprint) },
        host_mismatch: flags & 0x80 != 0,
        old_subject: unsafe { optional_c_string(old_subject) },
        old_issuer: unsafe { optional_c_string(old_issuer) },
        old_fingerprint: unsafe { optional_c_string(old_fingerprint) },
    };
    let (response, receiver) = mpsc::sync_channel(1);
    if context
        .output
        .send_blocking(SessionEvent::CertificateRequest { details, response })
        .is_err()
    {
        return CertificateDecision::Reject.code();
    }
    receiver
        .recv()
        .unwrap_or(CertificateDecision::Reject)
        .code()
}

unsafe fn c_string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}

unsafe fn optional_c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(unsafe { c_string(value) })
    }
}

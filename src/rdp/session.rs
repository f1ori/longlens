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
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use secrecy::{ExposeSecret, SecretString};

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
    ClipboardRequestText,
    Disconnect,
}

struct CallbackContext {
    output: async_channel::Sender<SessionEvent>,
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

        let mut callback_context = Box::new(CallbackContext { output });
        let callbacks = ffi::LLSessionCallbacks {
            user_data: (&mut *callback_context as *mut CallbackContext).cast(),
            frame: Some(frame_callback),
            cursor: Some(cursor_callback),
            cursor_system: Some(cursor_system_callback),
            clipboard_offer_text: Some(clipboard_offer_text_callback),
            clipboard_text: Some(clipboard_text_callback),
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

    pub fn request_clipboard_text(&self) {
        let _ = self.commands.send(SessionCommand::ClipboardRequestText);
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
                    let data = encode_clipboard_text(&text);
                    unsafe {
                        ffi::ll_session_clipboard_set_text(
                            native.raw.as_ptr(),
                            data.as_ptr(),
                            data.len() as u32,
                        );
                    }
                }
                SessionCommand::ClipboardRequestText => unsafe {
                    ffi::ll_session_clipboard_request_text(native.raw.as_ptr());
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
        let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
        let _ = context.output.send_blocking(SessionEvent::ClipboardText(text));
    }
}

fn encode_clipboard_text(text: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity((text.len() + 1) * 2);
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data
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

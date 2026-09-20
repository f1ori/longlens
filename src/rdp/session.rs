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

use super::clipboard_codec::{
    decode_clipboard_text, decode_file_group_descriptor, encode_clipboard_text,
    encode_file_group_descriptor,
};
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
    ClipboardRemoteFileContents {
        stream_id: u32,
        data: Vec<u8>,
    },
    CertificateRequest {
        details: CertificateDetails,
        response: mpsc::SyncSender<CertificateDecision>,
    },
    ConnectionFailure(ConnectionError),
    /// The transport died. The worker keeps the session alive and starts
    /// trying to restore it; `can_restore_session` tells whether the server
    /// issued an auto-reconnect cookie, i.e. whether a successful attempt
    /// reattaches to the existing session rather than opening a new one.
    Interrupted {
        detail: String,
        can_restore_session: bool,
    },
    /// Waiting before the next attempt. `attempt` is the number of attempts
    /// made so far.
    ReconnectCountdown {
        attempt: u32,
        seconds_left: u32,
    },
    /// `attempt` is in progress right now.
    ReconnectAttempt {
        attempt: u32,
    },
    /// The session is live again.
    Reconnected,
    Terminated(TerminationReason),
}

#[derive(Debug)]
pub enum TerminationReason {
    /// We asked for it: the disconnect button, an abort or the watchdog.
    Local,
    /// The server ended the session: logoff, kick, idle timeout.
    Remote,
    /// Gave up after an unrecoverable failure while trying to reconnect.
    Lost(ConnectionError),
}

#[derive(Debug)]
pub struct ConnectionError {
    pub code: u32,
    pub class: u32,
    pub name: String,
    pub message: String,
}

impl ConnectionError {
    /// Credential and authorisation failures will not fix themselves, so they
    /// end a reconnect cycle instead of prolonging it. The classes come from
    /// `ll_error_class` in the C adapter.
    pub fn is_recoverable(&self) -> bool {
        !matches!(self.class, 1 | 2)
    }
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
    /// Cut a reconnect countdown short.
    ReconnectNow,
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
    /// Set as soon as a shutdown is requested. `ll_session_abort` makes
    /// `freerdp_shall_disconnect_context()` true, so without this flag a
    /// local abort is indistinguishable from a remote logoff.
    stopping: AtomicBool,
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
        self.stopping.store(true, Ordering::Release);
        unsafe { ffi::ll_session_abort(self.raw.as_ptr()) };
    }

    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    /// True when the server issued an auto-reconnect cookie, so a successful
    /// reconnect restores the existing session.
    fn can_restore_session(&self) -> bool {
        unsafe { ffi::ll_session_can_restore_session(self.raw.as_ptr()) != 0 }
    }

    /// Non-zero when the server ended the session deliberately.
    fn error_info(&self) -> u32 {
        unsafe { ffi::ll_session_error_info(self.raw.as_ptr()) }
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
        };
        let raw = NonNull::new(unsafe { ffi::ll_session_new(&native_config, &callbacks) })?;
        let native = Arc::new(NativeSession {
            raw,
            _callbacks: callback_context,
            aborted: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
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
        let _ = self
            .commands
            .send(SessionCommand::ClipboardUnlockRemoteFiles);
    }

    pub fn request_clipboard_file_size(&self, stream_id: u32, index: u32) {
        info!(stream_id, index, "Requesting remote clipboard file size");
        let _ = self
            .commands
            .send(SessionCommand::ClipboardRequestFileSize { stream_id, index });
    }

    pub fn request_clipboard_file_contents(
        &self,
        stream_id: u32,
        index: u32,
        offset: u64,
        size: u32,
    ) {
        info!(
            stream_id,
            index, offset, size, "Requesting remote clipboard file content range"
        );
        let _ = self
            .commands
            .send(SessionCommand::ClipboardRequestFileContents {
                stream_id,
                index,
                offset,
                size,
            });
    }

    pub fn next_stream_id(&self) -> u32 {
        self.native
            ._callbacks
            .next_stream_id
            .fetch_add(1, Ordering::Relaxed)
    }

    pub fn disconnect(&self) {
        self.native.stopping.store(true, Ordering::Release);
        let _ = self.commands.send(SessionCommand::Disconnect);
    }

    pub fn abort(&self) {
        self.native.abort();
    }

    pub fn reconnect_now(&self) {
        let _ = self.commands.send(SessionCommand::ReconnectNow);
    }
}

/// How the poll loop gave up control.
enum Flow {
    /// The session is over for good.
    Ended(TerminationReason),
    /// The transport died; the session may be restorable.
    Interrupted(ConnectionError),
}

/// Delays between reconnect attempts, in seconds. The last entry repeats for
/// every further attempt, so the cycle continues until the user ends it.
const RETRY_DELAYS: [u32; 6] = [2, 5, 10, 20, 30, 60];

fn retry_delay(attempt: u32) -> u32 {
    RETRY_DELAYS[(attempt as usize).min(RETRY_DELAYS.len() - 1)]
}

fn run_worker(native: Arc<NativeSession>, commands: mpsc::Receiver<SessionCommand>) {
    let connected = unsafe { ffi::ll_session_connect(native.raw.as_ptr()) } != 0;
    if !connected {
        let event = if native.aborted.load(Ordering::Acquire) {
            SessionEvent::Terminated(TerminationReason::Local)
        } else {
            SessionEvent::ConnectionFailure(native.connection_error())
        };
        let _ = native._callbacks.output.send_blocking(event);
        return;
    }

    loop {
        match run_session(&native, &commands) {
            Flow::Ended(reason) => {
                let _ = native
                    ._callbacks
                    .output
                    .send_blocking(SessionEvent::Terminated(reason));
                return;
            }
            Flow::Interrupted(error) => {
                // reconnect_loop reports its own terminal event when it gives up.
                if !reconnect_loop(&native, &commands, error) {
                    return;
                }
            }
        }
    }
}

/// Pumps commands and FreeRDP events until the session ends or the transport
/// dies.
fn run_session(native: &Arc<NativeSession>, commands: &mpsc::Receiver<SessionCommand>) -> Flow {
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
                    ffi::ll_session_resize(native.raw.as_ptr(), width, height, desktop_scale);
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
                    return Flow::Ended(TerminationReason::Local);
                }
                // Only meaningful while a reconnect countdown is running.
                SessionCommand::ReconnectNow => {}
            }
        }

        let result = unsafe { ffi::ll_session_poll(native.raw.as_ptr(), 10) };
        if result <= 0 {
            if native.stopping() {
                return Flow::Ended(TerminationReason::Local);
            }
            // 0 means freerdp_shall_disconnect_context(): the server ended the
            // session. A negative result is a transport failure.
            if result == 0 {
                return Flow::Ended(TerminationReason::Remote);
            }
            return Flow::Interrupted(native.connection_error());
        }
    }
}

/// Tries to restore an interrupted session, reporting progress as it goes.
///
/// Returns `true` when the session is live again, `false` after having sent a
/// terminal event.
fn reconnect_loop(
    native: &Arc<NativeSession>,
    commands: &mpsc::Receiver<SessionCommand>,
    error: ConnectionError,
) -> bool {
    let send = |event| {
        let _ = native._callbacks.output.send_blocking(event);
    };

    if native.error_info() != 0 {
        // The server told us why it ended the session, so it is not coming back.
        send(SessionEvent::Terminated(TerminationReason::Remote));
        return false;
    }
    if !error.is_recoverable() {
        send(SessionEvent::Terminated(TerminationReason::Lost(error)));
        return false;
    }

    let can_restore_session = native.can_restore_session();
    warn!(
        detail = %error.message,
        can_restore_session,
        "RDP session interrupted; trying to reconnect"
    );
    send(SessionEvent::Interrupted {
        detail: error.message,
        can_restore_session,
    });

    let mut attempt = 0;
    loop {
        if !wait_for_retry(native, commands, attempt, &send) {
            return false;
        }
        // freerdp_connect() resets the abort event when it starts, so an abort
        // that lands just before an attempt would be swallowed and the user
        // would wait out a full DNS/TCP timeout. Check once more here.
        if native.stopping() {
            send(SessionEvent::Terminated(TerminationReason::Local));
            return false;
        }

        attempt += 1;
        send(SessionEvent::ReconnectAttempt { attempt });
        match unsafe { ffi::ll_session_reconnect(native.raw.as_ptr()) } {
            ffi::LL_RECONNECT_OK => {
                info!(attempt, "Reconnected");
                send(SessionEvent::Reconnected);
                return true;
            }
            ffi::LL_RECONNECT_REFUSED => {
                send(SessionEvent::Terminated(TerminationReason::Remote));
                return false;
            }
            status => {
                if status != ffi::LL_RECONNECT_FAILED {
                    warn!(status, "Unexpected reconnect result");
                }
                if native.stopping() {
                    send(SessionEvent::Terminated(TerminationReason::Local));
                    return false;
                }
                let error = native.connection_error();
                if !error.is_recoverable() {
                    send(SessionEvent::Terminated(TerminationReason::Lost(error)));
                    return false;
                }
                warn!(attempt, detail = %error.message, "Reconnect attempt failed");
            }
        }
    }
}

/// Waits out the backoff delay before `attempt + 1`, counting down once a
/// second. Returns `false` when the cycle was ended, having sent the terminal
/// event itself.
fn wait_for_retry(
    native: &Arc<NativeSession>,
    commands: &mpsc::Receiver<SessionCommand>,
    attempt: u32,
    send: &impl Fn(SessionEvent),
) -> bool {
    const TICK: std::time::Duration = std::time::Duration::from_millis(100);

    let delay = retry_delay(attempt);
    let mut seconds_left = delay;
    send(SessionEvent::ReconnectCountdown {
        attempt,
        seconds_left,
    });

    let start = std::time::Instant::now();
    // Never sleep out the whole delay in one go: the user must be able to end
    // the session or ask for an immediate retry at any point.
    while seconds_left > 0 {
        loop {
            match commands.try_recv() {
                Ok(SessionCommand::Disconnect) => {
                    send(SessionEvent::Terminated(TerminationReason::Local));
                    return false;
                }
                // Skip the rest of the countdown.
                Ok(SessionCommand::ReconnectNow) => return true,
                // Input and clipboard traffic is pointless while offline.
                Ok(_) => continue,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    send(SessionEvent::Terminated(TerminationReason::Local));
                    return false;
                }
            }
        }
        if native.stopping() {
            send(SessionEvent::Terminated(TerminationReason::Local));
            return false;
        }

        std::thread::sleep(TICK);

        let elapsed = start.elapsed().as_secs() as u32;
        let remaining = delay.saturating_sub(elapsed);
        if remaining != seconds_left {
            seconds_left = remaining;
            send(SessionEvent::ReconnectCountdown {
                attempt,
                seconds_left,
            });
        }
    }
    true
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
        info!(
            chars = text.chars().count(),
            "Received remote clipboard text"
        );
        let context = unsafe { &*(user_data.cast::<CallbackContext>()) };
        let _ = context
            .output
            .send_blocking(SessionEvent::ClipboardText(text));
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
        info!(
            count = files.len(),
            ?files,
            "Decoded remote clipboard file descriptors"
        );
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
    info!(
        stream_id,
        size, "Received remote clipboard file content response"
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_backs_off_and_then_repeats() {
        let delays: Vec<u32> = (0..8).map(retry_delay).collect();
        assert_eq!(delays, [2, 5, 10, 20, 30, 60, 60, 60]);
    }

    fn error_with_class(class: u32) -> ConnectionError {
        ConnectionError {
            code: 0,
            class,
            name: String::new(),
            message: String::new(),
        }
    }

    #[test]
    fn credential_errors_end_the_reconnect_cycle() {
        assert!(!error_with_class(1).is_recoverable());
        assert!(!error_with_class(2).is_recoverable());
        assert!(error_with_class(0).is_recoverable());
    }
}

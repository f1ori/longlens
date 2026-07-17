/* rdp/ffi.rs
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

use std::ffi::{c_char, c_void};

#[repr(C)]
pub struct LLSession {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LLSessionConfig {
    pub hostname: *const c_char,
    pub port: u16,
    pub username: *const c_char,
    pub domain: *const c_char,
    pub password: *const c_char,
    pub config_path: *const c_char,
    pub width: u32,
    pub height: u32,
    pub desktop_scale: u32,
}

#[repr(C)]
pub struct LLSessionCallbacks {
    pub user_data: *mut c_void,
    pub frame: Option<unsafe extern "C" fn(*mut c_void, *const u8, u32, u32, u32)>,
    pub cursor: Option<unsafe extern "C" fn(*mut c_void, *const u8, u32, u32, u32, u32)>,
    pub cursor_system: Option<unsafe extern "C" fn(*mut c_void, u32)>,
    pub clipboard_offer_text: Option<unsafe extern "C" fn(*mut c_void)>,
    pub clipboard_text: Option<unsafe extern "C" fn(*mut c_void, *const u8, u32)>,
    pub clipboard_offer_files: Option<unsafe extern "C" fn(*mut c_void)>,
    pub clipboard_files: Option<unsafe extern "C" fn(*mut c_void, *const u8, u32)>,
    pub clipboard_file_contents_response:
        Option<unsafe extern "C" fn(*mut c_void, u32, *const u8, u32)>,
    pub clipboard_file_size: Option<unsafe extern "C" fn(*mut c_void, u32) -> u64>,
    pub clipboard_file_contents:
        Option<unsafe extern "C" fn(*mut c_void, u32, u64, *mut u8, u32) -> u32>,
    pub verify_certificate: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            u16,
            *const c_char,
            *const c_char,
            *const c_char,
            *const c_char,
            u32,
            *const c_char,
            *const c_char,
            *const c_char,
        ) -> u32,
    >,
}

#[repr(C)]
#[derive(Default)]
pub struct LLRdpFile {
    pub hostname: *mut c_char,
    pub username: *mut c_char,
    pub domain: *mut c_char,
    pub port: u16,
}

unsafe extern "C" {
    pub fn ll_session_new(
        config: *const LLSessionConfig,
        callbacks: *const LLSessionCallbacks,
    ) -> *mut LLSession;
    pub fn ll_session_free(session: *mut LLSession);
    pub fn ll_session_connect(session: *mut LLSession) -> i32;
    pub fn ll_session_poll(session: *mut LLSession, timeout_ms: u32) -> i32;
    pub fn ll_session_disconnect(session: *mut LLSession);
    pub fn ll_session_abort(session: *mut LLSession);
    pub fn ll_session_last_error(session: *const LLSession) -> u32;
    pub fn ll_error_name(code: u32) -> *const c_char;
    pub fn ll_error_string(code: u32) -> *const c_char;
    pub fn ll_error_class(code: u32) -> u32;
    pub fn ll_session_send_key(session: *mut LLSession, scancode: u32, pressed: i32) -> i32;
    pub fn ll_session_send_unicode(session: *mut LLSession, code: u16, pressed: i32) -> i32;
    pub fn ll_session_send_mouse(
        session: *mut LLSession,
        flags: u16,
        x: u16,
        y: u16,
    ) -> i32;
    pub fn ll_session_resize(
        session: *mut LLSession,
        width: u32,
        height: u32,
        desktop_scale: u32,
    ) -> i32;
    pub fn ll_session_clipboard_set_text(
        session: *mut LLSession,
        data: *const u8,
        size: u32,
    ) -> i32;
    pub fn ll_session_clipboard_set_files(
        session: *mut LLSession,
        descriptor: *const u8,
        size: u32,
        count: u32,
    ) -> i32;
    pub fn ll_session_clipboard_request_text(session: *mut LLSession) -> i32;
    pub fn ll_session_clipboard_request_files(session: *mut LLSession) -> i32;
    pub fn ll_session_clipboard_unlock_remote_files(session: *mut LLSession) -> i32;
    pub fn ll_session_clipboard_request_file_size(
        session: *mut LLSession,
        stream_id: u32,
        index: u32,
    ) -> i32;
    pub fn ll_session_clipboard_request_file_contents(
        session: *mut LLSession,
        stream_id: u32,
        index: u32,
        offset: u64,
        size: u32,
    ) -> i32;

    pub fn ll_rdp_file_parse(path: *const c_char, result: *mut LLRdpFile) -> i32;
    pub fn ll_rdp_file_clear(result: *mut LLRdpFile);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    #[test]
    fn session_context_can_be_created_and_freed() {
        let host = CString::new("localhost").unwrap();
        let empty = CString::new("").unwrap();
        let config_path = CString::new(std::env::temp_dir().to_string_lossy().as_bytes()).unwrap();
        let config = LLSessionConfig {
            hostname: host.as_ptr(),
            port: 3389,
            username: empty.as_ptr(),
            domain: empty.as_ptr(),
            password: empty.as_ptr(),
            config_path: config_path.as_ptr(),
            width: 1280,
            height: 800,
            desktop_scale: 100,
        };
        let callbacks = LLSessionCallbacks {
            user_data: ptr::null_mut(),
            frame: None,
            cursor: None,
            cursor_system: None,
            clipboard_offer_text: None,
            clipboard_text: None,
            clipboard_offer_files: None,
            clipboard_files: None,
            clipboard_file_contents_response: None,
            clipboard_file_size: None,
            clipboard_file_contents: None,
            verify_certificate: None,
        };
        let session = unsafe { ll_session_new(&config, &callbacks) };
        assert!(!session.is_null());
        unsafe { ll_session_free(session) };
    }
}

/* rdp_file.rs
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

//! Parsing of Windows `.rdp` files with FreeRDP.

use std::ffi::{CStr, CString};
use std::path::Path;

use tracing::warn;

use crate::rdp::ffi;

pub struct RdpConnection {
    pub name: String,
    pub hostname: String,
    pub username: String,
}

pub fn parse_file(path: &Path) -> Option<RdpConnection> {
    let path_string = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut result = ffi::LLRdpFile::default();
    let parsed = unsafe { ffi::ll_rdp_file_parse(path_string.as_ptr(), &mut result) } != 0;
    if !parsed {
        warn!("Could not parse .rdp file {}", path.display());
        return None;
    }

    let hostname = unsafe { CStr::from_ptr(result.hostname) }
        .to_string_lossy()
        .into_owned();
    let user = unsafe { CStr::from_ptr(result.username) }
        .to_string_lossy()
        .into_owned();
    let domain = unsafe { CStr::from_ptr(result.domain) }
        .to_string_lossy()
        .into_owned();
    let port = result.port;
    unsafe { ffi::ll_rdp_file_clear(&mut result) };

    let hostname = if port != 0 && port != 3389 && !hostname.contains(':') {
        format!("{hostname}:{port}")
    } else {
        hostname
    };
    let username = if domain.is_empty() {
        user
    } else {
        format!("{domain}\\{user}")
    };
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| hostname.clone());

    Some(RdpConnection {
        name,
        hostname,
        username,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn parse_str(file_name: &str, contents: &str) -> Option<RdpConnection> {
        let path = std::env::temp_dir().join(file_name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        let connection = parse_file(&path);
        std::fs::remove_file(path).ok();
        connection
    }

    #[test]
    fn parses_address_port_and_domain() {
        let connection = parse_str(
            "longlens-test-full.rdp",
            "full address:s:server.example.com\nserver port:i:3390\nusername:s:alice\ndomain:s:CORP\n",
        )
        .unwrap();
        assert_eq!(connection.name, "longlens-test-full");
        assert_eq!(connection.hostname, "server.example.com:3390");
        assert_eq!(connection.username, "CORP\\alice");
    }

    #[test]
    fn default_port_is_not_appended() {
        let connection = parse_str(
            "longlens-test-default.rdp",
            "full address:s:host\nserver port:i:3389\nusername:s:bob\n",
        )
        .unwrap();
        assert_eq!(connection.hostname, "host");
        assert_eq!(connection.username, "bob");
    }

    #[test]
    fn port_in_address_is_preserved() {
        let connection = parse_str(
            "longlens-test-inline.rdp",
            "full address:s:host:4000\nserver port:i:3390\n",
        )
        .unwrap();
        assert_eq!(connection.hostname, "host:4000");
    }

    #[test]
    fn missing_address_returns_none() {
        assert!(parse_str("longlens-test-empty.rdp", "username:s:carol\n").is_none());
    }
}

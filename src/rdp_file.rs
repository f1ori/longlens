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

//! Parsing of Windows `.rdp` connection files into the fields a
//! [`DestinationData`](crate::model::destination_object::DestinationData)
//! understands, using the `ironrdp-rdpfile` crate.

use std::path::Path;

use tracing::warn;

/// The connection details extracted from a `.rdp` file, ready to pre-fill the
/// Add Destination dialog.
pub struct RdpConnection {
    pub name: String,
    pub hostname: String,
    pub username: String,
}

/// Parse the `.rdp` file at `path`. Returns `None` if the file can't be read or
/// has no usable `full address`. Malformed lines are logged but not fatal.
pub fn parse_file(path: &Path) -> Option<RdpConnection> {
    let input = match std::fs::read_to_string(path) {
        Ok(input) => input,
        Err(e) => {
            warn!("Could not read .rdp file {}: {}", path.display(), e);
            return None;
        }
    };

    let result = ironrdp_rdpfile::parse(&input);
    for error in &result.errors {
        warn!("Skipped entry in {}: {}", path.display(), error);
    }
    let properties = result.properties;

    let address = properties.get::<&str>("full address")?.trim();
    if address.is_empty() {
        return None;
    }

    // The model stores `host:port`; only append the port if the address doesn't
    // already carry one and it differs from the RDP default.
    let hostname = match properties.get::<u16>("server port") {
        Some(port) if port != 3389 && !address.contains(':') => format!("{address}:{port}"),
        _ => address.to_owned(),
    };

    // Recombine domain + username into the `DOMAIN\user` form the connection
    // config later splits apart again.
    let user = properties.get::<&str>("username").unwrap_or("").trim();
    let domain = properties.get::<&str>("domain").unwrap_or("").trim();
    let username = if domain.is_empty() {
        user.to_owned()
    } else {
        format!("{domain}\\{user}")
    };

    // `.rdp` files have no friendly name; fall back to the file stem, then host.
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_owned())
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
        let dir = std::env::temp_dir();
        let path = dir.join(file_name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        let conn = parse_file(&path);
        std::fs::remove_file(&path).ok();
        conn
    }

    #[test]
    fn parses_address_port_and_domain() {
        let conn = parse_str(
            "longlens-test-full.rdp",
            "full address:s:server.example.com\nserver port:i:3390\nusername:s:alice\ndomain:s:CORP\n",
        )
        .unwrap();
        assert_eq!(conn.name, "longlens-test-full");
        assert_eq!(conn.hostname, "server.example.com:3390");
        assert_eq!(conn.username, "CORP\\alice");
    }

    #[test]
    fn default_port_is_not_appended() {
        let conn = parse_str(
            "longlens-test-default.rdp",
            "full address:s:host\nserver port:i:3389\nusername:s:bob\n",
        )
        .unwrap();
        assert_eq!(conn.hostname, "host");
        assert_eq!(conn.username, "bob");
    }

    #[test]
    fn port_in_address_is_preserved() {
        let conn = parse_str(
            "longlens-test-inline.rdp",
            "full address:s:host:4000\nserver port:i:3390\n",
        )
        .unwrap();
        assert_eq!(conn.hostname, "host:4000");
        assert_eq!(conn.username, "");
    }

    #[test]
    fn missing_address_returns_none() {
        assert!(parse_str("longlens-test-empty.rdp", "username:s:carol\n").is_none());
    }
}

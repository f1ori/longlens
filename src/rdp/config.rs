/* rdp/config.rs
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

//! Construction of the FreeRDP session configuration.

use secrecy::SecretString;

use super::session::SessionConfig;

/// Splits a `DOMAIN\user` login into its domain and username parts.
pub fn split_domain(username: &str) -> (String, String) {
    match username.split_once('\\') {
        Some((domain, user)) => (domain.to_owned(), user.to_owned()),
        None => (String::new(), username.to_owned()),
    }
}

pub fn parse_hostname_port(input: &str) -> (String, u16) {
    let mut parts = input.splitn(2, ':');
    let host = parts.next().unwrap_or("").to_string();
    let port = parts
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3389);
    (host, port)
}

pub fn build_config(
    hostname: String,
    port: u16,
    username: String,
    password: SecretString,
    width: u16,
    height: u16,
    desktop_scale: u32,
    sound_enabled: bool,
) -> SessionConfig {
    let (domain, username) = split_domain(&username);
    let config_path = gtk::glib::user_config_dir()
        .join("longlens")
        .join("freerdp");
    if let Err(error) = std::fs::create_dir_all(&config_path) {
        tracing::warn!(
            "Could not create FreeRDP configuration directory {}: {error}",
            config_path.display()
        );
    }

    SessionConfig {
        hostname,
        port,
        username,
        domain,
        password,
        config_path,
        width: width.into(),
        height: height.into(),
        desktop_scale,
        sound_enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_domain_with_backslash() {
        assert_eq!(
            split_domain("DOMAIN\\user"),
            ("DOMAIN".into(), "user".into())
        );
    }

    #[test]
    fn split_domain_bare_username() {
        assert_eq!(split_domain("user"), (String::new(), "user".into()));
    }

    #[test]
    fn split_domain_only_first_backslash() {
        assert_eq!(
            split_domain("DOMAIN\\sub\\user"),
            ("DOMAIN".into(), "sub\\user".into())
        );
    }

    #[test]
    fn parse_hostname_port_default() {
        assert_eq!(parse_hostname_port("server"), ("server".into(), 3389));
    }

    #[test]
    fn parse_hostname_port_custom() {
        assert_eq!(parse_hostname_port("server:3390"), ("server".into(), 3390));
    }
}

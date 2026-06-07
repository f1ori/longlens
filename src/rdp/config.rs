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

//! Pure construction of the IronRDP client [`Config`] from connection parameters.

use ironrdp::connector::{self, Credentials};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_client::config::{ClipboardType, Config, Destination};
use secrecy::{ExposeSecret, SecretString};
use tracing::warn;

/// Splits a `DOMAIN\user` style login into its domain and username parts.
/// A bare username (no backslash) yields `(None, username)`.
pub fn split_domain(username: &str) -> (Option<String>, String) {
    match username.split_once('\\') {
        Some((d, u)) => (Some(d.to_owned()), u.to_owned()),
        None => (None, username.to_owned()),
    }
}

/// Maps the host platform to the RDP major platform type.
fn major_platform_type() -> MajorPlatformType {
    match whoami::platform() {
        whoami::Platform::Windows => MajorPlatformType::WINDOWS,
        whoami::Platform::Linux => MajorPlatformType::UNIX,
        whoami::Platform::MacOS => MajorPlatformType::MACINTOSH,
        whoami::Platform::Ios => MajorPlatformType::IOS,
        whoami::Platform::Android => MajorPlatformType::ANDROID,
        _ => MajorPlatformType::UNSPECIFIED,
    }
}

/// Builds the full IronRDP [`Config`] for a connection.
///
/// `phys_width`/`phys_height` are physical (scaled) pixels and `scale_factor`
/// is the desktop scale in percent (e.g. `100` or `200`). Returns `None` if the
/// codec capabilities could not be built.
pub fn build_config(
    hostname: String,
    port: u16,
    username: String,
    password: SecretString,
    phys_width: u16,
    phys_height: u16,
    scale_factor: u32,
) -> Option<Config> {
    let (domain, username) = split_domain(&username);

    let codecs: Vec<&str> = vec![];
    let codecs = match client_codecs_capabilities(&codecs) {
        Ok(c) => c,
        Err(e) => {
            warn!("Could not build codec capabilities: {}", e);
            return None;
        }
    };

    let connector_config = connector::Config {
        credentials: Credentials::UsernamePassword {
            username,
            password: password.expose_secret().clone(),
        },
        domain,
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize { width: phys_width, height: phys_height },
        desktop_scale_factor: scale_factor,
        bitmap: Some(connector::BitmapConfig {
            color_depth: 32,
            lossy_compression: true,
            codecs,
        }),
        client_build: 42,
        client_name: String::from("longlens"),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        compression_type: None,
        multitransport_flags: None,
        platform: major_platform_type(),
        hardware_id: None,
        license_cache: None,
        enable_server_pointer: true,
        autologon: false,
        enable_audio_playback: true,
        request_data: None,
        pointer_software_rendering: false,
        performance_flags: PerformanceFlags::default(),
        timezone_info: TimezoneInfo::default(),
    };

    Some(Config {
        destination: Destination::from_parts(hostname, port),
        connector: connector_config,
        clipboard_type: ClipboardType::Enable,
        log_file: None,
        gw: None,
        kerberos_config: None,
        rdcleanpath: None,
        fake_events_interval: None,
        dvc_pipe_proxies: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_domain_with_backslash() {
        let (domain, user) = split_domain("DOMAIN\\user");
        assert_eq!(domain.as_deref(), Some("DOMAIN"));
        assert_eq!(user, "user");
    }

    #[test]
    fn split_domain_bare_username() {
        let (domain, user) = split_domain("user");
        assert_eq!(domain, None);
        assert_eq!(user, "user");
    }

    #[test]
    fn split_domain_only_first_backslash() {
        let (domain, user) = split_domain("DOMAIN\\sub\\user");
        assert_eq!(domain.as_deref(), Some("DOMAIN"));
        assert_eq!(user, "sub\\user");
    }
}

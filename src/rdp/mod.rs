/* rdp/mod.rs
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

//! GTK widget and FreeRDP session integration.

mod clipboard;
mod clipboard_codec;
mod config;
mod errors;
pub(crate) mod ffi;
mod input;
mod key_handler;
mod portal_transfer;
mod render;
mod session;
mod widget;

pub use config::parse_hostname_port;
pub use widget::{RdpState, RdpWidget};

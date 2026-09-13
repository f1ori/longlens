/* rdp/errors.rs
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

use gettextrs::gettext;

use super::session::ConnectionError;

/// Reports why an established session could not be restored.
pub(super) fn friendly_termination_error(error: &ConnectionError) -> String {
    match error.class {
        1 | 2 => friendly_connection_error(error),
        _ => format!(
            "{}\n\n{}",
            gettext("The connection to the server was interrupted and could not be restored."),
            error_detail(error)
        ),
    }
}

pub(super) fn friendly_connection_error(error: &ConnectionError) -> String {
    match error.class {
        1 => gettext(
            "The username or password is incorrect. Please check your credentials and try again.",
        ),
        2 => gettext(
            "Access was denied. The username or password may be incorrect, or this account is not allowed to connect.",
        ),
        _ => format!(
            "{}\n\n{}",
            gettext("Could not connect to the server."),
            error_detail(error)
        ),
    }
}

fn error_detail(error: &ConnectionError) -> String {
    if !error.message.is_empty() {
        return error.message.clone();
    }
    if !error.name.is_empty() {
        return error.name.clone();
    }
    format!("FreeRDP error 0x{:08x}", error.code)
}

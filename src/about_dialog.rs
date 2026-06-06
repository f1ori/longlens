/* about_dialog.rs
 *
 * Copyright 2025 Florian Richter
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
use adw::prelude::*;
use gtk::Window;

use crate::config::{APP_ID, METAINFO_PATH, PROFILE, VCS_TAG, VERSION};

pub fn show(parent: &Window) {
    let about = adw::AboutDialog::from_appdata(METAINFO_PATH, Some(VERSION));

    if PROFILE == "development" {
        about.set_version(&format!("{VERSION}-{VCS_TAG} (devel)"));
    } else {
        about.set_version(VERSION);
    }

    about.set_application_icon(APP_ID);
    about.set_developers(&["Florian Richter <florian@richter-es.de>"]);
    // Translators: Replace "translator-credits" with your name/username, and optionally an email or URL.
    about.set_translator_credits(&gettext("translator-credits"));
    about.set_copyright("© 2026 Florian Richter");

    about.present(Some(parent));
}

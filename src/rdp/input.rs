/* rdp/input.rs
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

//! Pure translation of GDK input into FreeRDP input flags.

use gtk::gdk;
use tracing::warn;

pub const PTR_FLAGS_HWHEEL: u16 = 0x0400;
pub const PTR_FLAGS_WHEEL: u16 = 0x0200;
pub const PTR_FLAGS_WHEEL_NEGATIVE: u16 = 0x0100;
pub const PTR_FLAGS_MOVE: u16 = 0x0800;
pub const PTR_FLAGS_DOWN: u16 = 0x8000;
pub const PTR_FLAGS_BUTTON1: u16 = 0x1000;
pub const PTR_FLAGS_BUTTON2: u16 = 0x2000;
pub const PTR_FLAGS_BUTTON3: u16 = 0x4000;

pub fn mouse_button(gdk_button: u32) -> Option<u16> {
    match gdk_button {
        gdk::BUTTON_PRIMARY => Some(PTR_FLAGS_BUTTON1),
        gdk::BUTTON_SECONDARY => Some(PTR_FLAGS_BUTTON2),
        gdk::BUTTON_MIDDLE => Some(PTR_FLAGS_BUTTON3),
        _ => None,
    }
}

/// Translate an XKB keycode to the scancode form accepted by FreeRDP.
pub fn key_scancode(keycode: u16) -> Option<u32> {
    match keycode::KeyMap::from_key_mapping(keycode::KeyMapping::Xkb(keycode)) {
        Ok(map) => {
            let win = u32::from(map.win);
            Some(match win & 0xff00 {
                0xe000 => (win & 0xff) | 0x0100,
                0xe100 => (win & 0xff) | 0x0200,
                _ => win,
            })
        }
        Err(_) => {
            warn!("Unknown keycode {keycode}");
            None
        }
    }
}

/// Return FreeRDP wheel flags for each non-zero scroll axis.
pub fn scroll_flags(dx: f64, dy: f64, unit: gdk::ScrollUnit) -> Vec<u16> {
    const WHEEL_DELTA: f64 = 120.0;
    const PIXELS_PER_NOTCH: f64 = 50.0;
    let factor = match unit {
        gdk::ScrollUnit::Wheel => WHEEL_DELTA,
        _ => WHEEL_DELTA / PIXELS_PER_NOTCH,
    };

    let mut result = Vec::with_capacity(2);
    if dx.abs() > 0.001 {
        result.push(wheel_flags(dx * factor, true));
    }
    if dy.abs() > 0.001 {
        result.push(wheel_flags(-dy * factor, false));
    }
    result
}

fn wheel_flags(rotation: f64, horizontal: bool) -> u16 {
    let rotation = (rotation as i32).clamp(-255, 255);
    let axis = if horizontal {
        PTR_FLAGS_HWHEEL
    } else {
        PTR_FLAGS_WHEEL
    };
    if rotation < 0 {
        axis | PTR_FLAGS_WHEEL_NEGATIVE | ((0x100 + rotation) as u16)
    } else {
        axis | rotation as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_buttons() {
        assert_eq!(mouse_button(gdk::BUTTON_PRIMARY), Some(PTR_FLAGS_BUTTON1));
        assert_eq!(mouse_button(gdk::BUTTON_SECONDARY), Some(PTR_FLAGS_BUTTON2));
        assert_eq!(mouse_button(gdk::BUTTON_MIDDLE), Some(PTR_FLAGS_BUTTON3));
        assert_eq!(mouse_button(8), None);
    }

    #[test]
    fn keycodes_include_extended_flag() {
        assert_eq!(key_scancode(38), Some(0x001e)); // A
        assert_eq!(key_scancode(105), Some(0x011d)); // Right Control
    }

    #[test]
    fn wheel_sign_and_axes() {
        assert_eq!(
            scroll_flags(0.0, 1.0, gdk::ScrollUnit::Wheel),
            vec![PTR_FLAGS_WHEEL | PTR_FLAGS_WHEEL_NEGATIVE | (0x100 - 120)]
        );
        assert_eq!(
            scroll_flags(1.0, 0.0, gdk::ScrollUnit::Wheel),
            vec![PTR_FLAGS_HWHEEL | 120]
        );
    }

    #[test]
    fn pixel_scroll_is_scaled() {
        assert_eq!(
            scroll_flags(0.0, 50.0, gdk::ScrollUnit::Surface),
            vec![PTR_FLAGS_WHEEL | PTR_FLAGS_WHEEL_NEGATIVE | (0x100 - 120)]
        );
    }
}

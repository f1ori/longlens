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

//! Pure translation of GDK input into IronRDP input operations.

use gtk::gdk;
use ironrdp::input::{MouseButton, Operation, Scancode, WheelRotations};
use smallvec::{SmallVec, smallvec};
use tracing::warn;

/// Maps a GDK mouse button number to the RDP mouse button, or `None` for
/// buttons we don't forward.
pub fn mouse_button(gdk_button: u32) -> Option<MouseButton> {
    match gdk_button {
        gdk::BUTTON_PRIMARY => Some(MouseButton::Left),
        gdk::BUTTON_SECONDARY => Some(MouseButton::Right),
        gdk::BUTTON_MIDDLE => Some(MouseButton::Middle),
        _ => None,
    }
}

/// Translates an XKB keycode into a Windows RDP scancode, or `None` (with a
/// warning) if the keycode is unknown.
pub fn key_scancode(keycode: u16) -> Option<Scancode> {
    let map = keycode::KeyMap::from_key_mapping(keycode::KeyMapping::Xkb(keycode));
    match map {
        Ok(map) => Some(Scancode::from_u16(map.win)),
        Err(_) => {
            warn!("Unknown keycode {}", keycode);
            None
        }
    }
}

/// Builds the wheel-rotation operations for a scroll event's deltas. Returns up
/// to two operations (horizontal and/or vertical); sub-threshold deltas are
/// dropped. The vertical delta is inverted to match RDP's wheel direction.
pub fn scroll_operations(dx: f64, dy: f64) -> SmallVec<[Operation; 2]> {
    let mut ops: SmallVec<[Operation; 2]> = smallvec![];
    if dx.abs() > 0.001 {
        ops.push(Operation::WheelRotations(WheelRotations {
            is_vertical: false,
            rotation_units: (dx * 120.0) as i16,
        }));
    }
    if dy.abs() > 0.001 {
        ops.push(Operation::WheelRotations(WheelRotations {
            is_vertical: true,
            rotation_units: (-dy * 120.0) as i16,
        }));
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_known() {
        assert_eq!(mouse_button(gdk::BUTTON_PRIMARY), Some(MouseButton::Left));
        assert_eq!(mouse_button(gdk::BUTTON_SECONDARY), Some(MouseButton::Right));
        assert_eq!(mouse_button(gdk::BUTTON_MIDDLE), Some(MouseButton::Middle));
    }

    #[test]
    fn mouse_button_unknown() {
        assert_eq!(mouse_button(8), None);
    }

    #[test]
    fn scroll_below_threshold_is_empty() {
        assert!(scroll_operations(0.0, 0.0).is_empty());
        assert!(scroll_operations(0.0005, -0.0005).is_empty());
    }

    #[test]
    fn scroll_vertical_is_inverted() {
        let ops = scroll_operations(0.0, 1.0);
        assert_eq!(ops.len(), 1);
        let Operation::WheelRotations(w) = ops[0] else {
            panic!("expected wheel rotation");
        };
        assert!(w.is_vertical);
        assert_eq!(w.rotation_units, -120);
    }

    #[test]
    fn scroll_horizontal_not_inverted() {
        let ops = scroll_operations(1.0, 0.0);
        assert_eq!(ops.len(), 1);
        let Operation::WheelRotations(w) = ops[0] else {
            panic!("expected wheel rotation");
        };
        assert!(!w.is_vertical);
        assert_eq!(w.rotation_units, 120);
    }

    #[test]
    fn scroll_both_axes() {
        let ops = scroll_operations(1.0, 1.0);
        assert_eq!(ops.len(), 2);
    }
}

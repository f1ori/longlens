/* rdp/render.rs
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

//! Conversion of IronRDP framebuffer and pointer data into GDK textures/cursors.

use core::num::NonZeroU16;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ironrdp::graphics::pointer::DecodedPointer;

/// Wraps the framebuffer so it can be handed to `glib::Bytes::from_owned`
/// without copying. ironrdp encodes pixels as `0x00RRGGBB`, which on
/// little-endian is `[B, G, R, 0x00]` in memory — exactly `B8g8r8x8` — so the
/// `u32` slice can be reinterpreted as bytes directly.
pub struct PixelBytes(pub Vec<u32>);

impl AsRef<[u8]> for PixelBytes {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: the slice is valid for `len * 4` bytes, `u32` is suitably
        // aligned for `u8`, and every byte pattern is a valid `u8`.
        unsafe {
            std::slice::from_raw_parts(
                self.0.as_ptr() as *const u8,
                std::mem::size_of_val(self.0.as_slice()),
            )
        }
    }
}

/// Builds a GDK texture from an IronRDP framebuffer. The framebuffer layout
/// already matches `B8g8r8x8`, so it is reinterpreted as bytes without copying
/// (see [`PixelBytes`]).
pub fn image_texture(buffer: Vec<u32>, width: NonZeroU16, height: NonZeroU16) -> gdk::MemoryTexture {
    let bytes = glib::Bytes::from_owned(PixelBytes(buffer));
    gdk::MemoryTexture::new(
        width.get().into(),
        height.get().into(),
        gdk::MemoryFormat::B8g8r8x8,
        &bytes,
        (width.get() as usize) * 4,
    )
}

/// Builds a GDK cursor from an IronRDP pointer bitmap, or `None` if the cursor
/// could not be created.
pub fn pointer_cursor(pointer: &DecodedPointer) -> Option<gdk::Cursor> {
    let tex_w = pointer.width as i32;
    let tex_h = pointer.height as i32;
    let hotspot_x = pointer.hotspot_x as i32;
    let hotspot_y = pointer.hotspot_y as i32;
    let bitmap_bytes = glib::Bytes::from_owned(pointer.bitmap_data.clone());
    let texture = gdk::MemoryTexture::new(
        tex_w,
        tex_h,
        gdk::MemoryFormat::R8g8b8a8,
        &bitmap_bytes,
        (tex_w as usize) * 4,
    );
    // from_callback lets GTK pass width/height as logical pixels so the
    // cursor is displayed at the correct size on HiDPI displays.
    gdk::Cursor::from_callback(
        move |_cursor, _cursor_size, scale, width, height, hx, hy| {
            *width = (tex_w as f64 / scale).round() as i32;
            *height = (tex_h as f64 / scale).round() as i32;
            *hx = hotspot_x;
            *hy = hotspot_y;
            let t = texture.clone().upcast::<gdk::Texture>();
            // The gtk4-rs binding uses to_glib_none (no ref bump) but
            // GdkCursorGetTextureCallback has transfer:full semantics —
            // GTK calls g_object_unref on the returned pointer. Leaking
            // one clone here provides the extra ref GTK will consume,
            // keeping the closure's own reference intact.
            std::mem::forget(t.clone());
            t
        },
        None,
    )
}

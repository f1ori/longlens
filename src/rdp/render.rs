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

//! Conversion of FreeRDP framebuffer and pointer data into GDK objects.

use gtk::prelude::*;
use gtk::{gdk, glib};

pub fn image_texture(
    buffer: Vec<u8>,
    width: u32,
    height: u32,
    stride: u32,
) -> Option<gdk::MemoryTexture> {
    let width = i32::try_from(width).ok()?;
    let height = i32::try_from(height).ok()?;
    let bytes = glib::Bytes::from_owned(buffer);
    Some(gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::B8g8r8x8,
        &bytes,
        stride as usize,
    ))
}

pub fn pointer_cursor(
    data: Vec<u8>,
    width: u32,
    height: u32,
    hotspot_x: u32,
    hotspot_y: u32,
    connection_scale: f64,
) -> Option<gdk::Cursor> {
    let tex_w = i32::try_from(width).ok()?;
    let tex_h = i32::try_from(height).ok()?;
    let bytes = glib::Bytes::from_owned(data);
    let texture = gdk::MemoryTexture::new(
        tex_w,
        tex_h,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    );
    gdk::Cursor::from_callback(
        move |_cursor, _cursor_size, _scale, out_width, out_height, hx, hy| {
            *out_width = (width as f64 / connection_scale).round() as i32;
            *out_height = (height as f64 / connection_scale).round() as i32;
            *hx = (hotspot_x as f64 / connection_scale).round() as i32;
            *hy = (hotspot_y as f64 / connection_scale).round() as i32;
            let texture = texture.clone().upcast::<gdk::Texture>();
            std::mem::forget(texture.clone());
            texture
        },
        None,
    )
}

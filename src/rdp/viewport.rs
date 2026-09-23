/* rdp/viewport.rs
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

//! Placement of the remote desktop inside the widget, shared by drawing and
//! input so both agree on where the remote pixels are.

/// How the remote framebuffer is placed inside the widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// The framebuffer matches the widget; draw it pixel for pixel.
    Exact,
    /// Fill the whole widget, ignoring the aspect ratio. Used while a resize
    /// requested from the server is still in flight.
    Stretch,
    /// Scale to fit while keeping the aspect ratio, centered. Used when the
    /// server cannot follow the window size.
    Contain,
}

/// A rectangle in logical widget coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    /// Widget size in logical pixels.
    widget_width: f64,
    widget_height: f64,
    /// Surface scale, device pixels per logical pixel.
    scale: f64,
    /// Current remote framebuffer size in device pixels; 0 until the first
    /// frame arrives.
    remote_width: u32,
    remote_height: u32,
    /// Whether the server follows the window size.
    server_resizable: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            widget_width: 0.0,
            widget_height: 0.0,
            scale: 1.0,
            remote_width: 0,
            remote_height: 0,
            server_resizable: false,
        }
    }
}

impl Viewport {
    pub fn set_widget_size(&mut self, width: f64, height: f64, scale: f64) {
        self.widget_width = width;
        self.widget_height = height;
        self.scale = scale;
    }

    pub fn set_remote_size(&mut self, width: u32, height: u32) {
        self.remote_width = width;
        self.remote_height = height;
    }

    pub fn set_server_resizable(&mut self, resizable: bool) {
        self.server_resizable = resizable;
    }

    /// Forgets everything about the remote side, for a new connection. The
    /// widget size and scale are kept.
    pub fn reset_remote(&mut self) {
        self.set_remote_size(0, 0);
        self.server_resizable = false;
    }

    pub fn fit(&self) -> Fit {
        let physical_width = (self.widget_width * self.scale).round();
        let physical_height = (self.widget_height * self.scale).round();
        if (physical_width - self.remote_width as f64).abs() <= 1.0
            && (physical_height - self.remote_height as f64).abs() <= 1.0
        {
            Fit::Exact
        } else if self.server_resizable {
            Fit::Stretch
        } else {
            Fit::Contain
        }
    }

    /// Where the remote framebuffer is drawn, in logical widget coordinates.
    pub fn display_rect(&self) -> Rect {
        let remote_width = self.remote_width as f64;
        let remote_height = self.remote_height as f64;
        match self.fit() {
            Fit::Exact => Rect {
                x: 0.0,
                y: 0.0,
                width: remote_width / self.scale,
                height: remote_height / self.scale,
            },
            Fit::Stretch => Rect {
                x: 0.0,
                y: 0.0,
                width: self.widget_width,
                height: self.widget_height,
            },
            Fit::Contain => {
                let factor =
                    (self.widget_width / remote_width).min(self.widget_height / remote_height);
                let width = remote_width * factor;
                let height = remote_height * factor;
                Rect {
                    x: (self.widget_width - width) / 2.0,
                    y: (self.widget_height - height) / 2.0,
                    width,
                    height,
                }
            }
        }
    }

    /// Translates a point in widget coordinates to remote desktop
    /// coordinates. Points outside the drawn framebuffer are clamped to its
    /// edge.
    pub fn to_remote(&self, x: f64, y: f64) -> (u16, u16) {
        if self.remote_width == 0 || self.remote_height == 0 {
            return (
                (x * self.scale).round().clamp(0.0, u16::MAX as f64) as u16,
                (y * self.scale).round().clamp(0.0, u16::MAX as f64) as u16,
            );
        }

        fn axis(value: f64, origin: f64, extent: f64, size: u32) -> u16 {
            if extent <= 0.0 {
                return 0;
            }
            let max = (size - 1).min(u16::MAX as u32) as f64;
            ((value - origin) * size as f64 / extent)
                .floor()
                .clamp(0.0, max) as u16
        }
        let rect = self.display_rect();
        (
            axis(x, rect.x, rect.width, self.remote_width),
            axis(y, rect.y, rect.height, self.remote_height),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport(
        widget: (f64, f64),
        scale: f64,
        remote: (u32, u32),
        server_resizable: bool,
    ) -> Viewport {
        let mut viewport = Viewport::default();
        viewport.set_widget_size(widget.0, widget.1, scale);
        viewport.set_remote_size(remote.0, remote.1);
        viewport.set_server_resizable(server_resizable);
        viewport
    }

    #[test]
    fn exact_uses_surface_scale() {
        let viewport = viewport((640.0, 400.0), 2.0, (1280, 800), false);
        assert_eq!(viewport.fit(), Fit::Exact);
        assert_eq!(
            viewport.display_rect(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 400.0
            }
        );
        assert_eq!(viewport.to_remote(100.0, 50.0), (200, 100));
    }

    #[test]
    fn exact_tolerates_one_pixel() {
        let viewport = viewport((640.0, 400.0), 1.5, (961, 599), false);
        assert_eq!(viewport.fit(), Fit::Exact);
    }

    #[test]
    fn stretch_fills_widget_while_server_resizes() {
        let viewport = viewport((1000.0, 500.0), 1.0, (800, 600), true);
        assert_eq!(viewport.fit(), Fit::Stretch);
        assert_eq!(
            viewport.display_rect(),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 500.0
            }
        );
        assert_eq!(viewport.to_remote(500.0, 250.0), (400, 300));
    }

    #[test]
    fn contain_letterboxes_wide_widget() {
        let viewport = viewport((1000.0, 300.0), 1.0, (800, 600), false);
        assert_eq!(viewport.fit(), Fit::Contain);
        assert_eq!(
            viewport.display_rect(),
            Rect {
                x: 300.0,
                y: 0.0,
                width: 400.0,
                height: 300.0
            }
        );
        assert_eq!(viewport.to_remote(300.0, 0.0), (0, 0));
        assert_eq!(viewport.to_remote(500.0, 150.0), (400, 300));
    }

    #[test]
    fn contain_letterboxes_tall_widget() {
        let viewport = viewport((400.0, 1000.0), 1.0, (800, 600), false);
        assert_eq!(
            viewport.display_rect(),
            Rect {
                x: 0.0,
                y: 350.0,
                width: 400.0,
                height: 300.0
            }
        );
    }

    #[test]
    fn points_outside_are_clamped() {
        let viewport = viewport((1000.0, 300.0), 1.0, (800, 600), false);
        assert_eq!(viewport.to_remote(10.0, -5.0), (0, 0));
        assert_eq!(viewport.to_remote(990.0, 400.0), (799, 599));
    }

    #[test]
    fn falls_back_to_scale_before_first_frame() {
        let mut viewport = viewport((640.0, 400.0), 2.0, (800, 600), true);
        viewport.reset_remote();
        assert_eq!(viewport.to_remote(10.25, 20.0), (21, 40));
    }
}

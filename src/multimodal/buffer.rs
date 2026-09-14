//! RGBA image buffer with drawing primitives and PNG/Base64 serialization.

use anyhow::Result;
use base64::prelude::*;

use super::png::encode_rgba_to_png;

/// An in-memory RGBA pixel buffer representing a rasterized document page.
#[derive(Debug, Clone)]
pub struct RgbaImageBuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // 4 bytes per pixel: [R, G, B, A]
}

impl RgbaImageBuffer {
    /// Create a new RGBA buffer initialized with a solid fill color.
    pub fn new(width: u32, height: u32, fill: [u8; 4]) -> Self {
        let pixel_count = (width as usize) * (height as usize);
        let mut data = Vec::with_capacity(pixel_count * 4);
        for _ in 0..pixel_count {
            data.extend_from_slice(&fill);
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Set an individual pixel color with bounds checking.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if x < self.width && y < self.height {
            let idx = ((y as usize * self.width as usize) + x as usize) * 4;
            self.data[idx..idx + 4].copy_from_slice(&color);
        }
    }

    /// Draw a filled rectangle.
    pub fn draw_filled_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) {
        let x_end = (x + w).min(self.width);
        let y_end = (y + h).min(self.height);

        for py in y..y_end {
            for px in x..x_end {
                self.set_pixel(px, py, color);
            }
        }
    }

    /// Draw a rectangle outline with specified thickness.
    pub fn draw_rect(&mut self, x: u32, y: u32, w: u32, h: u32, thickness: u32, color: [u8; 4]) {
        // Top edge
        self.draw_filled_rect(x, y, w, thickness, color);
        // Bottom edge
        self.draw_filled_rect(x, (y + h).saturating_sub(thickness), w, thickness, color);
        // Left edge
        self.draw_filled_rect(x, y, thickness, h, color);
        // Right edge
        self.draw_filled_rect((x + w).saturating_sub(thickness), y, thickness, h, color);
    }

    /// Draw a horizontal line.
    pub fn draw_hline(&mut self, x: u32, y: u32, length: u32, color: [u8; 4]) {
        self.draw_filled_rect(x, y, length, 1, color);
    }

    /// Draw a vertical line.
    pub fn draw_vline(&mut self, x: u32, y: u32, length: u32, color: [u8; 4]) {
        self.draw_filled_rect(x, y, 1, length, color);
    }

    /// Scale and blit a bitmap (RGB or RGBA) into the target destination bounding box using nearest neighbor sampling.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_bitmap(
        &mut self,
        dest_x: u32,
        dest_y: u32,
        dest_w: u32,
        dest_h: u32,
        src_w: u32,
        src_h: u32,
        src_data: &[u8],
        channels: usize,
    ) {
        if src_w == 0 || src_h == 0 || dest_w == 0 || dest_h == 0 {
            return;
        }

        let bytes_per_pixel = channels.clamp(1, 4);

        for dy in 0..dest_h {
            let py = dest_y + dy;
            if py >= self.height {
                break;
            }

            let sy = (dy * src_h) / dest_h;

            for dx in 0..dest_w {
                let px = dest_x + dx;
                if px >= self.width {
                    break;
                }

                let sx = (dx * src_w) / dest_w;
                let s_idx = ((sy as usize * src_w as usize) + sx as usize) * bytes_per_pixel;

                if s_idx + bytes_per_pixel <= src_data.len() {
                    let pixel = match bytes_per_pixel {
                        1 => {
                            let g = src_data[s_idx];
                            [g, g, g, 255]
                        }
                        3 => [
                            src_data[s_idx],
                            src_data[s_idx + 1],
                            src_data[s_idx + 2],
                            255,
                        ],
                        4 => [
                            src_data[s_idx],
                            src_data[s_idx + 1],
                            src_data[s_idx + 2],
                            src_data[s_idx + 3],
                        ],
                        _ => [0, 0, 0, 255],
                    };
                    self.set_pixel(px, py, pixel);
                }
            }
        }
    }

    /// Convert the pixel buffer into PNG format bytes.
    pub fn to_png_bytes(&self) -> Result<Vec<u8>> {
        encode_rgba_to_png(self.width, self.height, &self.data)
    }

    /// Encode the pixel buffer directly as a Base64-encoded PNG string.
    pub fn to_base64_png(&self) -> Result<String> {
        let png_bytes = self.to_png_bytes()?;
        Ok(BASE64_STANDARD.encode(&png_bytes))
    }
}

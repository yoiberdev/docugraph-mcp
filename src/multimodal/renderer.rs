//! Page rendering abstraction (GoF Adapter pattern) and native rasterizer.

use anyhow::Result;
use lopdf::content::Content;
use lopdf::{Encoding, Object};
use std::collections::BTreeMap;
use tracing::debug;

use super::buffer::RgbaImageBuffer;
use crate::document::layout::extract_positioned_fragments;

/// GoF Adapter pattern: Abstract interface for document page rasterization.
pub trait PageRenderer: Send + Sync {
    /// Rasterize a single page into an RGBA pixel buffer.
    fn render_page(
        &self,
        doc: &lopdf::Document,
        page_id: (u32, u16),
        page_num: u32,
        max_width: u32,
    ) -> Result<RgbaImageBuffer>;
}

/// Native pure-Rust page renderer that computes spatial layouts, blits embedded
/// diagram images, and rasterizes text blocks without external C/C++ libraries.
#[derive(Debug, Default, Clone)]
pub struct NativePageRenderer;

impl NativePageRenderer {
    pub fn new() -> Self {
        Self
    }

    /// Extract page MediaBox bounds `[x_min, y_min, x_max, y_max]` in user-space points.
    fn get_page_dimensions(doc: &lopdf::Document, page_id: (u32, u16)) -> (f32, f32, f32, f32) {
        if let Ok(page_dict) = doc.get_dictionary(page_id)
            && let Ok(Object::Array(mb)) = page_dict.get(b"MediaBox")
            && mb.len() >= 4
        {
            let x0 = mb[0].as_float().unwrap_or(0.0);
            let y0 = mb[1].as_float().unwrap_or(0.0);
            let x1 = mb[2].as_float().unwrap_or(595.0);
            let y1 = mb[3].as_float().unwrap_or(842.0);
            return (x0, y0, x1, y1);
        }

        // Default to standard A4 (595.28 x 841.89 pt)
        (0.0, 0.0, 595.0, 842.0)
    }
}

impl PageRenderer for NativePageRenderer {
    fn render_page(
        &self,
        doc: &lopdf::Document,
        page_id: (u32, u16),
        page_num: u32,
        max_width: u32,
    ) -> Result<RgbaImageBuffer> {
        debug!(
            target: "renderer",
            page = page_num,
            max_width = max_width,
            "Rasterizing PDF page"
        );

        let (x0, y0, x1, y1) = Self::get_page_dimensions(doc, page_id);
        let pt_width = (x1 - x0).abs().max(50.0);
        let pt_height = (y1 - y0).abs().max(50.0);

        let target_width = max_width.clamp(200, 2048);
        let scale = (target_width as f32) / pt_width;
        let target_height = ((pt_height * scale).round() as u32).max(100);

        // 1. Initialize white background canvas
        let mut canvas = RgbaImageBuffer::new(target_width, target_height, [255, 255, 255, 255]);

        // 2. Draw subtle border around page margins
        canvas.draw_rect(0, 0, target_width, target_height, 1, [225, 228, 232, 255]);

        // 3. Check for embedded bitmap images (diagrams, architecture schemes, scans)
        if let Ok(images) = doc.get_page_images(page_id) {
            for img in images {
                if img.width > 0 && img.height > 0 {
                    let decompressed = match img.filters.as_deref() {
                        Some(filters) if filters.iter().any(|f| f == "FlateDecode") => {
                            miniz_oxide::inflate::decompress_to_vec_zlib(img.content).ok()
                        }
                        _ => Some(img.content.to_vec()),
                    };

                    if let Some(raw_bytes) = decompressed {
                        let channels = match img.color_space.as_deref() {
                            Some("DeviceRGB") => 3,
                            Some("DeviceGray") => 1,
                            Some("DeviceCMYK") => 4,
                            _ => 3,
                        };

                        // Blit image centered or scaled to fit nicely
                        let dest_w = ((img.width as f32 * scale).round() as u32)
                            .min(target_width.saturating_sub(40));
                        let dest_h = ((img.height as f32 * scale).round() as u32)
                            .min(target_height.saturating_sub(40));
                        let dest_x = (target_width.saturating_sub(dest_w)) / 2;
                        let dest_y = (target_height.saturating_sub(dest_h)) / 2;

                        canvas.draw_bitmap(
                            dest_x,
                            dest_y,
                            dest_w,
                            dest_h,
                            img.width as u32,
                            img.height as u32,
                            &raw_bytes,
                            channels,
                        );
                    }
                }
            }
        }

        // 4. Rasterize text layout blocks
        let content_data = doc
            .get_page_content_with_limit(page_id, crate::document::MAX_DECOMPRESSED_BYTES)
            .unwrap_or_default();
        if !content_data.is_empty()
            && let Ok(content) = Content::decode(&content_data)
        {
            let encodings: BTreeMap<Vec<u8>, Encoding> = doc
                .get_page_fonts(page_id)
                .map(|fonts| {
                    fonts
                        .into_iter()
                        .filter_map(|(name, font)| {
                            font.get_font_encoding(doc).ok().map(|enc| (name, enc))
                        })
                        .collect()
                })
                .unwrap_or_default();

            let fragments = extract_positioned_fragments(&content, &encodings);

            for frag in fragments {
                // PDF native coordinate origin is bottom-left; canvas origin is top-left
                let px = ((frag.bbox.x - x0) * scale).round() as i32;
                let py = ((y1 - frag.bbox.y - frag.bbox.height) * scale).round() as i32;
                let pw = (frag.bbox.width * scale).round() as u32;
                let ph = (frag.bbox.height * scale).round() as u32;

                if px >= 0 && py >= 0 && (px as u32) < target_width && (py as u32) < target_height {
                    let ux = px as u32;
                    let uy = py as u32;
                    let w = pw.max(4).min(target_width - ux);
                    let h = ph.max(2).min(target_height - uy);

                    // Headings (larger font size >= 14pt) drawn in darker ink
                    let color = if frag.bbox.height >= 14.0 {
                        [30, 41, 59, 230] // Dark slate for titles/headings
                    } else {
                        [71, 85, 105, 180] // Slate gray for regular body text
                    };

                    // Render text line block
                    let line_height = (h / 3).max(2);
                    canvas.draw_filled_rect(ux, uy + (h / 3), w, line_height, color);
                }
            }
        }

        Ok(canvas)
    }
}

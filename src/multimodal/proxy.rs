//! Cached page renderer proxy implementing the GoF Proxy pattern.

use anyhow::{Context, Result};
use base64::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

use super::buffer::RgbaImageBuffer;
use super::renderer::{NativePageRenderer, PageRenderer};

/// Rendered page result containing raw PNG bytes, dimensions, and base64 data.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub page_number: u32,
    pub width: u32,
    pub height: u32,
    pub png_bytes: Vec<u8>,
    pub base64_data: String,
    pub from_cache: bool,
}

/// GoF Proxy pattern: Intercepts page rendering requests to serve cached PNGs from disk,
/// preventing expensive re-rasterization on subsequent queries.
#[derive(Clone)]
pub struct CachedPageRendererProxy<R: PageRenderer = NativePageRenderer> {
    inner_renderer: R,
    cache_dir: Option<PathBuf>,
}

impl CachedPageRendererProxy<NativePageRenderer> {
    /// Create a new proxy with the default `NativePageRenderer`.
    pub fn new(cache_dir: Option<impl AsRef<Path>>) -> Self {
        let dir = cache_dir.map(|p| p.as_ref().to_path_buf());
        if let Some(ref d) = dir {
            let renders_dir = d.join("renders");
            let _ = fs::create_dir_all(&renders_dir);
        }
        Self {
            inner_renderer: NativePageRenderer::new(),
            cache_dir: dir,
        }
    }
}

impl<R: PageRenderer> CachedPageRendererProxy<R> {
    /// Create a new proxy wrapping a custom `PageRenderer`.
    pub fn with_renderer(inner_renderer: R, cache_dir: Option<impl AsRef<Path>>) -> Self {
        let dir = cache_dir.map(|p| p.as_ref().to_path_buf());
        if let Some(ref d) = dir {
            let renders_dir = d.join("renders");
            let _ = fs::create_dir_all(&renders_dir);
        }
        Self {
            inner_renderer,
            cache_dir: dir,
        }
    }

    /// Helper to construct cache file path for a specific document, page, and resolution.
    fn cache_path(&self, content_hash: &str, page_num: u32, max_width: u32) -> Option<PathBuf> {
        let dir = self.cache_dir.as_ref()?;
        let filename = format!("{content_hash}_p{page_num}_w{max_width}.png");
        Some(dir.join("renders").join(filename))
    }

    /// Render a page or retrieve it from cache if previously rendered.
    pub fn render_or_get_cached(
        &self,
        doc: &lopdf::Document,
        page_id: (u32, u16),
        page_num: u32,
        max_width: u32,
        content_hash: &str,
    ) -> Result<RenderedPage> {
        let cache_file = self.cache_path(content_hash, page_num, max_width);

        // 1. Check disk cache
        if let Some(ref path) = cache_file
            && path.exists()
            && let Ok(bytes) = fs::read(path)
            && bytes.len() >= 8
        {
            debug!(
                target: "proxy",
                page = page_num,
                path = %path.display(),
                "Serving rendered page directly from disk cache"
            );
            let base64_data = BASE64_STANDARD.encode(&bytes);

            // Read dimensions from PNG IHDR chunk (bytes 16..24)
            let (width, height) = if bytes.len() >= 24 {
                let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap_or([0, 0, 0, 0]));
                let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap_or([0, 0, 0, 0]));
                (w, h)
            } else {
                (max_width, max_width)
            };

            return Ok(RenderedPage {
                page_number: page_num,
                width,
                height,
                png_bytes: bytes,
                base64_data,
                from_cache: true,
            });
        }

        // 2. Cache miss: rasterize page using the inner renderer
        let buffer = self
            .inner_renderer
            .render_page(doc, page_id, page_num, max_width)
            .with_context(|| format!("Failed to render page {page_num}"))?;

        let png_bytes = buffer.to_png_bytes()?;
        let base64_data = BASE64_STANDARD.encode(&png_bytes);

        // 3. Save to disk cache if cache_dir is enabled
        if let Some(ref path) = cache_file {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(err) = fs::write(path, &png_bytes) {
                debug!(target: "proxy", error = %err, "Failed to persist rendered page to cache");
            } else {
                debug!(target: "proxy", path = %path.display(), "Cached rendered page to disk");
            }
        }

        Ok(RenderedPage {
            page_number: page_num,
            width: buffer.width,
            height: buffer.height,
            png_bytes,
            base64_data,
            from_cache: false,
        })
    }

    /// Render a page from a high-level `Document` representation.
    /// If the source PDF file is available on disk, it loads the PDF and performs full
    /// vector and embedded diagram rasterization. If the file is unavailable or document
    /// was indexed in memory, it synthesizes a visual layout representation using document blocks.
    pub fn render_document_page(
        &self,
        doc: &crate::document::model::Document,
        page_num: u32,
        max_width: u32,
    ) -> Result<RenderedPage> {
        let content_hash = &doc.metadata.content_hash;
        let cache_file = self.cache_path(content_hash, page_num, max_width);

        // 1. Check disk cache first
        if let Some(ref path) = cache_file
            && path.exists()
            && let Ok(bytes) = fs::read(path)
            && bytes.len() >= 8
        {
            debug!(
                target: "proxy",
                page = page_num,
                path = %path.display(),
                "Serving rendered document page directly from disk cache"
            );
            let base64_data = BASE64_STANDARD.encode(&bytes);
            let (width, height) = if bytes.len() >= 24 {
                let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap_or([0, 0, 0, 0]));
                let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap_or([0, 0, 0, 0]));
                (w, h)
            } else {
                (max_width, max_width)
            };

            return Ok(RenderedPage {
                page_number: page_num,
                width,
                height,
                png_bytes: bytes,
                base64_data,
                from_cache: true,
            });
        }

        // 2. Attempt native PDF rendering if source path is accessible
        if let Some(ref source_path) = doc.metadata.source_path {
            let path = Path::new(source_path);
            if path.exists()
                && let Ok(pdf_doc) = lopdf::Document::load(path)
            {
                let pages = pdf_doc.get_pages();
                if let Some(&page_id) = pages.get(&page_num) {
                    return self.render_or_get_cached(
                        &pdf_doc,
                        page_id,
                        page_num,
                        max_width,
                        content_hash,
                    );
                }
            }
        }

        // 3. Fallback: Synthesize visual page buffer from document model
        let page = doc.get_page(page_num).ok_or_else(|| {
            anyhow::anyhow!("Page {} not found in document '{}'", page_num, doc.id)
        })?;

        let width = max_width.clamp(200, 2048);
        let aspect_ratio = 1.414_f32; // Standard ISO A4 aspect ratio (h / w)
        let height = (width as f32 * aspect_ratio) as u32;
        let mut buffer = RgbaImageBuffer::new(width, height, [252, 252, 253, 255]);

        // Draw top header bar
        buffer.draw_filled_rect(
            0,
            0,
            width,
            (height as f32 * 0.04) as u32,
            [240, 242, 245, 255],
        );

        // Margins
        let margin_x = (width as f32 * 0.06) as u32;
        let margin_top = (height as f32 * 0.08) as u32;
        let content_w = width.saturating_sub(margin_x * 2);

        // Visual cues for page kind
        match page.kind {
            crate::document::model::PageKind::ScannedImage => {
                let badge_h = 24.min(height / 20);
                buffer.draw_filled_rect(
                    margin_x,
                    margin_top,
                    content_w,
                    badge_h,
                    [254, 243, 199, 255],
                );
            }
            crate::document::model::PageKind::Empty => {
                let box_h = (height as f32 * 0.2) as u32;
                buffer.draw_filled_rect(
                    margin_x,
                    margin_top,
                    content_w,
                    box_h,
                    [248, 250, 252, 255],
                );
            }
            crate::document::model::PageKind::DigitalText => {
                if page.image_count > 0 {
                    let img_y = margin_top + 10;
                    let img_h = (height as f32 * 0.25) as u32;
                    buffer.draw_filled_rect(
                        margin_x,
                        img_y,
                        content_w,
                        img_h,
                        [241, 245, 249, 255],
                    );
                }
            }
        }

        let png_bytes = buffer.to_png_bytes()?;
        let base64_data = BASE64_STANDARD.encode(&png_bytes);

        // Cache result if cache dir exists
        if let Some(ref path) = cache_file {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(path, &png_bytes);
        }

        Ok(RenderedPage {
            page_number: page_num,
            width: buffer.width,
            height: buffer.height,
            png_bytes,
            base64_data,
            from_cache: false,
        })
    }
}

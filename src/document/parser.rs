//! Robust PDF parsing, page extraction, and outline discovery using `lopdf`.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, warn};

use super::links::{decode_pdf_string, extract_page_links, object_to_string};
use super::model::{Document, DocumentId, DocumentMetadata, Page, PageKind};
use super::outline_strategy::{FallbackOutlineStrategy, OutlineExtractor};
use super::structure::slugify_title;

/// Security scan findings for a single page content stream.
#[derive(Debug, Default, Clone)]
pub struct PageSecurityScan {
    /// Whether invisible text (Tr 3) or microscopic text (font size < 1.5pt) was detected
    pub untrusted_text_detected: bool,
    /// Extracted suspicious text snippets that were hidden from visual presentation
    pub hidden_snippets: Vec<String>,
}

#[derive(Clone)]
struct GraphicsState {
    render_mode: i64,
    font_size: f32,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            render_mode: 0,
            font_size: 12.0,
        }
    }
}

/// Scan PDF page content stream operations for prompt injection vectors (Tr 3 invisible text, microscopic font size).
pub fn scan_page_security(doc: &lopdf::Document, page_id: (u32, u16)) -> PageSecurityScan {
    let mut scan = PageSecurityScan::default();
    let content_data = doc.get_page_content(page_id);
    if content_data.is_empty() {
        return scan;
    }

    let Ok(content) = lopdf::content::Content::decode(&content_data) else {
        return scan;
    };

    let mut state_stack = Vec::new();
    let mut current_state = GraphicsState::default();

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => {
                state_stack.push(current_state.clone());
            }
            "Q" => {
                if let Some(prev) = state_stack.pop() {
                    current_state = prev;
                }
            }
            "Tr" => {
                if let Some(mode) = op.operands.first().and_then(|o| o.as_i64().ok()) {
                    current_state.render_mode = mode;
                }
            }
            "Tf" => {
                if let Some(size) = op.operands.get(1).and_then(|o| o.as_float().ok()) {
                    current_state.font_size = size;
                }
            }
            "Tj" | "'" => {
                let is_invisible = current_state.render_mode == 3;
                let is_microscopic = current_state.font_size > 0.0 && current_state.font_size < 1.5;
                if is_invisible || is_microscopic {
                    scan.untrusted_text_detected = true;
                    if let Some(lopdf::Object::String(bytes, _)) = op.operands.first() {
                        let text = decode_pdf_string(bytes);
                        let clean = text.trim();
                        if !clean.is_empty() {
                            scan.hidden_snippets.push(clean.to_string());
                        }
                    }
                }
            }
            "\"" => {
                let is_invisible = current_state.render_mode == 3;
                let is_microscopic = current_state.font_size > 0.0 && current_state.font_size < 1.5;
                if is_invisible || is_microscopic {
                    scan.untrusted_text_detected = true;
                    if let Some(lopdf::Object::String(bytes, _)) = op.operands.get(2) {
                        let text = decode_pdf_string(bytes);
                        let clean = text.trim();
                        if !clean.is_empty() {
                            scan.hidden_snippets.push(clean.to_string());
                        }
                    }
                }
            }
            "TJ" => {
                let is_invisible = current_state.render_mode == 3;
                let is_microscopic = current_state.font_size > 0.0 && current_state.font_size < 1.5;
                if is_invisible || is_microscopic {
                    scan.untrusted_text_detected = true;
                    if let Some(lopdf::Object::Array(items)) = op.operands.first() {
                        let mut combined = String::new();
                        for item in items {
                            if let lopdf::Object::String(bytes, _) = item {
                                combined.push_str(&decode_pdf_string(bytes));
                            }
                        }
                        let clean = combined.trim();
                        if !clean.is_empty() {
                            scan.hidden_snippets.push(clean.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    scan
}

/// Inspect a page for embedded bitmap images (via direct /Resources/XObject or inherited from /Pages parent).
/// Returns a list of (width, height) tuples for discovered images.
pub fn inspect_page_images(doc: &lopdf::Document, page_id: (u32, u16)) -> Vec<(i64, i64)> {
    // 1. Try lopdf's built-in get_page_images
    if let Ok(imgs) = doc.get_page_images(page_id)
        && !imgs.is_empty()
    {
        return imgs
            .into_iter()
            .map(|img| (img.width, img.height))
            .collect();
    }

    // 2. Fallback: check inherited /Resources in parent /Pages if direct dictionary had no XObjects
    let mut images = Vec::new();
    let inherited_xobjects = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|p| p.get(b"Parent").ok())
        .and_then(|p| p.as_reference().ok())
        .and_then(|parent_ref| doc.get_dictionary(parent_ref).ok())
        .and_then(|parent_dict| doc.get_dict_in_dict(parent_dict, b"Resources").ok())
        .and_then(|res_dict| doc.get_dict_in_dict(res_dict, b"XObject").ok());

    if let Some(xobject_dict) = inherited_xobjects {
        for (_, xval) in xobject_dict.iter() {
            if let Ok(ref_id) = xval.as_reference()
                && let Ok(obj) = doc.get_object(ref_id)
                && let Ok(stream) = obj.as_stream()
                && stream
                    .dict
                    .get(b"Subtype")
                    .and_then(|s| s.as_name())
                    .is_ok_and(|n| n == b"Image")
            {
                let w = stream
                    .dict
                    .get(b"Width")
                    .and_then(|o| o.as_i64())
                    .unwrap_or(0);
                let h = stream
                    .dict
                    .get(b"Height")
                    .and_then(|o| o.as_i64())
                    .unwrap_or(0);
                images.push((w, h));
            }
        }
    }

    images
}

/// Load and parse a PDF document from filesystem path into a structured `Document`.
pub fn load_pdf_from_path(path: impl AsRef<Path>) -> Result<Document> {
    load_pdf_from_path_with_password(path, None)
}

/// Load and parse a PDF document with optional password for decryption.
pub fn load_pdf_from_path_with_password(
    path: impl AsRef<Path>,
    password: Option<&str>,
) -> Result<Document> {
    let path = path.as_ref();
    info!(target: "parser", path = %path.display(), "Opening PDF document");

    if !path.exists() {
        anyhow::bail!("PDF file does not exist at path: {}", path.display());
    }

    // Read bytes for SHA-256 content hashing (used for document identity and caching)
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read PDF file: {}", path.display()))?;
    let file_size_bytes = bytes.len() as u64;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256_hash = hex::encode(hasher.finalize());

    // Load PDF using lopdf
    let mut pdf_doc = lopdf::Document::load_mem(&bytes)
        .with_context(|| format!("Failed to parse PDF binary structure: {}", path.display()))?;

    // Handle decryption if the PDF is encrypted
    let is_encrypted = pdf_doc.is_encrypted();
    if is_encrypted {
        info!(target: "parser", path = %path.display(), "Encrypted PDF detected; attempting decryption");
        let mut decrypted = false;

        // 1. Try user-supplied password if provided
        if let Some(pwd) = password {
            if pdf_doc.decrypt(pwd).is_ok() {
                decrypted = true;
                info!(target: "parser", "Successfully decrypted PDF with user password");
            } else {
                anyhow::bail!(
                    "Failed to decrypt PDF document with the provided password. Please check your password."
                );
            }
        }

        // 2. Try empty password fallback (common for permission-restricted PDFs with default empty user password)
        if !decrypted && pdf_doc.decrypt("").is_ok() {
            decrypted = true;
            info!(target: "parser", "Successfully decrypted PDF with empty password fallback");
        }

        if !decrypted {
            anyhow::bail!(
                "PDF document is password-protected or encrypted. Please provide a password with --password."
            );
        }
    }

    // Extract pages in sequential order and build reverse mapping (ObjectId -> PageNumber)
    let (page_numbers, page_map, pages_dict) = {
        let pages_map = pdf_doc.get_pages();
        let mut nums: Vec<u32> = pages_map.keys().copied().collect();
        nums.sort_unstable();

        let mut reverse_map = HashMap::with_capacity(pages_map.len());
        for (page_num, obj_id) in &pages_map {
            reverse_map.insert(*obj_id, *page_num);
        }
        (nums, reverse_map, pages_map)
    };

    let total_pages = page_numbers.len() as u32;
    info!(target: "parser", total_pages = total_pages, "Extracting text and scanning stream security");

    let mut pages: Vec<Page> = Vec::with_capacity(page_numbers.len());
    let mut doc_untrusted_detected = false;
    let mut scanned_pages_count = 0u32;

    for page_num in page_numbers {
        let page_id = pages_dict.get(&page_num).copied();
        let security_scan = if let Some(id) = page_id {
            scan_page_security(&pdf_doc, id)
        } else {
            PageSecurityScan::default()
        };

        let images = if let Some(id) = page_id {
            inspect_page_images(&pdf_doc, id)
        } else {
            Vec::new()
        };
        let image_count = images.len();

        let spatial_text =
            page_id.and_then(|id| super::layout::extract_page_text_spatial(&pdf_doc, id, true));

        let mut text = match spatial_text {
            Some(reconstructed) => {
                debug!(target: "parser", page = page_num, "Applied multi-column spatial reading order reconstruction");
                reconstructed
            }
            None => match pdf_doc.extract_text(&[page_num]) {
                Ok(extracted) => extracted,
                Err(err) => {
                    warn!(target: "parser", page = page_num, error = %err, "Failed to extract text for page; recording as empty");
                    String::new()
                }
            },
        };

        // Reconstruct tabular text zones into GitHub Flavored Markdown (GFM) tables
        if text.contains("  ") || text.contains('\t') || text.contains('|') {
            text = super::table::reconstruct_tables_in_text(&text);
        }

        let untrusted_detected = security_scan.untrusted_text_detected;

        if untrusted_detected {
            doc_untrusted_detected = true;
            warn!(
                target: "security",
                page = page_num,
                hidden_snippets_count = security_scan.hidden_snippets.len(),
                "Untrusted hidden or microscopic text detected in PDF content stream"
            );

            if security_scan.hidden_snippets.is_empty() {
                text.insert_str(
                    0,
                    "[SECURITY ADVISORY: Untrusted hidden or microscopic text detected on this page]\n",
                );
            } else {
                for snippet in &security_scan.hidden_snippets {
                    if text.contains(snippet) {
                        text = text.replace(
                            snippet,
                            &format!("[Untrusted Hidden Text: \"{}\"]", snippet),
                        );
                    } else {
                        text.push_str(&format!(
                            "\n\n[Untrusted Hidden Text Detected: \"{}\"]",
                            snippet
                        ));
                    }
                }
            }
        }

        let non_ws_chars = text
            .chars()
            .filter(|c| !c.is_whitespace() && !c.is_control())
            .count();
        let has_large_image = images
            .iter()
            .any(|&(w, h)| (w >= 150 && h >= 150) || (w * h) >= 40_000);

        let kind =
            if image_count > 0 && (non_ws_chars < 50 || (non_ws_chars < 150 && has_large_image)) {
                PageKind::ScannedImage
            } else if non_ws_chars == 0 && image_count == 0 {
                PageKind::Empty
            } else {
                PageKind::DigitalText
            };

        if kind == PageKind::ScannedImage {
            scanned_pages_count += 1;
            info!(
                target: "parser",
                page = page_num,
                image_count = image_count,
                text_chars = non_ws_chars,
                "Page classified as ScannedImage (image present with little or no digital text layer)"
            );

            if non_ws_chars == 0 {
                text = format!(
                    "[Aviso: La página {} es una imagen escaneada sin capa de texto digital ({} imagen(es) detectada(s)). Se requiere OCR externo para extraer su contenido textual.]",
                    page_num, image_count
                );
            } else {
                text = format!(
                    "[Aviso: La página {} parece ser un escaneo ({} imagen(es) detectada(s)) con capa de texto mínima o ruidosa ({} caracteres). Se recomienda OCR para texto completo.]\n\n{}",
                    page_num, image_count, non_ws_chars, text
                );
            }
        }

        let links = if let Some(id) = page_id {
            if let Ok(page_dict) = pdf_doc.get_dictionary(id) {
                extract_page_links(&pdf_doc, page_dict, page_num, &page_map)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        pages.push(Page {
            page_number: page_num,
            char_count: text.chars().count(),
            text,
            untrusted_text_detected: untrusted_detected,
            kind,
            image_count,
            links,
        });
    }

    // Extract document outlines using GoF Strategy pattern (Native first, typographic fallback)
    let outline_strategy = FallbackOutlineStrategy::new();
    let sections = outline_strategy.extract(&pdf_doc, &page_map, &pages, total_pages);

    // Determine document title from file stem or metadata
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");

    let title = extract_pdf_title(&pdf_doc).unwrap_or_else(|| file_stem.to_string());
    let author = extract_pdf_author(&pdf_doc);
    let doc_id = DocumentId(slugify_title(&title));
    let total_sections = sections.iter().map(|s| s.total_count()).sum::<usize>() as u32;
    let total_links = pages.iter().map(|p| p.links.len() as u32).sum();

    let metadata = DocumentMetadata {
        id: doc_id.0.clone(),
        title,
        author,
        total_pages,
        total_sections,
        file_size_bytes,
        content_hash: sha256_hash,
        indexed_at: chrono_timestamp_iso8601(),
        is_encrypted,
        untrusted_text_detected: doc_untrusted_detected,
        scanned_pages_count,
        source_path: Some(path.to_string_lossy().to_string()),
        total_links,
    };

    info!(
        target: "parser",
        doc_id = %doc_id,
        sections = sections.len(),
        pages = pages.len(),
        links = total_links,
        is_encrypted = is_encrypted,
        untrusted_text_detected = doc_untrusted_detected,
        scanned_pages_count = scanned_pages_count,
        "PDF ingestion, link extraction, security scan, and outline resolution complete"
    );

    Ok(Document {
        id: doc_id,
        metadata,
        pages,
        sections,
    })
}

/// Extract optional Title from PDF Info dictionary.
fn extract_pdf_title(doc: &lopdf::Document) -> Option<String> {
    let info_ref = doc.trailer.get(b"Info").ok()?.as_reference().ok()?;
    let info_dict = doc.get_dictionary(info_ref).ok()?;
    let title_obj = info_dict.get(b"Title").ok()?;
    object_to_string(title_obj)
}

/// Extract optional Author from PDF Info dictionary.
fn extract_pdf_author(doc: &lopdf::Document) -> Option<String> {
    let info_ref = doc.trailer.get(b"Info").ok()?.as_reference().ok()?;
    let info_dict = doc.get_dictionary(info_ref).ok()?;
    let author_obj = info_dict.get(b"Author").ok()?;
    object_to_string(author_obj)
}

/// Helper to generate current UTC ISO-8601 string without heavy runtime dependencies.
fn chrono_timestamp_iso8601() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}-01-01T00:00:00Z", 1970 + duration.as_secs() / 31_536_000)
}

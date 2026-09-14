//! Robust PDF parsing, page extraction, and outline discovery using `lopdf`.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, warn};

use super::model::{Document, DocumentId, DocumentMetadata, Page, SectionNode};
use super::structure::{infer_sections_from_pages, slugify_title};

/// Load and parse a PDF document from filesystem path into a structured `Document`.
pub fn load_pdf_from_path(path: impl AsRef<Path>) -> Result<Document> {
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
    let pdf_doc = lopdf::Document::load_mem(&bytes)
        .with_context(|| format!("Failed to parse PDF binary structure: {}", path.display()))?;

    // Extract pages in sequential order and build reverse mapping (ObjectId -> PageNumber)
    let (page_numbers, page_map) = {
        let pages_map = pdf_doc.get_pages();
        let mut nums: Vec<u32> = pages_map.keys().copied().collect();
        nums.sort_unstable();

        let mut reverse_map = HashMap::with_capacity(pages_map.len());
        for (page_num, obj_id) in &pages_map {
            reverse_map.insert(*obj_id, *page_num);
        }
        (nums, reverse_map)
    };

    let total_pages = page_numbers.len() as u32;
    info!(target: "parser", total_pages = total_pages, "Extracting text from pages");

    let mut pages: Vec<Page> = Vec::with_capacity(page_numbers.len());
    for page_num in page_numbers {
        let text = match pdf_doc.extract_text(&[page_num]) {
            Ok(extracted) => extracted,
            Err(err) => {
                warn!(target: "parser", page = page_num, error = %err, "Failed to extract text for page; recording as empty");
                String::new()
            }
        };

        pages.push(Page {
            page_number: page_num,
            char_count: text.chars().count(),
            text,
        });
    }

    // Attempt to extract PDF outline / bookmarks from catalog with exact page destinations
    let mut sections = extract_pdf_outlines(&pdf_doc, &page_map);

    // If PDF has no bookmarks, infer sections using typographic heuristics
    if sections.is_empty() {
        debug!(target: "parser", "No native PDF outlines detected; applying typographic heuristics");
        sections = infer_sections_from_pages(&pages);
    } else {
        reconcile_section_pages_and_previews(&mut sections, &pages, total_pages);
    }

    // Determine document title from file stem or metadata
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");

    let title = extract_pdf_title(&pdf_doc).unwrap_or_else(|| file_stem.to_string());
    let author = extract_pdf_author(&pdf_doc);
    let doc_id = DocumentId(slugify_title(&title));
    let total_sections = sections.iter().map(|s| s.total_count()).sum::<usize>() as u32;

    let metadata = DocumentMetadata {
        id: doc_id.0.clone(),
        title,
        author,
        total_pages,
        total_sections,
        file_size_bytes,
        content_hash: sha256_hash,
        indexed_at: chrono_timestamp_iso8601(),
    };

    info!(
        target: "parser",
        doc_id = %doc_id,
        sections = sections.len(),
        pages = pages.len(),
        "PDF ingestion and structure generation complete"
    );

    Ok(Document {
        id: doc_id,
        metadata,
        pages,
        sections,
    })
}

/// Extract native PDF bookmarks / outlines if present in the document catalog.
fn extract_pdf_outlines(
    doc: &lopdf::Document,
    page_map: &HashMap<(u32, u16), u32>,
) -> Vec<SectionNode> {
    let mut sections = Vec::new();

    // Look for Outlines in Document Catalog
    let Ok(trailer_root) = doc.trailer.get(b"Root") else {
        return sections;
    };

    let Ok(root_dict) = doc.get_dictionary(trailer_root.as_reference().unwrap_or((0, 0))) else {
        return sections;
    };

    let Ok(outlines_ref) = root_dict.get(b"Outlines") else {
        return sections;
    };

    let Ok(outlines_dict) = doc.get_dictionary(outlines_ref.as_reference().unwrap_or((0, 0)))
    else {
        return sections;
    };

    // Traverse First outline item
    if let Some(item_id) = outlines_dict
        .get(b"First")
        .ok()
        .and_then(|r| r.as_reference().ok())
    {
        traverse_outline_items(doc, item_id, 1, None, page_map, &mut sections);
    }

    sections
}

/// Recursively traverse outline items following Next and First links.
fn traverse_outline_items(
    doc: &lopdf::Document,
    item_id: (u32, u16),
    level: u32,
    parent_id: Option<String>,
    page_map: &HashMap<(u32, u16), u32>,
    acc: &mut Vec<SectionNode>,
) {
    let Ok(item_dict) = doc.get_dictionary(item_id) else {
        return;
    };

    let title = item_dict
        .get(b"Title")
        .ok()
        .and_then(object_to_string)
        .unwrap_or_else(|| "Untitled Section".to_string());

    let page_target = resolve_outline_page(doc, item_dict, page_map).unwrap_or(1);
    let sec_id = format!("{}-p{}", slugify_title(&title), page_target);

    let mut node = SectionNode {
        id: sec_id.clone(),
        title,
        level,
        page_start: page_target,
        page_end: page_target,
        parent_id,
        children: Vec::new(),
        content_preview: String::new(),
    };

    // Traverse child items if any
    if let Some(child_id) = item_dict
        .get(b"First")
        .ok()
        .and_then(|r| r.as_reference().ok())
    {
        traverse_outline_items(
            doc,
            child_id,
            level + 1,
            Some(sec_id),
            page_map,
            &mut node.children,
        );
    }

    acc.push(node);

    // Traverse sibling items
    if let Some(next_id) = item_dict
        .get(b"Next")
        .ok()
        .and_then(|r| r.as_reference().ok())
    {
        traverse_outline_items(doc, next_id, level, None, page_map, acc);
    }
}

/// Resolve the destination page of an outline item via /Dest or /A (Action GoTo).
fn extract_target_page(
    doc: &lopdf::Document,
    dest_obj: &lopdf::Object,
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    // 1. Direct array destination: [page_ref, /XYZ, left, top, zoom]
    if let Ok(arr) = dest_obj.as_array() {
        let target_ref = arr.first().and_then(|o| o.as_reference().ok());
        if let Some(target_ref) = target_ref {
            return page_map.get(&target_ref).copied();
        }
    }

    // 2. Named destination as String or Name: check Dests in catalog or Names dictionary
    if let Ok(name_bytes) = dest_obj.as_name() {
        if let Some(num) = resolve_named_destination(doc, name_bytes, page_map) {
            return Some(num);
        }
    } else if let Ok(str_bytes) = dest_obj.as_str() {
        if let Some(num) = resolve_named_destination(doc, str_bytes, page_map) {
            return Some(num);
        }
        // Fallback: if string is a numeric page index like "0", "1", "15"
        if let Some(idx) = std::str::from_utf8(str_bytes)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        {
            return Some(idx + 1);
        }
    }

    None
}

/// Look up a destination name in the PDF Catalog /Names /Dests or Catalog /Dests.
fn resolve_named_destination(
    doc: &lopdf::Document,
    dest_name: &[u8],
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    let trailer_root = doc.trailer.get(b"Root").ok()?.as_reference().ok()?;
    let catalog = doc.get_dictionary(trailer_root).ok()?;

    // Catalog /Dests dictionary (PDF 1.1)
    if let Ok(dests_obj) = catalog.get(b"Dests") {
        let dests_dict = match dests_obj {
            lopdf::Object::Dictionary(dict) => Some(dict),
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            _ => None,
        };
        if let Some(dict) = dests_dict {
            let p_num = dict
                .get(dest_name)
                .ok()
                .and_then(|v| v.as_array().ok())
                .and_then(|arr| arr.first().cloned())
                .and_then(|o| o.as_reference().ok())
                .and_then(|r| page_map.get(&r).copied());
            if p_num.is_some() {
                return p_num;
            }
        }
    }

    // Catalog /Names /Dests tree (PDF 1.2+)
    if let Ok(names_obj) = catalog.get(b"Names") {
        let names_dict = match names_obj {
            lopdf::Object::Dictionary(dict) => Some(dict),
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            _ => None,
        };
        if let Some(nd) = names_dict {
            let dests_node = nd.get(b"Dests").ok();
            let node_dict = match dests_node {
                Some(lopdf::Object::Dictionary(dict)) => Some(dict),
                Some(lopdf::Object::Reference(id)) => doc.get_dictionary(*id).ok(),
                _ => None,
            };
            if let Some(arr) = node_dict
                .and_then(|d| d.get(b"Names").ok())
                .and_then(|o| o.as_array().ok())
            {
                for chunk in arr.chunks(2) {
                    if chunk.len() < 2 {
                        continue;
                    }
                    let matches_key = chunk[0].as_str().map(|k| k == dest_name).unwrap_or(false);
                    if matches_key {
                        let p_num = chunk[1]
                            .as_array()
                            .ok()
                            .and_then(|arr| arr.first().cloned())
                            .and_then(|o| o.as_reference().ok())
                            .and_then(|r| page_map.get(&r).copied());
                        if p_num.is_some() {
                            return p_num;
                        }
                    }
                }
            }
        }
    }

    None
}

fn resolve_outline_page(
    doc: &lopdf::Document,
    item_dict: &lopdf::Dictionary,
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    // 1. Direct /Dest
    if let Ok(dest) = item_dict.get(b"Dest") {
        let found = extract_target_page(doc, dest, page_map);
        if found.is_some() {
            return found;
        }
    }

    // 2. Action dictionary /A -> /D (GoTo)
    if let Ok(action_obj) = item_dict.get(b"A") {
        let action_dict = match action_obj {
            lopdf::Object::Dictionary(dict) => Some(dict),
            lopdf::Object::Reference(ref_id) => doc.get_dictionary(*ref_id).ok(),
            _ => None,
        };

        if let Some(dest) = action_dict.and_then(|d| d.get(b"D").ok()) {
            let found = extract_target_page(doc, dest, page_map);
            if found.is_some() {
                return found;
            }
        }
    }

    None
}

/// Helper to extract string values from lopdf::Object, supporting UTF-16BE decoding.
fn object_to_string(obj: &lopdf::Object) -> Option<String> {
    match obj {
        lopdf::Object::String(bytes, _) => {
            let s = decode_pdf_string(bytes);
            if s.is_empty() { None } else { Some(s) }
        }
        lopdf::Object::Name(bytes) => {
            let s = decode_pdf_string(bytes);
            if s.is_empty() { None } else { Some(s) }
        }
        _ => None,
    }
}

/// Decode raw PDF string bytes handling UTF-16BE (with BOM \xFE\xFF) and UTF-8.
fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let mut u16_chars = Vec::with_capacity((bytes.len().saturating_sub(2)) / 2);
        let mut i = 2;
        while i + 1 < bytes.len() {
            u16_chars.push(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
        String::from_utf16_lossy(&u16_chars).trim().to_string()
    } else {
        String::from_utf8_lossy(bytes).trim().to_string()
    }
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

/// Reconcile section page ranges and populate content previews from extracted pages.
fn reconcile_section_pages_and_previews(
    sections: &mut [SectionNode],
    pages: &[Page],
    total_pages: u32,
) {
    if sections.is_empty() || pages.is_empty() {
        return;
    }

    let mut last_known_page = 1;
    for node in sections.iter_mut() {
        resolve_node_page_and_preview(node, pages, &mut last_known_page);
    }

    fix_page_ends(sections, total_pages);
}

fn resolve_node_page_and_preview(
    node: &mut SectionNode,
    pages: &[Page],
    last_known_page: &mut u32,
) {
    let title_clean = node.title.trim();
    if node.page_start <= 1 && title_clean.len() > 3 {
        let title_lower = title_clean.to_lowercase();
        let search_start_idx = (*last_known_page).saturating_sub(1) as usize;
        for page in pages.iter().skip(search_start_idx) {
            let page_lower = page.text.to_lowercase();
            if page_lower.contains(&title_lower) {
                node.page_start = page.page_number;
                node.page_end = page.page_number;
                *last_known_page = page.page_number;
                break;
            }
        }
    } else if node.page_start > *last_known_page {
        *last_known_page = node.page_start;
    }

    if node.content_preview.is_empty() && node.page_start >= 1 {
        let p_idx = (node.page_start - 1) as usize;
        if let Some(page) = pages.get(p_idx) {
            let preview: String = page
                .text
                .chars()
                .filter(|c| !c.is_control())
                .take(160)
                .collect();
            node.content_preview = preview.trim().replace('\n', " ");
        }
    }

    for child in &mut node.children {
        resolve_node_page_and_preview(child, pages, last_known_page);
    }
}

fn fix_page_ends(sections: &mut [SectionNode], parent_end: u32) {
    let len = sections.len();
    for i in 0..len {
        let next_start = if i + 1 < len {
            Some(sections[i + 1].page_start)
        } else {
            None
        };

        let node = &mut sections[i];
        if !node.children.is_empty() {
            let child_max_end = next_start.unwrap_or(parent_end);
            fix_page_ends(&mut node.children, child_max_end);
            if let Some(last_child) = node.children.last() {
                node.page_end = last_child.page_end.max(node.page_start);
            }
        } else if let Some(next_p) = next_start {
            node.page_end = next_p.saturating_sub(1).max(node.page_start);
        } else {
            node.page_end = parent_end.max(node.page_start);
        }
    }
}

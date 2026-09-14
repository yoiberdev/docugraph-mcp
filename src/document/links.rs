//! Extraction and resolution of PDF page annotations and hyperlinks (/Annots, /URI, /GoTo).

use std::collections::HashMap;
use tracing::debug;

use super::model::DocumentLink;

/// Extract all hyperlinks (external web links and internal cross-references) from a page's `/Annots` dictionary.
pub fn extract_page_links(
    doc: &lopdf::Document,
    page_dict: &lopdf::Dictionary,
    page_num: u32,
    page_map: &HashMap<(u32, u16), u32>,
) -> Vec<DocumentLink> {
    let mut links = Vec::new();

    let Ok(annots_obj) = page_dict.get(b"Annots") else {
        return links;
    };

    // Annots can be an inline array or an indirect reference to an array
    let annots_arr = match annots_obj {
        lopdf::Object::Array(arr) => Some(arr.as_slice()),
        lopdf::Object::Reference(id) => match doc.get_object(*id) {
            Ok(lopdf::Object::Array(arr)) => Some(arr.as_slice()),
            _ => None,
        },
        _ => None,
    };

    let Some(arr) = annots_arr else {
        return links;
    };

    for annot_ref in arr {
        // Annotation item can be a reference or an inline dictionary
        let annot_dict = match annot_ref {
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            lopdf::Object::Dictionary(dict) => Some(dict),
            _ => None,
        };

        let Some(dict) = annot_dict else {
            continue;
        };

        // Check Subtype: only interested in /Link annotations
        let is_link = match dict.get(b"Subtype") {
            Ok(lopdf::Object::Name(name)) => name == b"Link",
            _ => false,
        };

        if !is_link {
            continue;
        }

        let rect = extract_rect(dict);

        // 1. Check Action dictionary: /A
        if let Ok(action_obj) = dict.get(b"A") {
            let action_dict = match action_obj {
                lopdf::Object::Dictionary(d) => Some(d),
                lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
                _ => None,
            };

            if let Some(act) = action_dict
                && let Ok(s_name) = act.get(b"S").and_then(|s| s.as_name())
            {
                match s_name {
                    b"URI" => {
                        if let Some(uri_str) = act.get(b"URI").ok().and_then(object_to_string)
                            && !uri_str.trim().is_empty()
                        {
                            links.push(DocumentLink::uri(page_num, uri_str, rect));
                            continue;
                        }
                    }
                    b"GoTo" => {
                        if let Ok(dest_obj) = act.get(b"D") {
                            if let Some(target_p) = resolve_dest(doc, dest_obj, page_map) {
                                links.push(DocumentLink::internal(page_num, target_p, rect));
                                continue;
                            } else if let Some(named) = object_to_string(dest_obj) {
                                links.push(DocumentLink::named(page_num, named, rect));
                                continue;
                            }
                        }
                    }
                    _ => {
                        debug!(
                            target: "links",
                            page = page_num,
                            action_s = ?String::from_utf8_lossy(s_name),
                            "Unsupported or unhandled action subtype in /Link annotation"
                        );
                    }
                }
            }
        }

        // 2. Direct Destination /Dest (alternative standard representation in PDF 1.1+)
        if let Ok(dest_obj) = dict.get(b"Dest") {
            if let Some(target_p) = resolve_dest(doc, dest_obj, page_map) {
                links.push(DocumentLink::internal(page_num, target_p, rect));
            } else if let Some(named) = object_to_string(dest_obj) {
                links.push(DocumentLink::named(page_num, named, rect));
            }
        }
    }

    links
}

/// Extract bounding box rectangle `[x0, y0, x1, y1]` from an annotation dictionary.
fn extract_rect(dict: &lopdf::Dictionary) -> Option<[f32; 4]> {
    let arr = dict.get(b"Rect").ok()?.as_array().ok()?;
    if arr.len() == 4 {
        let x0 = object_to_f32(&arr[0])?;
        let y0 = object_to_f32(&arr[1])?;
        let x1 = object_to_f32(&arr[2])?;
        let y1 = object_to_f32(&arr[3])?;
        Some([x0, y0, x1, y1])
    } else {
        None
    }
}

/// Helper to convert a numeric PDF Object (Real or Integer) to f32.
pub(crate) fn object_to_f32(obj: &lopdf::Object) -> Option<f32> {
    match obj {
        lopdf::Object::Real(f) => Some(*f),
        lopdf::Object::Integer(i) => Some(*i as f32),
        _ => None,
    }
}

/// Resolve a destination object (Array, Reference, Name, or String) to a 1-based page number.
pub fn resolve_dest(
    doc: &lopdf::Document,
    dest_obj: &lopdf::Object,
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    match dest_obj {
        // 1. Direct array: [page_ref, /XYZ, left, top, zoom] or [page_ref, /Fit]
        lopdf::Object::Array(arr) => {
            if let Some(first) = arr.first() {
                if let Ok(target_ref) = first.as_reference() {
                    if let Some(p) = page_map.get(&target_ref) {
                        return Some(*p);
                    }
                } else if let Ok(idx) = first.as_i64() {
                    // 0-based page index fallback
                    return Some((idx as u32) + 1);
                }
            }
            None
        }
        // 2. Reference to an array or destination object
        lopdf::Object::Reference(ref_id) => {
            if let Ok(resolved_obj) = doc.get_object(*ref_id) {
                resolve_dest(doc, resolved_obj, page_map)
            } else {
                None
            }
        }
        // 3. Named destination as Name
        lopdf::Object::Name(name_bytes) => resolve_named_destination(doc, name_bytes, page_map),
        // 4. Named destination as String
        lopdf::Object::String(str_bytes, _) => {
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
            None
        }
        _ => None,
    }
}

/// Look up a destination name in the PDF Catalog `/Names /Dests` or Catalog `/Dests`.
pub fn resolve_named_destination(
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

/// Helper to extract string values from `lopdf::Object`, supporting UTF-16BE decoding.
pub fn object_to_string(obj: &lopdf::Object) -> Option<String> {
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

/// Decode raw PDF string bytes handling UTF-16BE (with BOM \xFE\xFF) and UTF-8 lossy.
pub fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let mut u16_chars = Vec::with_capacity(bytes.len().saturating_sub(2) / 2);
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

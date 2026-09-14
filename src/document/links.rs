//! Extraction and resolution of PDF page annotations and hyperlinks (/Annots, /URI, /GoTo).

use std::collections::{HashMap, HashSet};
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

/// Maximum nesting followed through destination references, name trees and number trees.
///
/// Guards against reference cycles and pathologically deep trees in malformed PDFs.
pub(crate) const MAX_TREE_DEPTH: usize = 32;

/// Resolve a destination object (Array, Reference, Dictionary, Name, or String) to a 1-based page number.
///
/// Named destinations are looked up in the catalog `/Dests` dictionary and in the `/Names /Dests`
/// name tree. A name that is not found resolves to `None`: a destination name is never a page
/// index, even when it looks numeric (LaTeX/hyperref writes names such as `(0)`, `(15)` or `(1a)`).
pub fn resolve_dest(
    doc: &lopdf::Document,
    dest_obj: &lopdf::Object,
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    resolve_dest_at_depth(doc, dest_obj, page_map, 0)
}

fn resolve_dest_at_depth(
    doc: &lopdf::Document,
    dest_obj: &lopdf::Object,
    page_map: &HashMap<(u32, u16), u32>,
    depth: usize,
) -> Option<u32> {
    if depth > MAX_TREE_DEPTH {
        return None;
    }
    match dest_obj {
        // 1. Direct array: [page_ref, /XYZ, left, top, zoom] or [page_ref, /Fit]
        lopdf::Object::Array(arr) => {
            let first = arr.first()?;
            if let Ok(target_ref) = first.as_reference() {
                page_map.get(&target_ref).copied()
            } else if let Ok(idx) = first.as_i64() {
                // 0-based page index fallback
                u32::try_from(idx).ok().and_then(|i| i.checked_add(1))
            } else {
                None
            }
        }
        // 2. Reference to an array or destination object
        lopdf::Object::Reference(ref_id) => doc
            .get_object(*ref_id)
            .ok()
            .and_then(|obj| resolve_dest_at_depth(doc, obj, page_map, depth + 1)),
        // 3. Destination dictionary, as stored in name trees: << /D [page_ref /XYZ ...] >>
        lopdf::Object::Dictionary(dict) => dict
            .get(b"D")
            .ok()
            .and_then(|d| resolve_dest_at_depth(doc, d, page_map, depth + 1)),
        // 4. Named destination as Name or String
        lopdf::Object::Name(name) | lopdf::Object::String(name, _) => {
            resolve_named_destination_at_depth(doc, name, page_map, depth + 1)
        }
        _ => None,
    }
}

/// Look up a destination name in the PDF Catalog `/Dests` dictionary or the `/Names /Dests` name tree.
pub fn resolve_named_destination(
    doc: &lopdf::Document,
    dest_name: &[u8],
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    resolve_named_destination_at_depth(doc, dest_name, page_map, 0)
}

fn resolve_named_destination_at_depth(
    doc: &lopdf::Document,
    dest_name: &[u8],
    page_map: &HashMap<(u32, u16), u32>,
    depth: usize,
) -> Option<u32> {
    if depth > MAX_TREE_DEPTH {
        return None;
    }
    let catalog = catalog_dictionary(doc)?;

    // Catalog /Dests dictionary (PDF 1.1)
    if let Some(value) = catalog
        .get(b"Dests")
        .ok()
        .and_then(|obj| dereference_dictionary(doc, obj))
        .and_then(|dests| dests.get(dest_name).ok())
        && let Some(page) = resolve_dest_at_depth(doc, value, page_map, depth + 1)
    {
        return Some(page);
    }

    // Catalog /Names /Dests name tree (PDF 1.2+)
    let tree = catalog
        .get(b"Names")
        .ok()
        .and_then(|obj| dereference_dictionary(doc, obj))
        .and_then(|names| names.get(b"Dests").ok())
        .and_then(|obj| dereference_dictionary(doc, obj))?;
    let value = lookup_name_tree(doc, tree, dest_name)?;
    resolve_dest_at_depth(doc, value, page_map, depth + 1)
}

/// Find the value stored under `key` in a PDF name tree (ISO 32000-1, 7.9.6).
///
/// Intermediate `/Kids` nodes are followed: producers such as LaTeX/hyperref split the tree once a
/// document has more than a handful of names. `/Limits` let the walk skip subtrees that cannot
/// hold the key; if that pruned walk misses (for instance because of wrong limits), the tree is
/// scanned once more without pruning.
pub fn lookup_name_tree<'a>(
    doc: &'a lopdf::Document,
    root: &'a lopdf::Dictionary,
    key: &[u8],
) -> Option<&'a lopdf::Object> {
    walk_name_tree(doc, root, key, true).or_else(|| walk_name_tree(doc, root, key, false))
}

fn walk_name_tree<'a>(
    doc: &'a lopdf::Document,
    root: &'a lopdf::Dictionary,
    key: &[u8],
    use_limits: bool,
) -> Option<&'a lopdf::Object> {
    let mut stack = vec![(root, 0usize)];
    let mut visited = HashSet::new();

    while let Some((node, depth)) = stack.pop() {
        if let Some(names) = node
            .get(b"Names")
            .ok()
            .and_then(|obj| dereference_array(doc, obj))
        {
            let (pairs, _) = names.as_chunks::<2>();
            for [name, value] in pairs {
                if name.as_str().is_ok_and(|name| name == key) {
                    return Some(value);
                }
            }
        }

        if depth >= MAX_TREE_DEPTH {
            continue;
        }
        let Some(kids) = node
            .get(b"Kids")
            .ok()
            .and_then(|obj| dereference_array(doc, obj))
        else {
            continue;
        };
        // Push in reverse so kids are visited in document (sorted) order
        for kid in kids.iter().rev() {
            if let Ok(kid_id) = kid.as_reference()
                && !visited.insert(kid_id)
            {
                continue;
            }
            let Some(kid_node) = dereference_dictionary(doc, kid) else {
                continue;
            };
            if use_limits && !name_tree_limits_contain(doc, kid_node, key) {
                continue;
            }
            stack.push((kid_node, depth + 1));
        }
    }

    None
}

/// Whether a name tree node's `/Limits [low high]` may contain `key` (true when limits are absent).
fn name_tree_limits_contain(doc: &lopdf::Document, node: &lopdf::Dictionary, key: &[u8]) -> bool {
    let Some(limits) = node
        .get(b"Limits")
        .ok()
        .and_then(|obj| dereference_array(doc, obj))
    else {
        return true;
    };
    match (
        limits.first().and_then(|o| o.as_str().ok()),
        limits.get(1).and_then(|o| o.as_str().ok()),
    ) {
        (Some(low), Some(high)) => low <= key && key <= high,
        _ => true,
    }
}

/// Return the document catalog (`/Root`) dictionary.
pub(crate) fn catalog_dictionary(doc: &lopdf::Document) -> Option<&lopdf::Dictionary> {
    let root = doc.trailer.get(b"Root").ok()?.as_reference().ok()?;
    doc.get_dictionary(root).ok()
}

/// Follow indirect references and return the dictionary they point to.
pub(crate) fn dereference_dictionary<'a>(
    doc: &'a lopdf::Document,
    obj: &'a lopdf::Object,
) -> Option<&'a lopdf::Dictionary> {
    doc.dereference(obj).ok()?.1.as_dict().ok()
}

/// Follow indirect references and return the array they point to.
pub(crate) fn dereference_array<'a>(
    doc: &'a lopdf::Document,
    obj: &'a lopdf::Object,
) -> Option<&'a [lopdf::Object]> {
    doc.dereference(obj)
        .ok()?
        .1
        .as_array()
        .ok()
        .map(Vec::as_slice)
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
    decode_pdf_text(bytes).trim().to_string()
}

/// Decode raw PDF string bytes like [`decode_pdf_string`], keeping surrounding whitespace.
pub(crate) fn decode_pdf_text(bytes: &[u8]) -> String {
    if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (pairs, _) = utf16.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|&pair| u16::from_be_bytes(pair)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

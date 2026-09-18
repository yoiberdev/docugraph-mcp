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
        let page = dests_dict
            .and_then(|d| d.get(dest_name).ok())
            .and_then(|val| resolve_dest_target(doc, val, page_map));
        if page.is_some() {
            return page;
        }
    }

    // Catalog /Names /Dests tree (PDF 1.2+)
    if let Ok(names_obj) = catalog.get(b"Names") {
        let names_dict = match names_obj {
            lopdf::Object::Dictionary(dict) => Some(dict),
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            _ => None,
        };
        let page = names_dict
            .and_then(|nd| nd.get(b"Dests").ok())
            .and_then(|dests_node| {
                find_named_dest_in_node(doc, dests_node, dest_name, page_map, 0)
            });
        if page.is_some() {
            return page;
        }
    }

    None
}

/// Recursively search a PDF Name Tree node (handling /Names chunks and /Kids) for a named destination.
fn find_named_dest_in_node(
    doc: &lopdf::Document,
    node_obj: &lopdf::Object,
    dest_name: &[u8],
    page_map: &HashMap<(u32, u16), u32>,
    depth: usize,
) -> Option<u32> {
    if depth > 12 {
        return None;
    }
    let dict = match node_obj {
        lopdf::Object::Dictionary(d) => Some(d),
        lopdf::Object::Reference(r) => doc.get_dictionary(*r).ok(),
        _ => None,
    }?;

    // Check leaf node: /Names [ key0 val0 key1 val1 ... ]
    if let Ok(names_obj) = dict.get(b"Names") {
        let arr = match names_obj {
            lopdf::Object::Array(a) => Some(a.as_slice()),
            lopdf::Object::Reference(r) => match doc.get_object(*r) {
                Ok(lopdf::Object::Array(a)) => Some(a.as_slice()),
                _ => None,
            },
            _ => None,
        };

        if let Some(arr) = arr {
            for chunk in arr.chunks(2) {
                if chunk.len() < 2 {
                    continue;
                }
                let key_matches = match &chunk[0] {
                    lopdf::Object::String(bytes, _) => bytes.as_slice() == dest_name,
                    lopdf::Object::Name(bytes) => bytes.as_slice() == dest_name,
                    _ => false,
                };
                if key_matches {
                    let page = resolve_dest_target(doc, &chunk[1], page_map);
                    if page.is_some() {
                        return page;
                    }
                }
            }
        }
    }

    // Check intermediate node: /Kids [ ref0 ref1 ... ]
    if let Ok(kids_obj) = dict.get(b"Kids") {
        let kids_arr = match kids_obj {
            lopdf::Object::Array(a) => Some(a.as_slice()),
            lopdf::Object::Reference(r) => match doc.get_object(*r) {
                Ok(lopdf::Object::Array(a)) => Some(a.as_slice()),
                _ => None,
            },
            _ => None,
        };

        if let Some(kids) = kids_arr {
            for kid in kids {
                let kid_obj = match kid {
                    lopdf::Object::Reference(r) => doc.get_object(*r).ok(),
                    _ => Some(kid),
                };
                let found = kid_obj.and_then(|k_obj| {
                    find_named_dest_in_node(doc, k_obj, dest_name, page_map, depth + 1)
                });
                if found.is_some() {
                    return found;
                }
            }
        }
    }

    None
}

/// Helper to resolve target destination value (Array, Reference, or Dictionary with /D).
/// How many levels of `/D` indirection a destination may hide behind.
///
/// A destination that points at a destination that points back at it is a cycle;
/// without a cap, resolving it recurses until the stack aborts the process.
const MAX_DEST_DEPTH: usize = 12;

fn resolve_dest_target(
    doc: &lopdf::Document,
    target: &lopdf::Object,
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    resolve_dest_target_depth(doc, target, page_map, 0)
}

fn resolve_dest_target_depth(
    doc: &lopdf::Document,
    target: &lopdf::Object,
    page_map: &HashMap<(u32, u16), u32>,
    depth: usize,
) -> Option<u32> {
    if depth > MAX_DEST_DEPTH {
        return None;
    }
    match target {
        lopdf::Object::Array(arr) => arr.first().and_then(|first| match first {
            lopdf::Object::Reference(r) => page_map.get(r).copied(),
            lopdf::Object::Integer(idx) => Some((*idx as u32) + 1),
            _ => None,
        }),
        lopdf::Object::Reference(r) => {
            if let Ok(obj) = doc.get_object(*r) {
                resolve_dest_target_depth(doc, obj, page_map, depth + 1)
            } else {
                None
            }
        }
        lopdf::Object::Dictionary(d) => {
            if let Ok(d_obj) = d.get(b"D") {
                resolve_dest_target_depth(doc, d_obj, page_map, depth + 1)
            } else {
                None
            }
        }
        _ => None,
    }
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

/// The code points where PDFDocEncoding departs from Latin-1, at `0x18..=0x1F` and
/// `0x80..=0xA0`. Everything below is ASCII and everything above is Latin-1.
///
/// Table D.2 of the PDF specification.
const PDF_DOC_ENCODING_HIGH: [char; 41] = [
    // 0x18..=0x1F: accents a producer places over a following letter.
    '\u{02D8}', '\u{02C7}', '\u{02C6}', '\u{02D9}', '\u{02DD}', '\u{02DB}', '\u{02DA}', '\u{02DC}',
    // 0x80..=0xA0: typography, ligatures and the currency sign.
    '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}', '\u{2044}',
    '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}', '\u{201D}', '\u{2018}',
    '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}', '\u{0141}', '\u{0152}', '\u{0160}',
    '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}', '\u{0153}', '\u{0161}', '\u{017E}', '\u{FFFD}',
    '\u{20AC}',
];

/// Decode bytes in what the specification calls PDFDocEncoding: the encoding every
/// PDF text string without a UTF-16 byte-order mark is written in.
fn decode_pdf_doc_encoding(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x18..=0x1F => PDF_DOC_ENCODING_HIGH[(b - 0x18) as usize],
            0x80..=0xA0 => PDF_DOC_ENCODING_HIGH[(b - 0x80) as usize + 8],
            // Latin-1 agrees with PDFDocEncoding over the rest of the range.
            _ => b as char,
        })
        .collect()
}

/// Decode a PDF text string: UTF-16 when a byte-order mark says so, UTF-8 when the
/// bytes are valid UTF-8, and PDFDocEncoding otherwise.
///
/// The PDFDocEncoding arm is what the specification mandates for a string without a
/// mark, and leaving it out was not a cosmetic gap. `from_utf8_lossy` turns every
/// byte an accent occupies into `U+FFFD`, because those bytes are not valid UTF-8:
/// `o` with an acute accent is the single byte `0xF3` here, which a UTF-8 decoder
/// reads as a continuation byte with nothing to continue. Measured on the
/// consolidated Spanish criminal code from the BOE, 207 pages: 946 of its 953
/// outline titles came back corrupted, so a document whose every heading carries an
/// accent was indexed under headings no query could match.
///
/// UTF-8 is tried before PDFDocEncoding because producers do emit it against the
/// specification, and the two are only ambiguous for byte sequences that are valid
/// UTF-8, which PDFDocEncoding would have written as single high bytes instead.
pub fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let mut u16_chars = Vec::with_capacity(bytes.len().saturating_sub(2) / 2);
        let mut i = 2;
        while i + 1 < bytes.len() {
            u16_chars.push(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
        String::from_utf16_lossy(&u16_chars).trim().to_string()
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        // Not in the specification, but produced in the wild often enough that
        // reading it as PDFDocEncoding would mangle every second byte.
        let mut u16_chars = Vec::with_capacity(bytes.len().saturating_sub(2) / 2);
        let mut i = 2;
        while i + 1 < bytes.len() {
            u16_chars.push(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
        String::from_utf16_lossy(&u16_chars).trim().to_string()
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).trim().to_string()
    } else if let Ok(utf8) = std::str::from_utf8(bytes) {
        utf8.trim().to_string()
    } else {
        decode_pdf_doc_encoding(bytes).trim().to_string()
    }
}

/// Read a value that may be stored as an indirect reference, following it once.
///
/// A dictionary entry the specification types as a string may still be written as a
/// reference to one, and a match that only accepts `Object::String` silently reads
/// such an entry as absent. Measured on the C++ working draft N4950, 2134 pages:
/// every one of its 3075 outline entries stores `/Title` as a reference, so all
/// 3075 sections were indexed as "Untitled Section" - which is also what every
/// citation drawn from that document called them.
pub fn resolve_to_string(doc: &lopdf::Document, obj: &lopdf::Object) -> Option<String> {
    let resolved = doc.dereference(obj).map(|(_, o)| o).unwrap_or(obj);
    object_to_string(resolved)
}

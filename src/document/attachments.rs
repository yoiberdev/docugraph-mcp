//! Extraction and decoding of PDF embedded files and attachments (/EmbeddedFiles, /AF, /FileAttachment).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::debug;

use super::links::object_to_string;
use super::model::EmbeddedAttachment;

/// Extract all embedded file attachments from a PDF document.
///
/// Discovers attachments from:
/// 1. Document-level Name Tree: `/Root /Names /EmbeddedFiles`
/// 2. PDF/A-3 and PDF 2.0 Associated Files: `/Root /AF`
/// 3. Page-level File Attachment Annotations: `/Page /Annots /Subtype /FileAttachment`
pub fn extract_document_attachments(
    doc: &lopdf::Document,
    page_map: &HashMap<(u32, u16), u32>,
) -> Vec<EmbeddedAttachment> {
    let mut attachments = Vec::new();
    let mut seen_stream_ids = HashSet::new();

    let Ok(trailer_root) = doc.trailer.get(b"Root") else {
        return attachments;
    };

    let Ok(catalog) = doc.get_dictionary(trailer_root.as_reference().unwrap_or((0, 0))) else {
        return attachments;
    };

    // 1. Traverse Document-level /Root /Names /EmbeddedFiles
    if let Ok(names_obj) = catalog.get(b"Names") {
        let names_dict = match names_obj {
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            lopdf::Object::Dictionary(d) => Some(d),
            _ => None,
        };

        if let Some(nd) = names_dict
            && let Ok(ef_obj) = nd.get(b"EmbeddedFiles")
        {
            traverse_name_tree(doc, ef_obj, &mut attachments, &mut seen_stream_ids);
        }
    }

    // 2. Traverse PDF/A-3 and PDF 2.0 Associated Files: /Root /AF
    if let Ok(af_obj) = catalog.get(b"AF") {
        let af_arr = match af_obj {
            lopdf::Object::Array(arr) => Some(arr.clone()),
            lopdf::Object::Reference(id) => {
                if let Ok(lopdf::Object::Array(arr)) = doc.get_object(*id) {
                    Some(arr.clone())
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(arr) = af_arr {
            for item in &arr {
                if let Some(att) = parse_filespec(doc, item, None, None, &mut seen_stream_ids) {
                    attachments.push(att);
                }
            }
        }
    }

    // 3. Traverse Page-level /Annots with /Subtype /FileAttachment
    for (page_id, &page_num) in page_map {
        let Ok(page_dict) = doc.get_dictionary(*page_id) else {
            continue;
        };

        let Ok(annots_obj) = page_dict.get(b"Annots") else {
            continue;
        };

        let annots_arr = match annots_obj {
            lopdf::Object::Array(arr) => Some(arr.clone()),
            lopdf::Object::Reference(id) => {
                if let Ok(lopdf::Object::Array(arr)) = doc.get_object(*id) {
                    Some(arr.clone())
                } else {
                    None
                }
            }
            _ => None,
        };

        let Some(arr) = annots_arr else {
            continue;
        };

        for annot_ref in &arr {
            let annot_dict = match annot_ref {
                lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
                lopdf::Object::Dictionary(d) => Some(d),
                _ => None,
            };

            let Some(dict) = annot_dict else {
                continue;
            };

            let is_file_attachment = match dict.get(b"Subtype") {
                Ok(lopdf::Object::Name(name)) => name == b"FileAttachment",
                _ => false,
            };

            if !is_file_attachment {
                continue;
            }

            if let Ok(fs_obj) = dict.get(b"FS")
                && let Some(att) =
                    parse_filespec(doc, fs_obj, None, Some(page_num), &mut seen_stream_ids)
            {
                attachments.push(att);
            }
        }
    }

    debug!(
        target: "attachments",
        total = attachments.len(),
        "Embedded PDF file attachments extraction completed"
    );

    attachments
}

/// How deep an /EmbeddedFiles name tree may nest before we stop following it.
///
/// Name trees are balanced and shallow; the cap is here because a `/Kids` entry
/// pointing back at an ancestor would otherwise recurse until the stack aborts
/// the process, which no error handling above can contain.
const MAX_NAME_TREE_DEPTH: usize = 32;

fn traverse_name_tree(
    doc: &lopdf::Document,
    node_obj: &lopdf::Object,
    acc: &mut Vec<EmbeddedAttachment>,
    seen_stream_ids: &mut HashSet<(u32, u16)>,
) {
    let mut visited_nodes = HashSet::new();
    traverse_name_tree_depth(doc, node_obj, acc, seen_stream_ids, &mut visited_nodes, 0);
}

fn traverse_name_tree_depth(
    doc: &lopdf::Document,
    node_obj: &lopdf::Object,
    acc: &mut Vec<EmbeddedAttachment>,
    seen_stream_ids: &mut HashSet<(u32, u16)>,
    visited_nodes: &mut HashSet<(u32, u16)>,
    depth: usize,
) {
    if depth > MAX_NAME_TREE_DEPTH {
        return;
    }
    if let lopdf::Object::Reference(id) = node_obj
        && !visited_nodes.insert(*id)
    {
        return;
    }
    let dict = match node_obj {
        lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
        lopdf::Object::Dictionary(d) => Some(d),
        _ => None,
    };

    let Some(dict) = dict else {
        return;
    };

    // Leaf node: contains /Names [ (name_0) (filespec_0) ... ]
    if let Ok(names_obj) = dict.get(b"Names") {
        let names_arr = match names_obj {
            lopdf::Object::Array(arr) => Some(arr.as_slice()),
            lopdf::Object::Reference(id) => match doc.get_object(*id) {
                Ok(lopdf::Object::Array(arr)) => Some(arr.as_slice()),
                _ => None,
            },
            _ => None,
        };

        if let Some(arr) = names_arr {
            // The name tree stores alternating (name, filespec) entries.
            for chunk in arr.as_chunks::<2>().0 {
                let name_hint = object_to_string(&chunk[0]);
                let filespec_obj = &chunk[1];
                if let Some(att) =
                    parse_filespec(doc, filespec_obj, name_hint, None, seen_stream_ids)
                {
                    acc.push(att);
                }
            }
        }
    }

    // Intermediate node: contains /Kids [ ref_node1, ref_node2, ... ]
    if let Ok(kids_obj) = dict.get(b"Kids") {
        let kids_arr = match kids_obj {
            lopdf::Object::Array(arr) => Some(arr.clone()),
            lopdf::Object::Reference(id) => {
                if let Ok(lopdf::Object::Array(arr)) = doc.get_object(*id) {
                    Some(arr.clone())
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(arr) = kids_arr {
            for kid in &arr {
                traverse_name_tree_depth(doc, kid, acc, seen_stream_ids, visited_nodes, depth + 1);
            }
        }
    }
}

/// Parse a `/Filespec` dictionary or reference into an `EmbeddedAttachment`.
fn parse_filespec(
    doc: &lopdf::Document,
    filespec_obj: &lopdf::Object,
    name_hint: Option<String>,
    page_number: Option<u32>,
    seen_stream_ids: &mut HashSet<(u32, u16)>,
) -> Option<EmbeddedAttachment> {
    let dict = match filespec_obj {
        lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
        lopdf::Object::Dictionary(d) => Some(d),
        _ => None,
    };

    let dict = dict?;

    // Filename: prefer /UF (Unicode), fallback to /F, then name_hint
    let filename = dict
        .get(b"UF")
        .ok()
        .and_then(object_to_string)
        .or_else(|| dict.get(b"F").ok().and_then(object_to_string))
        .or(name_hint)
        .unwrap_or_else(|| "attachment.bin".to_string());

    // Description /Desc
    let description = dict.get(b"Desc").ok().and_then(object_to_string);

    // Embedded file stream reference: /EF << /UF (ref) or /F (ref) >>
    let ef_dict = dict.get(b"EF").ok().and_then(|ef| match ef {
        lopdf::Object::Dictionary(d) => Some(d),
        lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    });

    let ef_dict = ef_dict?;

    let stream_id = ef_dict
        .get(b"UF")
        .ok()
        .and_then(|o| o.as_reference().ok())
        .or_else(|| ef_dict.get(b"F").ok().and_then(|o| o.as_reference().ok()))?;

    // Avoid duplicate parsing of the same embedded stream
    if !seen_stream_ids.insert(stream_id) {
        return None;
    }

    let stream_obj = doc.get_object(stream_id).ok()?;
    let lopdf::Object::Stream(ref stream) = *stream_obj else {
        return None;
    };

    // Subtype /Subtype in stream dictionary (e.g. text#2Fxml -> text/xml)
    let mime_type = stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(|name| {
            let s = String::from_utf8_lossy(name);
            s.replace("#2F", "/")
        });

    // Decompress stream content
    let data = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());

    // Params dictionary: /Size, /ModDate, /CheckSum
    let mut size_bytes = data.len() as u64;
    let mut mod_date = None;
    let mut checksum_md5 = None;

    if let Ok(params_obj) = stream.dict.get(b"Params") {
        let params_dict = match params_obj {
            lopdf::Object::Dictionary(d) => Some(d),
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            _ => None,
        };

        if let Some(pd) = params_dict {
            if let Ok(size_obj) = pd.get(b"Size")
                && let Ok(s) = size_obj.as_i64()
                && s > 0
            {
                size_bytes = s as u64;
            }

            if let Ok(m_obj) = pd.get(b"ModDate") {
                mod_date = object_to_string(m_obj);
            }

            if let Ok(cs_obj) = pd.get(b"CheckSum")
                && let Ok(cs_bytes) = cs_obj.as_str()
            {
                checksum_md5 = Some(hex::encode(cs_bytes));
            }
        }
    }

    let is_text = std::str::from_utf8(&data).is_ok();
    let id = format!("att_{}", sanitize_id(&filename));

    Some(EmbeddedAttachment {
        id,
        filename,
        description,
        mime_type,
        size_bytes,
        checksum_md5,
        mod_date,
        is_text,
        page_number,
        data,
    })
}

fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A file name safe to join onto an output directory, or `None` when the
/// attachment does not supply a usable one.
///
/// An attachment's name comes from the PDF's `/UF` or `/F`, which is written by
/// whoever produced the file. Joining it to a destination directly let a document
/// choose where its own bytes landed: verified against the CLI, an attachment
/// named `../../ESCAPED_pwned.txt` wrote two directories above `--extract-dir`
/// while the command reported success into that directory, and an absolute name
/// ignored the destination entirely.
///
/// `Path::file_name` is what does the work - it discards every parent component,
/// every separator and any drive prefix, and yields nothing at all for `.` or
/// `..`. The rest is a guard against a platform surprising us.
pub fn safe_output_name(filename: &str) -> Option<String> {
    let name = Path::new(filename).file_name()?.to_str()?;
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if name.contains(['/', '\\', ':']) {
        return None;
    }
    Some(name.to_string())
}

/// Resolve where an attachment may be written under `dir`, refusing anything that
/// would land outside it.
///
/// The name is reduced to a bare file name first, so escaping is already
/// impossible; this then confirms it, which is what `SECURITY.md` promises and
/// what makes a symlinked destination safe too.
pub fn safe_output_path(dir: &Path, filename: &str) -> Result<PathBuf, String> {
    let name = safe_output_name(filename).ok_or_else(|| {
        format!("Attachment name {filename:?} is not a usable file name; refusing to write it.")
    })?;

    let base = std::fs::canonicalize(dir)
        .map_err(|e| format!("Cannot resolve output directory {}: {e}", dir.display()))?;
    let target = base.join(&name);

    if !target.starts_with(&base) {
        return Err(format!(
            "Attachment {filename:?} would be written outside {}; refusing.",
            dir.display()
        ));
    }
    Ok(target)
}

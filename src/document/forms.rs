//! Interactive AcroForm field discovery, hierarchical extraction, and value resolution.

use std::collections::HashMap;
use tracing::debug;

use super::links::{decode_pdf_string, object_to_f32, object_to_string};
use super::model::{FormField, FormFieldType};

/// Extract all interactive form fields (/AcroForm) from a PDF document.
pub fn extract_document_forms(
    doc: &lopdf::Document,
    page_map: &HashMap<(u32, u16), u32>,
) -> Vec<FormField> {
    let mut fields = Vec::new();

    let Ok(trailer_root) = doc.trailer.get(b"Root") else {
        return fields;
    };

    let Ok(catalog) = doc.get_dictionary(trailer_root.as_reference().unwrap_or((0, 0))) else {
        return fields;
    };

    let Ok(acro_form_obj) = catalog.get(b"AcroForm") else {
        return fields;
    };

    let acro_form_dict = match acro_form_obj {
        lopdf::Object::Dictionary(d) => Some(d),
        lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    };

    let Some(acro_dict) = acro_form_dict else {
        return fields;
    };

    let Ok(fields_obj) = acro_dict.get(b"Fields") else {
        return fields;
    };

    let fields_arr = match fields_obj {
        lopdf::Object::Array(arr) => Some(arr.as_slice()),
        lopdf::Object::Reference(id) => match doc.get_object(*id) {
            Ok(lopdf::Object::Array(arr)) => Some(arr.as_slice()),
            _ => None,
        },
        _ => None,
    };

    let Some(arr) = fields_arr else {
        return fields;
    };

    // Build reverse annotation map (widget_id -> page_num) in case /P is omitted in field dictionaries
    let annot_page_map = build_annot_page_map(doc, page_map);

    let ctx = FormContext {
        doc,
        page_map,
        annot_page_map: &annot_page_map,
    };

    for field_ref in arr {
        traverse_field(&ctx, field_ref, "", None, 0, &mut fields);
    }

    fields
}

/// Context passed during recursive AcroForm field traversal.
struct FormContext<'a> {
    doc: &'a lopdf::Document,
    page_map: &'a HashMap<(u32, u16), u32>,
    annot_page_map: &'a HashMap<(u32, u16), u32>,
}

/// Recursively traverse a field object and its /Kids, inheriting parent attributes.
fn traverse_field(
    ctx: &FormContext<'_>,
    field_obj: &lopdf::Object,
    parent_name: &str,
    inherited_type: Option<&[u8]>,
    inherited_flags: u32,
    acc: &mut Vec<FormField>,
) {
    let field_id = field_obj.as_reference().ok();
    let dict = match field_obj {
        lopdf::Object::Reference(id) => ctx.doc.get_dictionary(*id).ok(),
        lopdf::Object::Dictionary(d) => Some(d),
        _ => None,
    };

    let Some(dict) = dict else {
        return;
    };

    // Partial field name /T
    let partial_name = dict
        .get(b"T")
        .ok()
        .and_then(object_to_string)
        .unwrap_or_default();

    let fully_qualified = if parent_name.is_empty() {
        partial_name.clone()
    } else if partial_name.is_empty() {
        parent_name.to_string()
    } else {
        format!("{}.{}", parent_name, partial_name)
    };

    // Field type /FT (inherited if not specified)
    let ft_bytes = dict
        .get(b"FT")
        .ok()
        .and_then(|o| o.as_name().ok())
        .or(inherited_type);

    // Flags /Ff (inherited bitwise OR / fallback)
    let flags = dict
        .get(b"Ff")
        .ok()
        .and_then(|f| f.as_i64().ok())
        .map(|f| f as u32)
        .unwrap_or(inherited_flags);

    // Check if this node has /Kids that represent sub-fields or widget annotations
    if dict.has(b"Kids") {
        let kids_arr = match dict.get(b"Kids") {
            Ok(lopdf::Object::Array(arr)) => Some(arr.clone()),
            Ok(lopdf::Object::Reference(id)) => {
                if let Ok(lopdf::Object::Array(arr)) = ctx.doc.get_object(*id) {
                    Some(arr.clone())
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(kids) = kids_arr {
            let has_subfields = kids.iter().any(|k| {
                let k_dict = match k {
                    lopdf::Object::Reference(id) => ctx.doc.get_dictionary(*id).ok(),
                    lopdf::Object::Dictionary(d) => Some(d),
                    _ => None,
                };
                k_dict.map(|d| d.has(b"T")).unwrap_or(false)
            });

            if has_subfields {
                // Hierarchical parent with subfields
                for kid in &kids {
                    traverse_field(ctx, kid, &fully_qualified, ft_bytes, flags, acc);
                }
                return;
            }
        }
    }

    // Leaf field: extract value, flags, page, and rect
    let field_type = classify_field_type(ft_bytes, flags);

    // Current value /V
    let value = dict.get(b"V").ok().and_then(extract_field_value);

    // Default value /DV
    let default_value = dict.get(b"DV").ok().and_then(extract_field_value);

    // Permissions flags
    let read_only = (flags & 1) != 0;
    let required = (flags & 2) != 0;

    // Page resolution: check /P directly, then check widget annotations
    let mut page_number = dict
        .get(b"P")
        .ok()
        .and_then(|p| p.as_reference().ok())
        .and_then(|r| ctx.page_map.get(&r).copied());

    if page_number.is_none() {
        page_number = field_id.and_then(|id| ctx.annot_page_map.get(&id).copied());
    }

    // Rect coordinates
    let mut rect = extract_field_rect(dict);

    // If rect or page is missing, check first kid widget if any
    if (rect.is_none() || page_number.is_none())
        && let Ok(kids_arr) = dict.get(b"Kids").and_then(|k| k.as_array())
    {
        for kid in kids_arr {
            let kid_dict = match kid {
                lopdf::Object::Reference(id) => {
                    if page_number.is_none() {
                        page_number = ctx.annot_page_map.get(id).copied();
                    }
                    ctx.doc.get_dictionary(*id).ok()
                }
                lopdf::Object::Dictionary(d) => Some(d),
                _ => None,
            };

            if let Some(kd) = kid_dict {
                if page_number.is_none() {
                    page_number = kd
                        .get(b"P")
                        .ok()
                        .and_then(|p| p.as_reference().ok())
                        .and_then(|r| ctx.page_map.get(&r).copied());
                }
                if rect.is_none() {
                    rect = extract_field_rect(kd);
                }
                if page_number.is_some() && rect.is_some() {
                    break;
                }
            }
        }
    }

    let name = if partial_name.is_empty() {
        fully_qualified.clone()
    } else {
        partial_name
    };

    debug!(
        target: "forms",
        field = %fully_qualified,
        field_type = ?field_type,
        has_value = value.is_some(),
        "Discovered AcroForm field"
    );

    acc.push(FormField {
        name,
        fully_qualified_name: fully_qualified,
        field_type,
        value,
        default_value,
        read_only,
        required,
        page_number,
        rect,
    });
}

/// Classify a field's type based on /FT and /Ff flags.
fn classify_field_type(ft: Option<&[u8]>, flags: u32) -> FormFieldType {
    match ft {
        Some(b"Btn") => {
            let is_pushbutton = (flags & (1 << 16)) != 0;
            let is_radio = (flags & (1 << 15)) != 0;

            if is_pushbutton {
                FormFieldType::Button
            } else if is_radio {
                FormFieldType::Radio
            } else {
                FormFieldType::Checkbox
            }
        }
        Some(b"Tx") => FormFieldType::Text,
        Some(b"Ch") => FormFieldType::Choice,
        Some(b"Sig") => FormFieldType::Signature,
        Some(other) => FormFieldType::Unknown(String::from_utf8_lossy(other).to_string()),
        None => FormFieldType::Unknown("Unknown".to_string()),
    }
}

/// Extract string representation of field value /V or /DV.
fn extract_field_value(val_obj: &lopdf::Object) -> Option<String> {
    match val_obj {
        lopdf::Object::String(bytes, _) => {
            let s = decode_pdf_string(bytes);
            if s.is_empty() { None } else { Some(s) }
        }
        lopdf::Object::Name(bytes) => {
            let s = String::from_utf8_lossy(bytes).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        }
        lopdf::Object::Array(arr) => {
            let items: Vec<String> = arr.iter().filter_map(extract_field_value).collect();
            if items.is_empty() {
                None
            } else {
                Some(items.join(", "))
            }
        }
        lopdf::Object::Integer(i) => Some(i.to_string()),
        lopdf::Object::Real(f) => Some(format!("{:.2}", f)),
        lopdf::Object::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Extract bounding box rectangle from a field dictionary.
fn extract_field_rect(dict: &lopdf::Dictionary) -> Option<[f32; 4]> {
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

/// Build a map of annotation object ID -> 1-based page number by inspecting page /Annots.
fn build_annot_page_map(
    doc: &lopdf::Document,
    page_map: &HashMap<(u32, u16), u32>,
) -> HashMap<(u32, u16), u32> {
    let mut map = HashMap::new();

    for (page_id, page_num) in page_map {
        let Ok(page_dict) = doc.get_dictionary(*page_id) else {
            continue;
        };

        let Ok(annots_obj) = page_dict.get(b"Annots") else {
            continue;
        };

        let annots_arr = match annots_obj {
            lopdf::Object::Array(arr) => Some(arr.as_slice()),
            lopdf::Object::Reference(id) => match doc.get_object(*id) {
                Ok(lopdf::Object::Array(arr)) => Some(arr.as_slice()),
                _ => None,
            },
            _ => None,
        };

        if let Some(arr) = annots_arr {
            for item in arr {
                if let Ok(annot_id) = item.as_reference() {
                    map.insert(annot_id, *page_num);
                }
            }
        }
    }

    map
}

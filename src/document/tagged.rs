//! Discovery and structural verification of Tagged PDF (/StructTreeRoot, /MarkInfo).

use tracing::debug;

/// Result of inspecting a PDF for Tagged PDF / PDF/UA structure tree elements.
#[derive(Debug, Clone, Default)]
pub struct TaggedPdfInfo {
    /// Whether the document contains a valid /StructTreeRoot or /MarkInfo /Marked flag
    pub is_tagged: bool,
    /// Whether /StructTreeRoot is present in catalog
    pub struct_tree_root_present: bool,
    /// Whether /MarkInfo << /Marked true >> is present
    pub mark_info_marked: bool,
    /// Total count of root-level or top-level structural elements discovered
    pub root_elements_count: usize,
    /// Whether a custom /RoleMap dictionary is defined
    pub has_role_map: bool,
}

/// Detect whether the given PDF document contains a Tagged PDF logical structure tree.
pub fn detect_tagged_pdf_structure(doc: &lopdf::Document) -> TaggedPdfInfo {
    let mut info = TaggedPdfInfo::default();

    let Ok(trailer_root) = doc.trailer.get(b"Root") else {
        return info;
    };

    let Ok(catalog) = doc.get_dictionary(trailer_root.as_reference().unwrap_or((0, 0))) else {
        return info;
    };

    // 1. Check /MarkInfo dictionary for /Marked true
    let is_marked = catalog
        .get(b"MarkInfo")
        .ok()
        .and_then(|obj| match obj {
            lopdf::Object::Dictionary(d) => Some(d),
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            _ => None,
        })
        .and_then(|d| d.get(b"Marked").ok())
        .and_then(|m| m.as_bool().ok())
        .unwrap_or(false);

    info.mark_info_marked = is_marked;

    // 2. Check /StructTreeRoot dictionary
    let struct_tree_obj = catalog.get(b"StructTreeRoot").ok();
    let struct_tree_dict = struct_tree_obj.and_then(|obj| match obj {
        lopdf::Object::Dictionary(d) => Some(d),
        lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    });

    if let Some(tree_dict) = struct_tree_dict {
        info.is_tagged = true;
        info.struct_tree_root_present = true;

        // Check for RoleMap
        info.has_role_map = tree_dict.has(b"RoleMap");

        // Count kids in /K
        if let Ok(k_obj) = tree_dict.get(b"K") {
            info.root_elements_count = match k_obj {
                lopdf::Object::Array(arr) => arr.len(),
                lopdf::Object::Reference(id) => {
                    if let Ok(lopdf::Object::Array(arr)) = doc.get_object(*id) {
                        arr.len()
                    } else {
                        1
                    }
                }
                lopdf::Object::Dictionary(_) => 1,
                _ => 0,
            };
        }

        debug!(
            target: "tagged_pdf",
            root_elements = info.root_elements_count,
            has_role_map = info.has_role_map,
            "Tagged PDF structure tree discovered (/StructTreeRoot)"
        );
    } else if is_marked {
        info.is_tagged = true;
        debug!(target: "tagged_pdf", "Document marked as Tagged via /MarkInfo");
    }

    info
}

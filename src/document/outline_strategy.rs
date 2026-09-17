//! Outline and bookmarks extraction strategy implementations (GoF Strategy Pattern).

use std::collections::{HashMap, HashSet};
use tracing::debug;

use super::links::{object_to_string, resolve_dest};
use super::model::{Page, SectionNode};
use super::structure::{infer_sections_from_pages, slugify_title};

/// Strategy interface for extracting or inferring hierarchical document sections.
pub trait OutlineExtractor: Send + Sync {
    /// Extract or infer hierarchical section nodes from document structures.
    fn extract(
        &self,
        doc: &lopdf::Document,
        page_map: &HashMap<(u32, u16), u32>,
        pages: &[Page],
        total_pages: u32,
    ) -> Vec<SectionNode>;
}

/// Strategy 1: Extracts native PDF outlines / bookmarks from the Document Catalog (`/Root /Outlines`).
pub struct NativeOutlineExtractor;

impl OutlineExtractor for NativeOutlineExtractor {
    fn extract(
        &self,
        doc: &lopdf::Document,
        page_map: &HashMap<(u32, u16), u32>,
        _pages: &[Page],
        _total_pages: u32,
    ) -> Vec<SectionNode> {
        let mut sections = Vec::new();

        // Look for Outlines in Document Catalog
        let Ok(trailer_root) = doc.trailer.get(b"Root") else {
            return sections;
        };

        let Ok(root_dict) = doc.get_dictionary(trailer_root.as_reference().unwrap_or((0, 0)))
        else {
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
            let walk = OutlineWalk { doc, page_map };
            let mut visited = HashSet::new();
            traverse_outline_items(&walk, item_id, 1, None, &mut sections, &mut visited, 0);
        }

        sections
    }
}

/// Strategy 2: Infers section hierarchy from text typography, heading patterns, and numbering.
pub struct TypographicOutlineExtractor;

impl OutlineExtractor for TypographicOutlineExtractor {
    fn extract(
        &self,
        _doc: &lopdf::Document,
        _page_map: &HashMap<(u32, u16), u32>,
        pages: &[Page],
        _total_pages: u32,
    ) -> Vec<SectionNode> {
        infer_sections_from_pages(pages)
    }
}

/// Composite fallback strategy: attempts native outline extraction first, falling back to
/// typographic heuristics if the document lacks native bookmarks.
pub struct FallbackOutlineStrategy {
    primary: Box<dyn OutlineExtractor>,
    fallback: Box<dyn OutlineExtractor>,
}

impl FallbackOutlineStrategy {
    /// Create a standard fallback strategy with Native as primary and Typographic as fallback.
    pub fn new() -> Self {
        Self {
            primary: Box::new(NativeOutlineExtractor),
            fallback: Box::new(TypographicOutlineExtractor),
        }
    }

    /// Construct a strategy with custom primary and fallback extractors.
    pub fn custom(primary: Box<dyn OutlineExtractor>, fallback: Box<dyn OutlineExtractor>) -> Self {
        Self { primary, fallback }
    }
}

impl Default for FallbackOutlineStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlineExtractor for FallbackOutlineStrategy {
    fn extract(
        &self,
        doc: &lopdf::Document,
        page_map: &HashMap<(u32, u16), u32>,
        pages: &[Page],
        total_pages: u32,
    ) -> Vec<SectionNode> {
        let mut sections = self.primary.extract(doc, page_map, pages, total_pages);

        if sections.is_empty() {
            debug!(
                target: "outline_strategy",
                "No native PDF outlines detected; applying typographic fallback strategy"
            );
            sections = self.fallback.extract(doc, page_map, pages, total_pages);
        } else {
            reconcile_section_pages_and_previews(&mut sections, pages, total_pages);
        }

        sections
    }
}

/// Recursively traverse outline items following Next and First links.
/// How deep an outline may nest before we stop following it.
///
/// Real outlines are a handful of levels; this is only here so that a malicious
/// or broken `/First` chain cannot recurse until the stack gives out. A stack
/// overflow aborts the process, so `anyhow` and the per-file error handling in
/// `docugraph index <dir>` cannot contain it: one hostile PDF would take the
/// whole batch down with it, including the files queued behind it.
const MAX_OUTLINE_DEPTH: usize = 32;

/// What every step of the outline walk needs but none of it changes.
struct OutlineWalk<'a> {
    doc: &'a lopdf::Document,
    page_map: &'a HashMap<(u32, u16), u32>,
}

fn traverse_outline_items(
    walk: &OutlineWalk<'_>,
    first_id: (u32, u16),
    level: u32,
    parent_id: Option<String>,
    acc: &mut Vec<SectionNode>,
    visited: &mut HashSet<(u32, u16)>,
    depth: usize,
) {
    if depth > MAX_OUTLINE_DEPTH {
        return;
    }

    // `/Next` is a flat chain, so it is walked rather than recursed into: a
    // document with a few thousand top-level bookmarks is unremarkable, and
    // recursing once per sibling overflows the stack at around two thousand.
    let mut current = Some(first_id);
    while let Some(item_id) = current {
        // A `/First` or `/Next` that points back at an item already seen is a
        // cycle; without this it is an unrecoverable abort.
        if !visited.insert(item_id) {
            return;
        }
        current =
            traverse_one_outline_item(walk, item_id, level, parent_id.clone(), acc, visited, depth);
    }
}

/// Handle a single outline item, returning the sibling that follows it.
fn traverse_one_outline_item(
    walk: &OutlineWalk<'_>,
    item_id: (u32, u16),
    level: u32,
    parent_id: Option<String>,
    acc: &mut Vec<SectionNode>,
    visited: &mut HashSet<(u32, u16)>,
    depth: usize,
) -> Option<(u32, u16)> {
    let Ok(item_dict) = walk.doc.get_dictionary(item_id) else {
        return None;
    };

    let title = item_dict
        .get(b"Title")
        .ok()
        .and_then(object_to_string)
        .unwrap_or_else(|| "Untitled Section".to_string());

    let page_target = resolve_outline_page(walk.doc, item_dict, walk.page_map).unwrap_or(1);
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
            walk,
            child_id,
            level + 1,
            Some(sec_id),
            &mut node.children,
            visited,
            depth + 1,
        );
    }

    acc.push(node);

    // The sibling that follows, for the caller's loop to continue with. Siblings
    // keep the same parent: recursing with `None` here used to leave every
    // bookmark but the first child of a node without a parent_id, so
    // `document_get_section(include_parent: true)` returned no parent context for
    // most sections of a bookmarked PDF.
    item_dict
        .get(b"Next")
        .ok()
        .and_then(|r| r.as_reference().ok())
}

/// Resolve the destination page of an outline item via /Dest or /A (Action GoTo).
fn resolve_outline_page(
    doc: &lopdf::Document,
    item_dict: &lopdf::Dictionary,
    page_map: &HashMap<(u32, u16), u32>,
) -> Option<u32> {
    // 1. Direct /Dest
    if let Ok(dest) = item_dict.get(b"Dest") {
        let found = resolve_dest(doc, dest, page_map);
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
            let found = resolve_dest(doc, dest, page_map);
            if found.is_some() {
                return found;
            }
        }
    }

    None
}

/// Reconcile section page ranges and populate content previews from extracted pages.
pub fn reconcile_section_pages_and_previews(
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
                node.id = format!("{}-p{}", slugify_title(&node.title), node.page_start);
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

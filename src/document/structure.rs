//! Structural and typographic heuristics for inferring headings and document hierarchy.

use super::model::{Page, SectionNode};

/// Detect whether a trimmed line of text exhibits characteristics of a document heading.
///
/// Heuristics applied:
/// 1. Length constraint: Headings are concise (< 120 characters).
/// 2. Not standard sentence punctuation: Headings do not end in periods, commas, or semicolons.
/// 3. Structural markers:
///    - Numbered patterns: `1. `, `1.1 `, `1.1.1 `, `Chapter `, `Capítulo `
///    - Markdown-style `# ` prefixes (if pre-processed)
///    - ALL-CAPS uppercase words of significant length
pub fn detect_heading_level(line: &str) -> Option<(u32, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.len() > 120 {
        return None;
    }

    // Exclude regular prose sentences ending in full stops or commas
    if trimmed.ends_with('.')
        && !trimmed.ends_with("...")
        && !trimmed.chars().take(5).any(|c| c.is_ascii_digit())
    {
        return None;
    }
    if trimmed.ends_with(',') || trimmed.ends_with(';') {
        return None;
    }

    // Markdown syntax detection
    if let Some(rest) = trimmed.strip_prefix("### ") {
        return Some((3, rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("## ") {
        return Some((2, rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("# ") {
        return Some((1, rest.trim()));
    }

    // Explicit Chapter keywords
    let lower = trimmed.to_lowercase();
    if lower.starts_with("chapter ")
        || lower.starts_with("capítulo ")
        || lower.starts_with("sección ")
    {
        return Some((1, trimmed));
    }

    // Numbered hierarchical patterns: "1.2.3 Title"
    if let Some((digits, rest)) = split_numbering_prefix(trimmed) {
        let dots = digits.matches('.').count();
        let level = match dots {
            0 | 1 => 1,
            2 => 2,
            _ => 3,
        };
        return Some((level, rest));
    }

    // All-caps short titles (e.g., "INTENT", "APPLICABILITY", "CONSEQUENCES")
    let letters: String = trimmed.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() >= 4 && letters.chars().all(|c| c.is_uppercase()) && trimmed.len() <= 60 {
        return Some((2, trimmed));
    }

    None
}

/// Helper to extract numeric prefixes like "1. ", "2.3 ", "4.1.2"
fn split_numbering_prefix(text: &str) -> Option<(&str, &str)> {
    let (first, second) = text.split_once(' ')?;

    if first.chars().all(|c| c.is_ascii_digit() || c == '.') && first.contains('.') {
        Some((first, second.trim()))
    } else {
        None
    }
}

/// Generate a URL-friendly, deterministic section identifier slug.
pub fn slugify_title(title: &str) -> String {
    let clean: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();

    let slug = clean
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

/// Infer a hierarchical outline tree of `SectionNode`s from a sequential collection of pages.
pub fn infer_sections_from_pages(pages: &[Page]) -> Vec<SectionNode> {
    let mut root_sections: Vec<SectionNode> = Vec::new();
    let mut current_h1: Option<SectionNode> = None;
    let mut current_h2: Option<SectionNode> = None;

    for page in pages {
        for line in page.text.lines() {
            if let Some((level, title)) = detect_heading_level(line) {
                let id = format!("{}-p{}", slugify_title(title), page.page_number);
                let node = SectionNode {
                    id: id.clone(),
                    title: title.to_string(),
                    level,
                    page_start: page.page_number,
                    page_end: page.page_number,
                    parent_id: None,
                    children: Vec::new(),
                    content_preview: String::new(),
                };

                match level {
                    1 => {
                        // Flush preceding H2 into H1
                        if let Some(h2) = current_h2.take() {
                            if let Some(ref mut h1) = current_h1 {
                                h1.children.push(h2);
                            } else {
                                root_sections.push(h2);
                            }
                        }
                        // Flush preceding H1 to roots
                        if let Some(h1) = current_h1.take() {
                            root_sections.push(h1);
                        }
                        current_h1 = Some(node);
                    }
                    2 => {
                        // Flush preceding H2 into current H1
                        if let Some(h2) = current_h2.take() {
                            if let Some(ref mut h1) = current_h1 {
                                h1.children.push(h2);
                            } else {
                                root_sections.push(h2);
                            }
                        }
                        let mut mut_node = node;
                        if let Some(ref h1) = current_h1 {
                            mut_node.parent_id = Some(h1.id.clone());
                        }
                        current_h2 = Some(mut_node);
                    }
                    _ => {
                        // Level 3 or deeper goes into current H2 or H1
                        let mut mut_node = node;
                        if let Some(ref mut h2) = current_h2 {
                            mut_node.parent_id = Some(h2.id.clone());
                            h2.children.push(mut_node);
                        } else if let Some(ref mut h1) = current_h1 {
                            mut_node.parent_id = Some(h1.id.clone());
                            h1.children.push(mut_node);
                        } else {
                            root_sections.push(mut_node);
                        }
                    }
                }
            }
        }
    }

    // Flush remaining open sections
    if let Some(h2) = current_h2.take() {
        if let Some(ref mut h1) = current_h1 {
            h1.children.push(h2);
        } else {
            root_sections.push(h2);
        }
    }
    if let Some(h1) = current_h1.take() {
        root_sections.push(h1);
    }

    // Fallback: If no headings were detected, create a synthetic root section covering the document
    if root_sections.is_empty() && !pages.is_empty() {
        let first_page = pages.first().map(|p| p.page_number).unwrap_or(1);
        let last_page = pages.last().map(|p| p.page_number).unwrap_or(first_page);
        root_sections.push(SectionNode {
            id: "full-document".to_string(),
            title: "Document Content".to_string(),
            level: 1,
            page_start: first_page,
            page_end: last_page,
            parent_id: None,
            children: Vec::new(),
            content_preview: pages
                .first()
                .and_then(|p| p.text.lines().next())
                .unwrap_or("")
                .to_string(),
        });
    }

    root_sections
}

//! Design Patterns domain adapter.
//!
//! Dynamically analyzes document structure and text to detect patterns, intents,
//! participants, consequences, and relationships without hardcoding static databases.

use super::adapter::DomainAdapter;
use crate::document::model::{Document, SectionNode};
use serde::{Deserialize, Serialize};

/// Well-known structural sub-clauses in design pattern literature (GoF style).
const PATTERN_SUBSECTIONS: &[(&str, &str)] = &[
    ("intent", "Intent"),
    ("propósito", "Intent"),
    ("motivation", "Motivation"),
    ("motivación", "Motivation"),
    ("applicability", "Applicability"),
    ("aplicabilidad", "Applicability"),
    ("structure", "Structure"),
    ("estructura", "Structure"),
    ("participants", "Participants"),
    ("participantes", "Participants"),
    ("collaborations", "Collaborations"),
    ("colaboraciones", "Collaborations"),
    ("consequences", "Consequences"),
    ("consecuencias", "Consequences"),
    ("implementation", "Implementation"),
    ("implementación", "Implementation"),
    ("sample code", "Sample Code"),
    ("código de ejemplo", "Sample Code"),
    ("related patterns", "Related Patterns"),
    ("patrones relacionados", "Related Patterns"),
];

/// A design pattern extracted dynamically from the indexed document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedPattern {
    pub name: String,
    pub category: Option<String>,
    pub page_start: u32,
    pub page_end: u32,
    pub section_id: String,
    pub intent: Option<String>,
    pub motivation: Option<String>,
    pub applicability: Option<String>,
    pub structure: Option<String>,
    pub participants: Option<String>,
    pub consequences: Option<String>,
    pub sample_code: Option<String>,
    pub related_patterns: Vec<String>,
}

/// Dynamic knowledge adapter for Design Patterns literature.
pub struct DesignPatternsAdapter;

impl DomainAdapter for DesignPatternsAdapter {
    fn domain_id(&self) -> &'static str {
        "design_patterns"
    }

    fn domain_name(&self) -> &'static str {
        "Design Patterns Knowledge"
    }

    fn is_applicable(&self, doc: &Document) -> bool {
        let title_lower = doc.metadata.title.to_lowercase();
        if title_lower.contains("pattern")
            || title_lower.contains("patrón")
            || title_lower.contains("patrones")
        {
            return true;
        }

        // Check if common pattern sections appear in the outline
        for s in &doc.sections {
            let s_lower = s.title.to_lowercase();
            if s_lower.contains("creational")
                || s_lower.contains("creacionales")
                || s_lower.contains("structural")
                || s_lower.contains("estructurales")
                || s_lower.contains("behavioral")
                || s_lower.contains("comportamiento")
            {
                return true;
            }
        }

        false
    }
}

impl DesignPatternsAdapter {
    /// Dynamically locate and extract a design pattern by name from the document.
    pub fn get_pattern(doc: &Document, pattern_name: &str) -> Option<ExtractedPattern> {
        let target = pattern_name.to_lowercase();

        // Search through sections for a node matching the pattern name
        let matching_section = find_pattern_section(&doc.sections, &target)?;

        let mut intent = None;
        let mut motivation = None;
        let mut applicability = None;
        let mut structure = None;
        let mut participants = None;
        let mut consequences = None;
        let mut sample_code = None;
        let mut related_patterns = Vec::new();

        // 1. Inspect child subsections if the TOC has explicit children (e.g. "Intent", "Consequences")
        for child in &matching_section.children {
            let child_lower = child.title.to_lowercase();
            let text = collect_section_text(doc, child);

            for &(keyword, standard_name) in PATTERN_SUBSECTIONS {
                if child_lower.contains(keyword) {
                    match standard_name {
                        "Intent" => intent = Some(text.clone()),
                        "Motivation" => motivation = Some(text.clone()),
                        "Applicability" => applicability = Some(text.clone()),
                        "Structure" => structure = Some(text.clone()),
                        "Participants" => participants = Some(text.clone()),
                        "Consequences" => consequences = Some(text.clone()),
                        "Sample Code" => sample_code = Some(text.clone()),
                        "Related Patterns" => {
                            related_patterns = extract_related_names(&text);
                        }
                        _ => {}
                    }
                    break;
                }
            }
        }

        // 2. If no explicit child sections in TOC, extract sub-blocks from section page text
        let mut fields = PatternFields {
            intent,
            motivation,
            applicability,
            structure,
            participants,
            consequences,
            sample_code,
            related_patterns,
        };

        if fields.intent.is_none() && fields.consequences.is_none() {
            let full_text = collect_section_text(doc, matching_section);
            extract_subsections_from_text(&full_text, &mut fields);
        }

        Some(ExtractedPattern {
            name: matching_section.title.clone(),
            category: matching_section.parent_id.clone(),
            page_start: matching_section.page_start,
            page_end: matching_section.page_end,
            section_id: matching_section.id.clone(),
            intent: fields.intent,
            motivation: fields.motivation,
            applicability: fields.applicability,
            structure: fields.structure,
            participants: fields.participants,
            consequences: fields.consequences,
            sample_code: fields.sample_code,
            related_patterns: fields.related_patterns,
        })
    }

    /// Compare two patterns dynamically from the indexed document.
    pub fn compare_patterns(
        doc: &Document,
        pattern_a: &str,
        pattern_b: &str,
    ) -> Option<(ExtractedPattern, ExtractedPattern)> {
        let a = Self::get_pattern(doc, pattern_a)?;
        let b = Self::get_pattern(doc, pattern_b)?;
        Some((a, b))
    }
}

#[derive(Default)]
struct PatternFields {
    intent: Option<String>,
    motivation: Option<String>,
    applicability: Option<String>,
    structure: Option<String>,
    participants: Option<String>,
    consequences: Option<String>,
    sample_code: Option<String>,
    related_patterns: Vec<String>,
}

fn find_pattern_section<'a>(sections: &'a [SectionNode], target: &str) -> Option<&'a SectionNode> {
    for s in sections {
        let s_lower = s.title.to_lowercase();
        // Exact or word boundary match
        if s_lower == target || s_lower.contains(target) {
            return Some(s);
        }
        if let Some(child_match) = find_pattern_section(&s.children, target) {
            return Some(child_match);
        }
    }
    None
}

fn collect_section_text(doc: &Document, section: &SectionNode) -> String {
    let mut text = String::new();
    for p in section.page_start..=section.page_end {
        if let Some(page) = doc.get_page(p) {
            text.push_str(&page.text);
            text.push('\n');
        }
    }
    text
}

fn extract_subsections_from_text(text: &str, fields: &mut PatternFields) {
    let lines: Vec<&str> = text.lines().collect();
    let mut current_block: Option<&'static str> = None;
    let mut accumulated = String::new();

    for line in lines {
        let trimmed = line.trim();
        let trimmed_lower = trimmed.to_lowercase();

        let mut matched_clause = None;
        for &(keyword, std_name) in PATTERN_SUBSECTIONS {
            if trimmed_lower == keyword || trimmed_lower.starts_with(keyword) && trimmed.len() < 40
            {
                matched_clause = Some(std_name);
                break;
            }
        }

        if let Some(clause) = matched_clause {
            // Save previous block
            if let Some(b) = current_block {
                assign_block(b, &accumulated, fields);
            }
            current_block = Some(clause);
            accumulated.clear();
        } else if current_block.is_some() {
            accumulated.push_str(line);
            accumulated.push('\n');
        }
    }

    if let Some(b) = current_block {
        assign_block(b, &accumulated, fields);
    }
}

fn assign_block(clause: &str, text: &str, fields: &mut PatternFields) {
    let clean = text.trim().to_string();
    if clean.is_empty() {
        return;
    }

    match clause {
        "Intent" if fields.intent.is_none() => fields.intent = Some(clean),
        "Motivation" if fields.motivation.is_none() => fields.motivation = Some(clean),
        "Applicability" if fields.applicability.is_none() => fields.applicability = Some(clean),
        "Structure" if fields.structure.is_none() => fields.structure = Some(clean),
        "Participants" if fields.participants.is_none() => fields.participants = Some(clean),
        "Consequences" if fields.consequences.is_none() => fields.consequences = Some(clean),
        "Sample Code" if fields.sample_code.is_none() => fields.sample_code = Some(clean),
        "Related Patterns" => {
            fields
                .related_patterns
                .extend(extract_related_names(&clean));
        }
        _ => {}
    }
}

fn extract_related_names(text: &str) -> Vec<String> {
    text.split([',', ';', '\n'])
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() >= 3 && s.len() <= 40)
        .collect()
}

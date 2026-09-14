//! Context budgeting, token estimation, compact formatting, and evidence extraction.

use super::hybrid::HybridSearchHit;
use crate::document::model::Document;
use crate::document::page_labels::printed_label_suffix;
use serde::{Deserialize, Serialize};

/// Approximate characters per token for technical prose and code (tiktoken/Llama heuristics).
pub const CHARS_PER_TOKEN: f32 = 3.8;

/// Estimate the number of tokens in a given text.
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() as f32 / CHARS_PER_TOKEN).ceil() as usize
}

/// Budget configuration to constrain context size for LLM ingestion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ContextBudget {
    /// Maximum estimated tokens allowed in the response.
    pub max_tokens: usize,
    /// Maximum number of distinct chunks or evidence items.
    pub max_chunks: usize,
    /// Whether to compress whitespace and eliminate empty lines.
    pub compact: bool,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_tokens: 1500,
            max_chunks: 5,
            compact: true,
        }
    }
}

/// A compact, verifiable evidence item for LLM reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub document_id: String,
    pub page: u32,
    /// Printed label of `page`, when the PDF defines page labels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_label: Option<String>,
    pub section_title: String,
    pub section_id: Option<String>,
    pub citation: String,
    pub text: String,
    pub estimated_tokens: usize,
}

/// A bundle of evidence items within a strict token budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub query: String,
    pub items: Vec<EvidenceItem>,
    pub total_tokens: usize,
    pub total_items: usize,
    pub truncated: bool,
}

impl EvidenceBundle {
    /// Format as a clean, LLM-ready markdown evidence block.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "### Evidencia Recuperada ({} fragmentos, ~{} tokens)\n\n",
            self.items.len(),
            self.total_tokens
        ));

        for (idx, item) in self.items.iter().enumerate() {
            out.push_str(&format!(
                "**[{}] {}**\n> {}\n\n",
                idx + 1,
                item.citation,
                item.text.replace('\n', "\n> ")
            ));
        }

        if self.truncated {
            out.push_str(
                "*(Resultados adicionales omitidos para respetar el presupuesto de contexto)*\n",
            );
        }

        out
    }
}

/// Context builder with budgeting and citation generation.
pub struct ContextBuilder;

impl ContextBuilder {
    /// Build an evidence bundle from hybrid search hits respecting the given budget.
    pub fn build_evidence(
        query: &str,
        hits: &[HybridSearchHit],
        budget: ContextBudget,
    ) -> EvidenceBundle {
        let mut items = Vec::new();
        let mut current_tokens = 0;
        let mut truncated = false;

        for hit in hits.iter().take(budget.max_chunks) {
            let clean_text = if budget.compact {
                compact_text(&hit.snippet)
            } else {
                hit.snippet.clone()
            };

            let item_tokens = estimate_tokens(&clean_text);
            if current_tokens + item_tokens > budget.max_tokens && !items.is_empty() {
                truncated = true;
                break;
            }

            // Cite the page the snippet comes from, not the first page of a matched section,
            // with its printed label when the PDF defines one
            let page_ref = format!(
                "p. {}{}",
                hit.snippet_page,
                printed_label_suffix(hit.snippet_page, hit.snippet_page_label.as_deref())
            );
            let citation = match &hit.section_id {
                Some(sec) => format!("[Doc: {} {} § {}]", hit.document_id, page_ref, sec),
                None => format!("[Doc: {} {}]", hit.document_id, page_ref),
            };

            items.push(EvidenceItem {
                document_id: hit.document_id.clone(),
                page: hit.snippet_page,
                page_label: hit.snippet_page_label.clone(),
                section_title: hit.title.clone(),
                section_id: hit.section_id.clone(),
                citation,
                text: clean_text,
                estimated_tokens: item_tokens,
            });

            current_tokens += item_tokens;
        }

        EvidenceBundle {
            query: query.to_string(),
            items,
            total_tokens: current_tokens,
            total_items: hits.len(),
            truncated,
        }
    }

    /// Retrieve and expand a section with its surrounding context (parent/children/pages).
    pub fn expand_section_context(
        doc: &Document,
        section_id: &str,
        include_parent: bool,
        budget: ContextBudget,
    ) -> Option<String> {
        let node = doc.find_section(section_id)?;

        let mut out = String::new();
        out.push_str(&format!("# {}\n\n", node.title));
        out.push_str(&format!(
            "**Metadatos de Provenance:** Documento: `{}`, Páginas: {}-{}, ID: `{}`\n\n",
            doc.metadata.id, node.page_start, node.page_end, node.id
        ));

        // Parent context
        if let Some(parent) = node
            .parent_id
            .as_deref()
            .filter(|_| include_parent)
            .and_then(|pid| doc.find_section(pid))
        {
            out.push_str(&format!(
                "*Sección Padre:* **{}** (pp. {}-{})\n\n",
                parent.title, parent.page_start, parent.page_end
            ));
        }

        // Subsections overview
        if !node.children.is_empty() {
            out.push_str("### Subsecciones:\n");
            for child in &node.children {
                out.push_str(&format!(
                    "- **{}** (pp. {}-{})\n",
                    child.title, child.page_start, child.page_end
                ));
            }
            out.push('\n');
        }

        // Section body text from pages
        out.push_str("### Contenido:\n");
        let mut accumulated_tokens = estimate_tokens(&out);

        for p in node.page_start..=node.page_end {
            if let Some(page) = doc.get_page(p) {
                let page_tokens = estimate_tokens(&page.text);
                if accumulated_tokens + page_tokens > budget.max_tokens {
                    // Budget reached: take a slice
                    let remaining_tokens = budget.max_tokens.saturating_sub(accumulated_tokens);
                    let allowed_chars = (remaining_tokens as f32 * CHARS_PER_TOKEN) as usize;
                    let slice: String = page.text.chars().take(allowed_chars).collect();
                    out.push_str(&format!("\n--- [Página {} (Parcial)] ---\n", p));
                    out.push_str(&slice);
                    out.push_str(
                        "\n\n*(Contenido podado por límite de presupuesto de contexto)*\n",
                    );
                    break;
                } else {
                    out.push_str(&format!("\n--- [Página {}] ---\n", p));
                    out.push_str(&page.text);
                    accumulated_tokens += page_tokens;
                }
            }
        }

        Some(out)
    }

    /// Retrieve broader surrounding conceptual context for a query across documents.
    pub fn build_conceptual_context(
        query: &str,
        hits: &[HybridSearchHit],
        docs: &[Document],
        budget: ContextBudget,
    ) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "### Contexto Conceptual para: '{}' (Presupuesto: ~{} tokens)\n\n",
            query, budget.max_tokens
        ));

        let mut accumulated_tokens = estimate_tokens(&out);
        let mut included_sections = std::collections::HashSet::new();

        for hit in hits.iter().take(budget.max_chunks) {
            if accumulated_tokens >= budget.max_tokens {
                out.push_str("\n*(Límite de presupuesto de contexto alcanzado)*\n");
                break;
            }

            let doc = match docs.iter().find(|d| d.id.0 == hit.document_id) {
                Some(d) => d,
                None => continue,
            };

            if let Some(ref sec_id) = hit.section_id {
                if !included_sections.insert((hit.document_id.clone(), sec_id.clone())) {
                    continue;
                }

                let remaining_tokens = budget.max_tokens.saturating_sub(accumulated_tokens);
                let section_budget = ContextBudget {
                    max_tokens: remaining_tokens.min(600),
                    max_chunks: 3,
                    compact: budget.compact,
                };

                if let Some(section_text) =
                    Self::expand_section_context(doc, sec_id, true, section_budget)
                {
                    let section_tokens = estimate_tokens(&section_text);
                    out.push_str(&section_text);
                    out.push_str("\n---\n\n");
                    accumulated_tokens += section_tokens;
                }
            } else {
                let snippet_clean = if budget.compact {
                    compact_text(&hit.snippet)
                } else {
                    hit.snippet.clone()
                };
                let snippet_text = format!(
                    "**[Doc: {} p. {}{}]** {}\n\n",
                    hit.document_id,
                    hit.snippet_page,
                    printed_label_suffix(hit.snippet_page, hit.snippet_page_label.as_deref()),
                    snippet_clean
                );
                let snippet_tokens = estimate_tokens(&snippet_text);
                out.push_str(&snippet_text);
                accumulated_tokens += snippet_tokens;
            }
        }

        out
    }
}

/// Compact text by collapsing multiple newlines and consecutive whitespace.
pub fn compact_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_space = false;
    let mut newline_count = 0;

    for c in text.chars() {
        if c == '\n' || c == '\r' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push('\n');
            }
            last_was_space = false;
        } else if c.is_whitespace() {
            if !last_was_space && newline_count == 0 {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            newline_count = 0;
            last_was_space = false;
            result.push(c);
        }
    }

    result.trim().to_string()
}

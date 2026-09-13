//! High-performance in-memory Okapi BM25 index for sections and pages.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::document::model::{Document, SectionNode};

/// BM25 parameters. Standard values: k1 = 1.2, b = 0.75.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bm25Config {
    pub k1: f32,
    pub b: f32,
}

impl Default for Bm25Config {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// A searchable document unit (can be a Section or a Page).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchUnit {
    pub id: String,
    pub document_id: String,
    pub title: String,
    pub page_start: u32,
    pub page_end: u32,
    pub section_id: Option<String>,
    pub text: String,
    pub term_counts: HashMap<String, u32>,
    pub length: usize,
}

/// In-memory inverted index implementing Okapi BM25 ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Index {
    pub config: Bm25Config,
    pub units: Vec<SearchUnit>,
    pub inverted_index: HashMap<String, Vec<(usize, u32)>>, // term -> [(unit_idx, term_freq)]
    pub avg_doc_length: f32,
    pub total_docs: usize,
}

/// A ranked search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub unit_id: String,
    pub document_id: String,
    pub title: String,
    pub page_start: u32,
    pub page_end: u32,
    pub section_id: Option<String>,
    pub snippet: String,
    pub score: f32,
}

impl Bm25Index {
    /// Build a BM25 index from a collection of documents.
    pub fn build_from_documents(docs: &[Document], config: Option<Bm25Config>) -> Self {
        let config = config.unwrap_or_default();
        let mut units = Vec::new();

        for doc in docs {
            // Index sections
            for section in &doc.sections {
                collect_section_units(doc, section, &mut units);
            }

            // Also index pages for fine-grained lookups
            for page in &doc.pages {
                let text = page.text.trim();
                if text.len() > 20 {
                    let tokens = tokenize(text);
                    let length = tokens.len();
                    let term_counts = count_terms(&tokens);

                    units.push(SearchUnit {
                        id: format!("{}:p{}", doc.metadata.id, page.page_number),
                        document_id: doc.metadata.id.clone(),
                        title: format!("Página {}", page.page_number),
                        page_start: page.page_number,
                        page_end: page.page_number,
                        section_id: None,
                        text: text.to_string(),
                        term_counts,
                        length,
                    });
                }
            }
        }

        let total_docs = units.len();
        let total_length: usize = units.iter().map(|u| u.length).sum();
        let avg_doc_length = if total_docs > 0 {
            total_length as f32 / total_docs as f32
        } else {
            1.0
        };

        // Build inverted index
        let mut inverted_index: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
        for (idx, unit) in units.iter().enumerate() {
            for (term, &count) in &unit.term_counts {
                inverted_index
                    .entry(term.clone())
                    .or_default()
                    .push((idx, count));
            }
        }

        Self {
            config,
            units,
            inverted_index,
            avg_doc_length,
            total_docs,
        }
    }

    /// Query the BM25 index and return the top `limit` results.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() || self.total_docs == 0 {
            return Vec::new();
        }

        let mut scores: HashMap<usize, f32> = HashMap::new();

        for term in &query_terms {
            if let Some(postings) = self.inverted_index.get(term) {
                let n_q = postings.len() as f32;
                // Standard Robertson-Spärck Jones IDF
                let idf = ((self.total_docs as f32 - n_q + 0.5) / (n_q + 0.5) + 1.0).ln();

                for &(unit_idx, freq) in postings {
                    let unit = &self.units[unit_idx];
                    let f = freq as f32;
                    let numerator = f * (self.config.k1 + 1.0);
                    let denominator = f + self.config.k1
                        * (1.0 - self.config.b
                            + self.config.b * (unit.length as f32 / self.avg_doc_length));
                    let term_score = idf * (numerator / denominator);
                    *scores.entry(unit_idx).or_default() += term_score;
                }
            }
        }

        let mut ranked: Vec<(usize, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked
            .into_iter()
            .take(limit)
            .map(|(idx, score)| {
                let unit = &self.units[idx];
                SearchHit {
                    unit_id: unit.id.clone(),
                    document_id: unit.document_id.clone(),
                    title: unit.title.clone(),
                    page_start: unit.page_start,
                    page_end: unit.page_end,
                    section_id: unit.section_id.clone(),
                    snippet: extract_snippet(&unit.text, &query_terms, 250),
                    score,
                }
            })
            .collect()
    }
}

fn collect_section_units(doc: &Document, section: &SectionNode, units: &mut Vec<SearchUnit>) {
    let mut combined_text = format!("{}\n", section.title);
    for p in section.page_start..=section.page_end {
        if let Some(page) = doc.get_page(p) {
            combined_text.push_str(&page.text);
            combined_text.push('\n');
        }
    }

    let tokens = tokenize(&combined_text);
    let length = tokens.len();
    let term_counts = count_terms(&tokens);

    units.push(SearchUnit {
        id: format!("{}:sec:{}", doc.metadata.id, section.id),
        document_id: doc.metadata.id.clone(),
        title: section.title.clone(),
        page_start: section.page_start,
        page_end: section.page_end,
        section_id: Some(section.id.clone()),
        text: combined_text,
        term_counts,
        length,
    });

    for child in &section.children {
        collect_section_units(doc, child, units);
    }
}

/// Tokenize text into lowercased alphanumeric tokens, stripping punctuation and stop words.
pub fn tokenize(text: &str) -> Vec<String> {
    let stop_words = get_stop_words();
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter_map(|word| {
            let clean = word.trim().to_lowercase();
            if clean.len() >= 2 && !stop_words.contains(clean.as_str()) {
                Some(clean)
            } else {
                None
            }
        })
        .collect()
}

fn count_terms(tokens: &[String]) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for t in tokens {
        *map.entry(t.clone()).or_default() += 1;
    }
    map
}

/// Extract a contextual snippet around matching terms.
fn extract_snippet(text: &str, query_terms: &[String], max_chars: usize) -> String {
    let lower = text.to_lowercase();
    let mut best_pos = 0;

    for term in query_terms {
        if let Some(pos) = lower.find(term) {
            best_pos = pos;
            break;
        }
    }

    let start = best_pos.saturating_sub(60);
    let end = (start + max_chars).min(text.len());

    let mut snippet = text[start..end].trim().replace('\n', " ");
    if start > 0 {
        snippet = format!("... {}", snippet);
    }
    if end < text.len() {
        snippet = format!("{} ...", snippet);
    }

    snippet
}

fn get_stop_words() -> HashSet<&'static str> {
    let mut s = HashSet::new();
    // English
    for w in [
        "the", "is", "at", "which", "on", "a", "an", "and", "or", "in", "with", "as", "to", "for",
        "of", "by", "that", "this", "it", "from", "be", "are", "was",
    ] {
        s.insert(w);
    }
    // Spanish
    for w in [
        "el", "la", "los", "las", "un", "una", "unos", "unas", "de", "del", "en", "para", "por",
        "con", "sin", "sobre", "entre", "que", "y", "o", "es", "son", "fue", "este", "esta",
        "estos", "estas", "como", "su", "sus",
    ] {
        s.insert(w);
    }
    s
}

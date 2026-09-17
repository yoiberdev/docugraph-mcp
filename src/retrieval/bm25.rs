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

/// A query term together with how much information it carries in this corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryTerm {
    pub term: String,
    /// How many units contain the term. Zero means it is absent from the corpus.
    pub df: usize,
    pub idf: f32,
}

/// The distinct terms of a query, measured against a specific index.
///
/// This is what makes "no evidence" decidable. A fused relevance score is
/// normalised per query, so its best hit always scores near the top whatever the
/// query was; IDF is absolute, so it can answer whether a passage carries enough
/// of what was asked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryProfile {
    pub terms: Vec<QueryTerm>,
    pub total_idf: f32,
}

impl QueryProfile {
    /// The mean information of a query term: the bar a unit must clear to count
    /// as evidence.
    ///
    /// Derived from the query and the corpus, so there is no constant to tune and
    /// nothing to recalibrate for a different document. Terms absent from the
    /// corpus carry the maximum IDF, so asking about something the corpus does not
    /// contain raises the bar rather than lowering it. Being a ratio, it is also
    /// invariant to corpus size and to how verbose the query is.
    ///
    /// Measured on a 437-page Spanish manual (488 units), against the 83-to-228
    /// units the previous fused-score threshold admitted for the same questions.
    /// Queries are quoted verbatim, because the verdict depends on the exact
    /// wording:
    ///
    /// | query                                        | covered | admitted |
    /// |----------------------------------------------|---------|----------|
    /// | "receta de paella valenciana …"              | no      | 0        |
    /// | "cómo implementar memoization en Rust …"     | no      | 0        |
    /// | "how to resolve merge conflicts …"           | no      | 0        |
    /// | "cuál es la dosis … de ibuprofeno …"         | no      | 0        |
    /// | "cómo configurar ingress de kubernetes …"    | no      | 5        |
    /// | "intención del patrón Strategy"              | yes     | 31       |
    /// | "principio abierto cerrado OCP"              | yes     | 41       |
    /// | "Observer pattern subscribers"               | yes     | 0        |
    ///
    /// Two residuals, in opposite directions.
    ///
    /// Admitting too much: the "cómo configurar X" shape. When every informative
    /// term is absent, the generic verbs left over can just clear the mean (5.796
    /// against a bar of 5.291).
    ///
    /// Admitting too little: a query whose informative terms are in a language the
    /// corpus is not written in. "Observer pattern subscribers" is refused even
    /// though `observer` appears in 25 units, because `pattern` and `subscribers`
    /// are absent, take the maximum IDF and lift the bar to 5.575 - past what
    /// `observer` alone (2.95) can supply. Agents that reason in English over a
    /// Spanish corpus will hit this; `document_outline` shows them the vocabulary
    /// the corpus actually uses.
    ///
    /// Both alternatives measured over 13 queries make it worse, not better:
    /// averaging only the present terms drops the two cross-language misses but
    /// admits 85 units for the memoization query and 9 for the ibuprofen one (4
    /// wrong verdicts); taking the maximum present IDF behaves the same (4). The
    /// mean over all terms is wrong 3 times, and is what ships.
    pub fn admission_floor(&self) -> f32 {
        if self.terms.is_empty() {
            return f32::INFINITY;
        }
        self.total_idf / self.terms.len() as f32
    }

    /// Query terms that appear nowhere in the corpus.
    pub fn absent_terms(&self) -> Vec<&str> {
        self.terms
            .iter()
            .filter(|t| t.df == 0)
            .map(|t| t.term.as_str())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
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

    /// Measure a query's distinct terms against this index.
    pub fn profile_query(&self, query: &str) -> QueryProfile {
        let mut seen = HashSet::new();
        let mut terms = Vec::new();
        for term in tokenize(query) {
            if !seen.insert(term.clone()) {
                continue;
            }
            let df = self.inverted_index.get(&term).map_or(0, |p| p.len());
            terms.push(QueryTerm {
                idf: idf(self.total_docs, df),
                term,
                df,
            });
        }
        let total_idf = terms.iter().map(|t| t.idf).sum();
        QueryProfile { terms, total_idf }
    }

    /// How much of the query's IDF mass each unit actually contains.
    ///
    /// Walks the postings of the query terms, so the cost is proportional to the
    /// matches rather than to the size of the corpus.
    pub fn matched_idf(&self, profile: &QueryProfile) -> HashMap<usize, f32> {
        let mut mass: HashMap<usize, f32> = HashMap::new();
        for qt in &profile.terms {
            let Some(postings) = self.inverted_index.get(qt.term.as_str()) else {
                continue;
            };
            for &(unit_idx, _) in postings {
                *mass.entry(unit_idx).or_default() += qt.idf;
            }
        }
        mass
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
                let idf = idf(self.total_docs, postings.len());

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

/// Robertson-Spärck Jones IDF, Lucene's non-negative variant, for a term present
/// in `df` of `total` units.
///
/// A term absent from the corpus (`df == 0`) takes the maximum value: it is both
/// maximally informative and maximally unsatisfied. Admission and ranking share
/// this function so the two can never disagree about what a term is worth.
fn idf(total: usize, df: usize) -> f32 {
    let n = total as f32;
    let d = df as f32;
    ((n - d + 0.5) / (d + 0.5) + 1.0).ln()
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

/// Extract a contextual snippet around matching terms, safely respecting UTF-8 boundaries.
fn extract_snippet(text: &str, query_terms: &[String], max_chars: usize) -> String {
    let lower = text.to_lowercase();
    let mut best_pos = 0;

    for term in query_terms {
        if let Some(pos) = lower.find(term) {
            best_pos = pos;
            break;
        }
    }

    let raw_start = best_pos.saturating_sub(60);
    let raw_end = (raw_start + max_chars).min(text.len());

    // Snap to valid UTF-8 character boundaries
    let mut start = raw_start;
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = raw_end;
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }

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

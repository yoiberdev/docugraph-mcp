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
    /// Byte offset at which each page's text starts inside `text`, ascending.
    ///
    /// A section unit is the concatenation of every page it spans, so without this
    /// a snippet taken from the middle of a 171-page chapter could only be cited
    /// against the chapter's first page. Measured on a 437-page manual, that put
    /// 29 of 34 multi-page section citations on a page the text is not on.
    #[serde(default)]
    pub page_offsets: Vec<(usize, u32)>,
    pub term_counts: HashMap<String, u32>,
    pub length: usize,
}

impl SearchUnit {
    /// The page a byte offset inside `text` falls on.
    pub fn page_at(&self, offset: usize) -> u32 {
        self.page_offsets
            .iter()
            .rev()
            .find(|(start, _)| offset >= *start)
            .map(|(_, page)| *page)
            .unwrap_or(self.page_start)
    }
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
    pub idf: f64,
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
    pub total_idf: f64,
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
    /// | "cómo configurar ingress de kubernetes …"    | no      | 0        |
    /// | "intención del patrón Strategy"              | yes     | 31       |
    /// | "principio abierto cerrado OCP"              | yes     | 41       |
    /// | "Observer pattern subscribers"               | yes     | 5        |
    ///
    /// Alternatives to the mean, measured over 13 queries, all make it worse:
    /// averaging only the present terms admits 85 units for the memoization query
    /// and 9 for the ibuprofen one; taking the maximum present IDF behaves the
    /// same. Both score 4 wrong verdicts against this rule's 3.
    ///
    /// The "cómo configurar X" residual this used to document is gone, and not by
    /// changing the bar. IDF cannot tell a rare verb (`configurar`, df 8) from a
    /// rare topic (`observer`, df 25) - both are rare - so no threshold could have
    /// separated them. What removed it was treating the interrogative as what it
    /// is: `cómo` now folds to `como`, which was already a stop word, leaving one
    /// generic verb that cannot reach the bar alone. Over a labelled set of 20
    /// queries against the 437-page manual, in both languages and with and without
    /// accents, there are currently no wrong verdicts.
    ///
    /// The opposite residual is gone. English queries over this Spanish manual used
    /// to be refused on covered topics, because the manual keeps the English
    /// pattern names (`observer`, `decorator`, `strategy`, `factory` are all in its
    /// vocabulary) while inflected forms like `pattern` and `subscribers` are not.
    /// [`Bm25Index::corpus_form_of`] resolves those to the form the corpus uses.
    ///
    /// Compare against it with [`QueryProfile::admits`] rather than `>=` directly:
    /// the bar and the mass a passage carries are floating-point sums of the same
    /// per-term IDFs accumulated in different orders, so an exact comparison
    /// mis-handles the tie this rule is meant to include.
    pub fn admission_floor(&self) -> f64 {
        if self.terms.is_empty() {
            return f64::INFINITY;
        }
        self.total_idf / self.terms.len() as f64
    }

    /// Does a passage carrying `matched_idf` of this query count as evidence?
    ///
    /// Inclusive by design and tolerant by necessity. When every query term shares a
    /// `df` - the normal case for the rare identifiers agents search for, all
    /// sitting at df=1 - the bar is `(v + v + v) / 3` while a passage holding one of
    /// them carries exactly `v`. Neither `3v` nor the division is exact in binary
    /// floating point, so the bar can land one ULP above `v` and the passage is
    /// refused although the rule says it qualifies. That refusal is worse than it
    /// sounds: no query term is absent, so `NoEvidence` reports none, and the
    /// message tells the agent to rephrase a query that was already right while
    /// `document_search` happily returns the same passages.
    ///
    /// The slack is the error bound of the bar itself - at most `n` additions and
    /// one division, each contributing at most one ULP - so it is derived from the
    /// computation rather than tuned. At roughly 1e-15 relative it cannot change any
    /// verdict that was not already a tie.
    pub fn admits(&self, matched_idf: f64) -> bool {
        let floor = self.admission_floor();
        if !floor.is_finite() {
            return false;
        }
        let slack = floor.abs() * f64::EPSILON * (self.terms.len() + 1) as f64;
        matched_idf >= floor - slack
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
    /// The page this snippet's text is actually on.
    ///
    /// Distinct from `page_start`, which is where the unit begins: for a section
    /// spanning many pages those are rarely the same page, and this is the one a
    /// citation has to name for a reader to be able to check it.
    pub snippet_page: u32,
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
                        page_offsets: vec![(0, page.page_number)],
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
        for token in tokenize(query) {
            // A term the corpus does not have may still be a form of one it does.
            let term = match self.inverted_index.contains_key(&token) {
                true => token,
                false => self.corpus_form_of(&token).unwrap_or(token),
            };
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

    /// The form of `token` that this corpus actually uses, if it uses one.
    ///
    /// An absent term takes the maximum IDF and so raises the admission bar for
    /// every passage. That is right when the corpus does not cover the topic, and
    /// wrong when it covers it under another form of the same word: asking a
    /// Spanish manual about "Observer pattern subscribers" was refused even though
    /// `observer` appears in 25 units, because `pattern` and `subscribers` are
    /// absent while `patterns` and `subscribe` are present.
    ///
    /// There is no similarity score and no threshold here. A candidate counts only
    /// if the corpus contains it, which is why genuinely absent topics stay absent:
    /// measured over the 437-page manual, all eight morphological variants tried
    /// resolved and none of `paella`, `kubernetes`, `ingress`, `memoization`,
    /// `ibuprofeno`, `sourdough`, `macros`, `rust`, `azafrán`, `garrofón`, `tls` or
    /// `procedurales` resolved to anything.
    ///
    /// The suffixes are the ones that change a word's form without changing what it
    /// denotes, and each one had to earn its place. Appending "es" was tried and
    /// removed: it resolved none of the eight cases the other rules already cover,
    /// and it turned the English "intent" into the Spanish verb form "intentes",
    /// which answered a query through a word that has nothing to do with it. A
    /// plausible answer reached through a wrong match is the failure this whole
    /// admission step exists to prevent, so the rule went rather than the case.
    ///
    /// Words that differ by more than a suffix - a true translation like "intent"
    /// for "intención" - are out of reach here and stay absent. That is the honest
    /// outcome: the query is refused, and the refusal says to check `document_outline`
    /// for the vocabulary the corpus actually uses.
    fn corpus_form_of(&self, token: &str) -> Option<String> {
        let mut best: Option<(String, usize)> = None;
        let mut consider = |candidate: String| {
            if candidate.len() < 2 {
                return;
            }
            if let Some(postings) = self.inverted_index.get(&candidate) {
                let df = postings.len();
                // Prefer the form the corpus uses most: it is the established one.
                if best.as_ref().is_none_or(|(_, b)| df > *b) {
                    best = Some((candidate, df));
                }
            }
        };

        consider(format!("{token}s"));
        if let Some(stem) = token.strip_suffix('s') {
            consider(stem.to_string());
            if let Some(shorter) = stem.strip_suffix('e') {
                consider(shorter.to_string());
            }
            // Agent nouns: "subscribers" is how the reader asks about "subscribe".
            if let Some(root) = stem.strip_suffix("er") {
                consider(format!("{root}e"));
                consider(root.to_string());
            }
        }
        if let Some(root) = token.strip_suffix("er") {
            consider(format!("{root}e"));
            consider(root.to_string());
        }
        if let Some(root) = token.strip_suffix("ing") {
            consider(format!("{root}e"));
            consider(root.to_string());
        }

        // `ñ` survives folding because it is a letter rather than an accented `n`,
        // so `año` and `ano` stay different terms. That leaves the reader who types
        // `diseno` for `diseño`, which is common enough to be worth catching - but
        // only here, where the plain form is already known to be absent, so the
        // distinction still holds for every word the corpus actually contains.
        if token.contains('n') {
            let chars: Vec<char> = token.chars().collect();
            for (i, c) in chars.iter().enumerate() {
                if *c == 'n' {
                    let mut candidate: Vec<char> = chars.clone();
                    candidate[i] = 'ñ';
                    consider(candidate.into_iter().collect());
                }
            }
        }

        best.map(|(term, _)| term)
    }

    /// How much of the query's IDF mass each unit actually contains.
    ///
    /// Walks the postings of the query terms, so the cost is proportional to the
    /// matches rather than to the size of the corpus.
    pub fn matched_idf(&self, profile: &QueryProfile) -> HashMap<usize, f64> {
        let mut mass: HashMap<usize, f64> = HashMap::new();
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
        self.search_terms(&tokenize(query), limit)
    }

    /// BM25 score for every unit carrying any of these terms.
    ///
    /// Separate from ranking because scoring is cheap and snippet extraction is
    /// not: a caller that needs scores for a wide candidate set but snippets for
    /// only the few it returns should not pay for the ones it discards.
    pub fn score_terms(&self, query_terms: &[String]) -> HashMap<usize, f32> {
        let mut scores: HashMap<usize, f32> = HashMap::new();
        if query_terms.is_empty() || self.total_docs == 0 {
            return scores;
        }

        for term in query_terms {
            if let Some(postings) = self.inverted_index.get(term) {
                let idf = idf(self.total_docs, postings.len()) as f32;

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
        scores
    }

    /// The quotable window from a unit, and the page it is on.
    pub fn snippet_for(&self, unit_idx: usize, query_terms: &[String]) -> (String, u32) {
        let unit = &self.units[unit_idx];
        let (snippet, offset) = extract_snippet(&unit.text, query_terms, 250);
        (snippet, unit.page_at(offset))
    }

    /// Rank against an already-resolved term set.
    ///
    /// The hybrid path passes the terms from its `QueryProfile` so that ranking
    /// scores the same query admission judged. Re-tokenizing the raw string here
    /// would drop any term `profile_query` resolved to the form the corpus uses.
    pub fn search_terms(&self, query_terms: &[String], limit: usize) -> Vec<SearchHit> {
        let scores = self.score_terms(query_terms);
        let mut ranked: Vec<(usize, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked
            .into_iter()
            .take(limit)
            .map(|(idx, score)| {
                let unit = &self.units[idx];
                let (snippet, snippet_page) = self.snippet_for(idx, query_terms);
                SearchHit {
                    unit_id: unit.id.clone(),
                    document_id: unit.document_id.clone(),
                    title: unit.title.clone(),
                    page_start: unit.page_start,
                    page_end: unit.page_end,
                    section_id: unit.section_id.clone(),
                    snippet,
                    snippet_page,
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
fn idf(total: usize, df: usize) -> f64 {
    let n = total as f64;
    let d = df as f64;
    ((n - d + 0.5) / (d + 0.5) + 1.0).ln()
}

fn collect_section_units(doc: &Document, section: &SectionNode, units: &mut Vec<SearchUnit>) {
    let mut combined_text = format!("{}\n", section.title);
    let mut page_offsets: Vec<(usize, u32)> = Vec::new();
    for p in section.page_start..=section.page_end {
        if let Some(page) = doc.get_page(p) {
            // Recorded before appending, so the offset is where this page begins.
            page_offsets.push((combined_text.len(), p));
            combined_text.push_str(&page.text);
            combined_text.push('\n');
        }
    }
    // The title prefix sits before the first page's text but belongs to it.
    if let Some((first, _)) = page_offsets.first_mut() {
        *first = 0;
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
        page_offsets,
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
            let clean = fold_for_matching(word.trim());
            if clean.chars().count() >= 2 && !stop_words.contains(clean.as_str()) {
                Some(clean)
            } else {
                None
            }
        })
        .collect()
}

/// Lowercase a word and reduce the spellings that should match each other.
///
/// Applied to the index and to the query alike, so the two always agree.
///
/// Accents are folded because dropping one is the commonest way to mistype a
/// Spanish word, and the consequence was severe rather than merely unhelpful: an
/// absent term takes the maximum IDF, and the admission bar is the mean of the
/// query's term IDFs, so one missing tilde lifted the bar above anything the
/// corpus could supply. `intención del patrón Strategy` returned passages while
/// `intencion del patron Strategy` returned "no evidence" - the strongest verdict
/// this system can give, about a document that plainly contains the text.
///
/// `ñ` is deliberately left alone. It is a letter in its own right, not an `n`
/// wearing an accent: folding it would make `año` and `ano` the same term, and on
/// a Spanish keyboard it is a single key, so it is not what gets dropped in a
/// hurry - accents are.
///
/// Ligatures are expanded because a PDF whose font maps to `/fi` and `/fl` - which
/// is most LaTeX and InDesign output - stores `configuración` with a single
/// codepoint that no query will ever spell that way.
fn fold_for_matching(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for ch in word.chars().flat_map(|c| c.to_lowercase()) {
        match ch {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => out.push('o'),
            'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ç' => out.push('c'),
            // Combining marks, for text that arrived already decomposed.
            '\u{0300}'..='\u{036F}' => {}
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            other => out.push(other),
        }
    }
    out
}

fn count_terms(tokens: &[String]) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for t in tokens {
        *map.entry(t.clone()).or_default() += 1;
    }
    map
}

/// Extract a contextual snippet around matching terms, safely respecting UTF-8 boundaries.
/// A window of `text` around the first query term, and the offset of the term
/// itself so the caller can resolve which page to cite.
///
/// The term's offset, not the window's: the window is backed up 60 bytes for
/// context, which for a term near the top of a page starts it on the previous
/// one. A citation should name the page the matched text is on.
fn extract_snippet(text: &str, query_terms: &[String], max_chars: usize) -> (String, usize) {
    // Query terms arrive folded by `tokenize`, so the haystack has to be folded
    // the same way or an accented word would never be located - and the snippet
    // would silently fall back to the opening of the unit, taking the citation's
    // page with it. Folding changes byte lengths, so the offset of each folded
    // byte back into the original is carried alongside.
    let mut folded = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len());
    for (byte_idx, ch) in text.char_indices() {
        let before = folded.len();
        folded.push_str(&fold_for_matching(&ch.to_string()));
        origin.resize(folded.len().max(before), byte_idx);
    }

    let mut best_pos = 0;
    for term in query_terms {
        if let Some(pos) = folded.find(term.as_str()) {
            best_pos = origin.get(pos).copied().unwrap_or(0);
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

    (snippet, best_pos)
}

fn get_stop_words() -> HashSet<&'static str> {
    let mut s = HashSet::new();
    // English
    for w in [
        "the", "is", "at", "which", "on", "a", "an", "and", "or", "in", "with", "as", "to", "for",
        "of", "by", "that", "this", "it", "from", "be", "are", "was",
        // Interrogatives and framing verbs. A question word carries no topic, but
        // when it is absent from the corpus it takes the maximum IDF and lifts the
        // admission bar, so "What is the replication factor?" was refused by a
        // corpus containing "replication" hundreds of times.
        "what", "when", "where", "who", "why", "how", "does", "do", "did", "can", "should", "would",
        "will", "use", "using", "used",
    ] {
        s.insert(w);
    }
    // Spanish
    for w in [
        "el", "la", "los", "las", "un", "una", "unos", "unas", "de", "del", "en", "para", "por",
        "con", "sin", "sobre", "entre", "que", "y", "o", "es", "son", "fue", "este", "esta",
        "estos", "estas", "como", "su", "sus",
        // Written folded, because tokenize strips the accents before this lookup.
        "cual", "cuales", "cuando", "donde", "quien", "porque", "cuanto", "usa", "usar", "usando",
        "hacer", "hace", "ser", "estan", "mas", "asi",
    ] {
        s.insert(w);
    }
    s
}

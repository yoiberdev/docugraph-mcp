use docugraph::document::model::{Document, DocumentMetadata, Page, SectionNode};
use docugraph::knowledge::design_patterns::DesignPatternsAdapter;
use docugraph::retrieval::{
    Bm25Index, ContextBudget, ContextBuilder, DeterministicSubwordEmbedding, EmbeddingProvider,
    HybridRetriever, HybridWeights,
};

fn create_sample_design_pattern_doc() -> Document {
    let mut doc = Document::new(DocumentMetadata {
        id: "gof-sample".to_string(),
        title: "Design Patterns: Elements of Reusable Object-Oriented Software".to_string(),
        author: Some("Gang of Four".to_string()),
        total_pages: 10,
        total_sections: 4,
        file_size_bytes: 4096,
        content_hash: "abcd1234deadbeef".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
        total_links: 0,
        ..Default::default()
    });

    // Page 1: Overview
    doc.add_page(Page::new(
        1,
        "Chapter 5: Behavioral Patterns. Algorithms and assignment of responsibilities between objects.",
    ));

    // Page 2: Strategy Pattern
    doc.add_page(Page::new(
        2,
        "Strategy Pattern\n\nIntent\nDefine a family of algorithms, encapsulate each one, and make them interchangeable.\n\nMotivation\nMany algorithms exist for breaking a stream of text into lines.",
    ));

    // Page 3: Strategy Details
    doc.add_page(Page::new(
        3,
        "Applicability\nUse Strategy when many related classes differ only in their behavior.\n\nParticipants\n- Strategy: declares an interface common to all supported algorithms.\n- ConcreteStrategy: implements the algorithm.\n- Context: is configured with a ConcreteStrategy object.\n\nConsequences\n1. Families of related algorithms.\n2. An alternative to subclassing.\n3. Eliminates conditional statements.",
    ));

    // Page 4: State Pattern
    doc.add_page(Page::new(
        4,
        "State Pattern\n\nIntent\nAllow an object to alter its behavior when its internal state changes. The object will appear to change its class.\n\nConsequences\n1. It localizes state-specific behavior and partitions behavior for different states.",
    ));

    // Sections
    let mut strategy_sec = SectionNode::new(
        "strategy",
        "Strategy",
        2,
        2,
        3,
        Some("behavioral".to_string()),
    );
    let intent_sec = SectionNode::new(
        "strategy-intent",
        "Intent",
        3,
        2,
        2,
        Some("strategy".to_string()),
    );
    let consequences_sec = SectionNode::new(
        "strategy-consequences",
        "Consequences",
        3,
        3,
        3,
        Some("strategy".to_string()),
    );
    strategy_sec.add_child(intent_sec);
    strategy_sec.add_child(consequences_sec);

    let state_sec = SectionNode::new("state", "State", 2, 4, 4, Some("behavioral".to_string()));

    let mut behavioral_sec = SectionNode::new("behavioral", "Behavioral Patterns", 1, 1, 4, None);
    behavioral_sec.add_child(strategy_sec);
    behavioral_sec.add_child(state_sec);

    doc.sections.push(behavioral_sec);
    doc
}

#[test]
fn test_bm25_search() {
    let doc = create_sample_design_pattern_doc();
    let bm25 = Bm25Index::build_from_documents(&[doc], None);

    let hits = bm25.search("family algorithms interchangeable", 3);
    assert!(!hits.is_empty(), "Expected BM25 hits for strategy intent");
    assert_eq!(hits[0].document_id, "gof-sample");
    assert!(hits[0].snippet.to_lowercase().contains("algorithm"));
}

#[test]
fn test_bm25_utf8_spanish_characters() {
    let mut doc = Document::new(DocumentMetadata {
        id: "spanish-test".to_string(),
        title: "Aprendiendo Git y Arquitectura de Software".to_string(),
        author: Some("Autor Hispano".to_string()),
        total_pages: 1,
        total_sections: 1,
        file_size_bytes: 1024,
        content_hash: "1234utf8".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
        total_links: 0,
        ..Default::default()
    });

    doc.add_page(Page::new(
        1,
        "¿Cómo funciona la sincronización en Git? El árbol de confirmaciones añade ramas para el diseño ágil y estructurado con un ñandú.",
    ));
    doc.sections
        .push(SectionNode::new("sec-1", "Sincronización", 1, 1, 1, None));

    let bm25 = Bm25Index::build_from_documents(&[doc], None);
    let hits = bm25.search("sincronización ramas diseño", 1);
    assert!(!hits.is_empty(), "BM25 should match Spanish terms");
    assert!(!hits[0].snippet.is_empty());
}

#[test]
fn test_embedding_and_cosine_similarity() {
    let provider = DeterministicSubwordEmbedding::default();
    let vec1 = provider.embed("Strategy algorithm design pattern").unwrap();
    let vec2 = provider.embed("Strategy algorithm patterns").unwrap();
    let vec3 = provider.embed("Cooking recipes italian pasta").unwrap();

    let sim_related = docugraph::retrieval::embedding::cosine_similarity(&vec1, &vec2);
    let sim_unrelated = docugraph::retrieval::embedding::cosine_similarity(&vec1, &vec3);

    assert!(
        sim_related > sim_unrelated,
        "Related terms should have higher similarity"
    );
    assert!(
        sim_related > 0.5,
        "Close terms should have significant similarity"
    );
}

#[test]
fn test_hybrid_search_scoring() {
    let doc = create_sample_design_pattern_doc();
    let retriever = HybridRetriever::build(&[doc], None);

    let hits = retriever
        .search(
            "encapsulate interchangeable algorithms",
            3,
            &HybridWeights::DEFAULT,
        )
        .expect("the sample document covers this query");
    assert!(!hits.is_empty());
    assert!(hits[0].final_score > 0.3);
    assert!(hits[0].bm25_score > 0.0);
    assert!(hits[0].semantic_score > 0.0);
    assert!(hits[0].title == "Strategy" || hits[0].title == "Intent");
}

#[test]
fn test_context_budgeter_and_evidence() {
    let doc = create_sample_design_pattern_doc();
    let retriever = HybridRetriever::build(&[doc], None);
    let hits = retriever
        .search("interchangeable algorithms", 5, &HybridWeights::DEFAULT)
        .expect("the sample document covers this query");

    let budget = ContextBudget {
        max_tokens: 100,
        max_chunks: 2,
        compact: true,
    };

    let bundle = ContextBuilder::build_evidence("interchangeable algorithms", &hits, budget);
    assert!(!bundle.items.is_empty());
    assert!(bundle.items.len() <= 2);
    assert!(bundle.total_tokens <= 150);

    let markdown = bundle.to_markdown();
    assert!(markdown.contains("Evidencia Recuperada"));
    assert!(markdown.contains("[Doc: gof-sample"));
}

#[test]
fn test_design_patterns_dynamic_adapter() {
    let doc = create_sample_design_pattern_doc();

    let pattern = DesignPatternsAdapter::get_pattern(&doc, "Strategy")
        .expect("Strategy pattern must be detected");
    assert_eq!(pattern.name, "Strategy");
    assert!(pattern.intent.is_some());
    let intent_str = pattern.intent.as_ref().unwrap();
    assert!(intent_str.contains("family of algorithms"));

    // Test compare patterns
    let (p1, p2) = DesignPatternsAdapter::compare_patterns(&doc, "Strategy", "State")
        .expect("Both Strategy and State should be found for comparison");
    assert_eq!(p1.name, "Strategy");
    assert_eq!(p2.name, "State");
    assert!(p2.intent.unwrap().contains("internal state"));
}

/// A query the corpus cannot answer must produce no evidence at all.
///
/// Regression: every section unit used to clear the inclusion threshold on its
/// structural bonus alone (0.20 * 0.3 = 0.06 > 0.05), and pages cleared it on
/// embedding noise, so an off-topic question returned section openings carrying
/// real page numbers and real `[Doc: ... p. ... § ...]` citations.
#[test]
fn test_uncovered_query_yields_no_evidence_instead_of_cited_noise() {
    let doc = create_sample_design_pattern_doc();
    let retriever = HybridRetriever::build(&[doc], None);

    let err = retriever
        .search(
            "receta de paella valenciana con azafrán y garrofón",
            5,
            &HybridWeights::DEFAULT,
        )
        .expect_err("a cookery question must not retrieve from a design patterns book");

    assert!(
        err.absent_terms.contains(&"paella".to_string()),
        "the refusal must name the terms the corpus lacks, got: {:?}",
        err.absent_terms
    );
    assert!(
        err.best_matched_idf < err.required_idf,
        "no passage should have cleared the bar: {} vs {}",
        err.best_matched_idf,
        err.required_idf
    );

    let md = err.to_markdown();
    assert!(md.contains("Sin evidencia"));
    assert!(
        !md.contains("[Doc:"),
        "a refusal must never carry a citation"
    );
}

/// The admission bar rises for absent terms, so a query mixing known words with
/// unknown ones is still refused rather than answered from the known ones alone.
#[test]
fn test_partially_matching_query_is_refused_when_key_terms_are_absent() {
    let doc = create_sample_design_pattern_doc();
    let retriever = HybridRetriever::build(&[doc], None);

    let result = retriever.search(
        "how to implement memoization with Rust procedural macros",
        5,
        &HybridWeights::DEFAULT,
    );
    assert!(
        result.is_err(),
        "matching only the filler words must not count as evidence"
    );
}

/// The structural score compares terms, not substrings.
///
/// Regression: `title.to_lowercase().contains(word)` scored "con" against
/// "Conceptos" and "de" against any title containing those letters, so Spanish
/// particles inflated the structural score of unrelated headings.
#[test]
fn test_structural_score_does_not_reward_substring_collisions() {
    let doc = create_sample_design_pattern_doc();
    let retriever = HybridRetriever::build(&[doc], None);

    let hits = retriever
        .search("strategy algorithms", 5, &HybridWeights::DEFAULT)
        .expect("the sample document covers this query");

    for hit in &hits {
        let title_words: Vec<String> = hit
            .title
            .to_lowercase()
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .collect();
        if hit.structural_score > 0.3 {
            assert!(
                title_words
                    .iter()
                    .any(|w| w == "strategy" || w == "algorithms"),
                "title '{}' scored {} structurally without containing a query term",
                hit.title,
                hit.structural_score
            );
        }
    }
}

/// The same corpus must return the very same index, not an equal one.
///
/// Building it costs ~537 ms against ~0.5 ms to search it, measured on a
/// 437-page manual, and it was being rebuilt once per MCP tool call.
#[test]
fn test_retriever_cache_returns_the_same_index_for_the_same_corpus() {
    use docugraph::retrieval::RetrieverCache;
    use std::sync::Arc;

    let docs = vec![create_sample_design_pattern_doc()];
    let cache = RetrieverCache::default();

    let first = cache.get_or_build(&docs);
    let second = cache.get_or_build(&docs);
    assert!(
        Arc::ptr_eq(&first, &second),
        "the second call must reuse the built index, not rebuild it"
    );
}

/// A different corpus is a different key: reindexed content must not be served
/// from the entry built for the old content.
#[test]
fn test_retriever_cache_misses_when_the_content_changes() {
    use docugraph::retrieval::{CorpusSignature, RetrieverCache};
    use std::sync::Arc;

    let docs = vec![create_sample_design_pattern_doc()];

    let mut edited = create_sample_design_pattern_doc();
    edited.metadata.content_hash = "a-different-content-hash".to_string();
    let edited = vec![edited];

    assert_ne!(
        CorpusSignature::of(&docs),
        CorpusSignature::of(&edited),
        "a new content hash must produce a new signature"
    );

    let cache = RetrieverCache::default();
    let first = cache.get_or_build(&docs);
    let after_edit = cache.get_or_build(&edited);
    assert!(
        !Arc::ptr_eq(&first, &after_edit),
        "edited content must not be served from the stale entry"
    );
}

/// The signature is a set, not a list: ordering the same documents differently
/// must not force a rebuild.
#[test]
fn test_corpus_signature_is_order_independent() {
    use docugraph::retrieval::CorpusSignature;

    let a = create_sample_design_pattern_doc();
    let mut b = create_sample_design_pattern_doc();
    b.metadata.id = "second-doc".to_string();
    b.metadata.content_hash = "second-hash".to_string();

    let forward = CorpusSignature::of(&[a.clone(), b.clone()]);
    let backward = CorpusSignature::of(&[b, a]);
    assert_eq!(forward, backward);
}

fn doc_with_rare_terms(id: &str, pages: usize, rare: &[&str]) -> Document {
    let mut doc = Document::new(DocumentMetadata {
        id: id.to_string(),
        title: "Spec".to_string(),
        total_pages: pages as u32,
        content_hash: format!("hash-{id}-{pages}"),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    for p in 1..=pages {
        let mut text = format!("Pagina {p} texto comun de relleno para el indice");
        for (i, word) in rare.iter().enumerate() {
            if p == 3 + i * 2 {
                text.push(' ');
                text.push_str(word);
            }
        }
        doc.add_page(Page::new(p as u32, &text));
    }
    doc
}

/// Admission must never refuse a query whose every term is in the corpus.
///
/// Regression: the bar is the mean of the per-term IDFs and a passage holding one
/// term carries exactly that IDF when all terms share a `df` - the normal case for
/// rare identifiers, all at df=1. Neither the sum nor the division is exact in
/// binary floating point, so an exact `>=` refused the passage about 9% of the
/// time. The refusal reported no absent terms and told the agent to rephrase a
/// query that was already right, while document_search returned the passages.
#[test]
fn test_admission_never_refuses_a_query_whose_terms_are_all_present() {
    const RARE: &[&str] = &["zeta", "kappa", "omega", "sigma", "delta", "gamma", "theta"];
    let mut refused = Vec::new();

    for nterms in 2..=7usize {
        let rare = &RARE[..nterms];
        let query = rare.join(" ");
        for pages in 20..=140usize {
            let doc = doc_with_rare_terms("spec", pages, rare);
            let retriever = HybridRetriever::build(&[doc], None);
            let lexical = retriever.bm25().search(&query, 10).len();
            let hybrid = retriever
                .search(&query, 10, &HybridWeights::DEFAULT)
                .map(|h| h.len())
                .unwrap_or(0);
            if lexical > 0 && hybrid == 0 {
                refused.push((nterms, pages));
            }
        }
    }

    assert!(
        refused.is_empty(),
        "the hybrid path refused {} corpora that plain BM25 matched, e.g. {:?}",
        refused.len(),
        &refused[..refused.len().min(5)]
    );
}

/// A passage carrying exactly the mean information of one query term is evidence.
#[test]
fn test_admits_is_inclusive_at_the_bar() {
    let doc = doc_with_rare_terms("tie", 26, &["zeta", "kappa", "omega"]);
    let index = docugraph::retrieval::Bm25Index::build_from_documents(&[doc], None);
    let profile = index.profile_query("zeta kappa omega");

    let single_term_mass = profile.terms[0].idf;
    assert!(
        profile.admits(single_term_mass),
        "carrying one of three equally rare terms is exactly the bar: mass {} vs floor {}",
        single_term_mass,
        profile.admission_floor()
    );
    assert!(
        !profile.admits(single_term_mass * 0.9),
        "the tolerance must not admit a passage that is genuinely below the bar"
    );
}

/// Identical calls must return identical results.
///
/// Regression: the admitted set came out of a HashMap, whose iteration order is
/// randomised per instance, and the comparator ordered by score alone. Tied units
/// therefore landed in a different order on every call and truncate() kept an
/// arbitrary subset of the tie. The repo requires determinism (8cc8a52).
#[test]
fn test_hybrid_search_is_deterministic_across_identical_calls() {
    let mut doc = Document::new(DocumentMetadata {
        id: "tied".to_string(),
        title: "Tied".to_string(),
        total_pages: 6,
        content_hash: "hash-tied".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    for p in 1..=6 {
        doc.add_page(Page::new(
            p,
            "procedimiento de calibracion identico en cada pagina",
        ));
    }

    let retriever = HybridRetriever::build(&[doc], None);
    let first: Vec<String> = retriever
        .search("procedimiento de calibracion", 3, &HybridWeights::DEFAULT)
        .expect("the corpus covers this query")
        .iter()
        .map(|h| h.unit_id.clone())
        .collect();

    for call in 1..40 {
        let again: Vec<String> = retriever
            .search("procedimiento de calibracion", 3, &HybridWeights::DEFAULT)
            .expect("the corpus covers this query")
            .iter()
            .map(|h| h.unit_id.clone())
            .collect();
        assert_eq!(again, first, "call {call} returned a different ordering");
    }
}

/// Build a small Spanish manual that keeps the English pattern names, the way a
/// real translated design-patterns book does.
fn spanish_manual_with_english_names() -> Document {
    let mut doc = Document::new(DocumentMetadata {
        id: "manual-es".to_string(),
        title: "Patrones de diseño".to_string(),
        total_pages: 4,
        content_hash: "hash-manual-es".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    doc.add_page(Page::new(
        1,
        "Observer. El patrón Observer define una dependencia uno a muchos: cuando el emisor \
         cambia de estado, los objetos que se subscribe reciben la notificación.",
    ));
    doc.add_page(Page::new(
        2,
        "Decorator. El patrón Decorator envuelve un objeto para añadir comportamiento sin herencia.",
    ));
    doc.add_page(Page::new(
        3,
        "Catalogo de patterns de comportamiento y su intención dentro del diseño orientado a objetos.",
    ));
    doc.add_page(Page::new(
        4,
        "Singleton. El patrón Singleton garantiza una unica instancia de la clase en el programa.",
    ));
    doc.sections
        .push(SectionNode::new("obs", "Observer", 1, 1, 1, None));
    doc
}

/// An English query about a topic the Spanish manual covers must not be refused
/// just because the query used an inflected form of a word the manual has.
///
/// Regression: `observer` is in the corpus but `pattern` and `subscribers` are
/// not - only `patterns` and `subscribe` are. The two absent forms took the
/// maximum IDF and lifted the bar past anything `observer` alone could supply, so
/// a covered question came back as "no evidence".
#[test]
fn test_inflected_query_terms_resolve_to_the_form_the_corpus_uses() {
    let retriever = HybridRetriever::build(&[spanish_manual_with_english_names()], None);

    let profile = retriever
        .bm25()
        .profile_query("Observer pattern subscribers");
    let resolved: Vec<&str> = profile.terms.iter().map(|t| t.term.as_str()).collect();
    assert!(
        resolved.contains(&"patterns"),
        "'pattern' must resolve to the corpus form 'patterns', got {resolved:?}"
    );
    assert!(
        resolved.contains(&"subscribe"),
        "'subscribers' must resolve to the corpus form 'subscribe', got {resolved:?}"
    );
    assert!(
        profile.absent_terms().is_empty(),
        "nothing should be reported absent here: {:?}",
        profile.absent_terms()
    );

    let hits = retriever
        .search("Observer pattern subscribers", 5, &HybridWeights::DEFAULT)
        .expect("the manual covers Observer");
    assert!(!hits.is_empty());
}

/// Resolution must never invent a match: a term with no form in the corpus stays
/// absent, so genuinely uncovered questions are still refused.
#[test]
fn test_absent_topics_do_not_resolve_to_anything() {
    let index = docugraph::retrieval::Bm25Index::build_from_documents(
        &[spanish_manual_with_english_names()],
        None,
    );

    for query in ["kubernetes ingress", "paella azafrán", "memoization macros"] {
        let profile = index.profile_query(query);
        assert_eq!(
            profile.absent_terms().len(),
            profile.terms.len(),
            "'{query}' must stay entirely absent, got {:?}",
            profile
                .terms
                .iter()
                .map(|t| (&t.term, t.df))
                .collect::<Vec<_>>()
        );
    }
}

/// Resolution must not reach a word that merely looks similar.
///
/// Regression: appending "es" turned the English "intent" into the Spanish verb
/// form "intentes", which answered the query through a word that has nothing to
/// do with it. Refusing is the honest outcome for a true translation.
#[test]
fn test_resolution_does_not_reach_an_unrelated_lookalike() {
    let mut doc = spanish_manual_with_english_names();
    doc.add_page(Page::new(
        5,
        "No intentes resolver el problema antes de comprenderlo por completo.",
    ));
    doc.metadata.total_pages = 5;

    let index = docugraph::retrieval::Bm25Index::build_from_documents(&[doc], None);
    let profile = index.profile_query("Strategy intent");
    let resolved: Vec<&str> = profile.terms.iter().map(|t| t.term.as_str()).collect();

    assert!(
        !resolved.contains(&"intentes"),
        "'intent' must not resolve to the unrelated verb form, got {resolved:?}"
    );
    assert!(
        profile.absent_terms().contains(&"intent"),
        "'intent' has no form in this corpus and must stay absent, got {resolved:?}"
    );
}

/// Ranking must score the same terms admission judged.
#[test]
fn test_ranking_uses_the_resolved_terms() {
    let index = docugraph::retrieval::Bm25Index::build_from_documents(
        &[spanish_manual_with_english_names()],
        None,
    );

    // "pattern" alone finds nothing: the corpus spells it "patterns".
    assert!(index.search("pattern", 5).is_empty());

    let profile = index.profile_query("pattern");
    let terms: Vec<String> = profile.terms.iter().map(|t| t.term.clone()).collect();
    assert!(
        !index.search_terms(&terms, 5).is_empty(),
        "ranking against the resolved terms must find the passage admission accepted"
    );
}

/// A manual whose one section spans five pages, with a distinctive term on each.
fn multi_page_section_doc() -> Document {
    let mut doc = Document::new(DocumentMetadata {
        id: "manual-spans".to_string(),
        title: "Manual".to_string(),
        total_pages: 5,
        content_hash: "hash-spans".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    for (page, marker) in [
        (1, "alfa"),
        (2, "bravo"),
        (3, "charlie"),
        (4, "delta"),
        (5, "eco"),
    ] {
        doc.add_page(Page::new(
            page,
            format!(
                "Pagina {page} del procedimiento de calibracion. El indicador {marker} \
                 se describe aqui con detalle suficiente para ser citado."
            ),
        ));
    }
    doc.sections.push(SectionNode::new(
        "proc",
        "Procedimiento de calibracion",
        1,
        1,
        5,
        None,
    ));
    doc
}

/// A citation must name the page its snippet is on, not the page its section
/// starts on.
///
/// Regression: a section unit is the concatenation of every page it spans, and
/// the hit carried only the section's first page. Measured on a 437-page manual,
/// 30% of multi-page section citations named a page the quoted text was not on -
/// specific, confident, and checkable in one click, which is the worst shape a
/// wrong citation can take for a tool that sells verifiable evidence.
#[test]
fn test_citation_names_the_page_the_snippet_is_on() {
    let retriever = HybridRetriever::build(&[multi_page_section_doc()], None);

    for (marker, expected_page) in [("bravo", 2), ("charlie", 3), ("delta", 4), ("eco", 5)] {
        let hits = retriever
            .search(marker, 10, &HybridWeights::DEFAULT)
            .unwrap_or_else(|_| panic!("'{marker}' is in the corpus"));

        let section_hit = hits
            .iter()
            .find(|h| h.section_id.is_some())
            .unwrap_or_else(|| panic!("the section unit must be admitted for '{marker}'"));

        assert_eq!(
            section_hit.page_start, 1,
            "the section still starts on page 1"
        );
        assert_eq!(
            section_hit.snippet_page, expected_page,
            "'{marker}' is on page {expected_page}, so that is the page to cite; \
             got p.{} with snippet: {}",
            section_hit.snippet_page, section_hit.snippet
        );
    }
}

/// The rendered citation carries that page, not the section's first one.
#[test]
fn test_evidence_markdown_cites_the_snippet_page() {
    let docs = vec![multi_page_section_doc()];
    let retriever = HybridRetriever::build(&docs, None);
    let hits = retriever
        .search("delta", 5, &HybridWeights::DEFAULT)
        .expect("the corpus covers this");

    let budget = ContextBudget {
        max_tokens: 500,
        max_chunks: 3,
        compact: true,
    };
    let markdown = ContextBuilder::build_evidence("delta", &hits, budget).to_markdown();

    assert!(
        markdown.contains("p. 4"),
        "the citation must name page 4, where 'delta' is: {markdown}"
    );
}

/// A page unit's snippet page is simply its page, and must stay that way.
#[test]
fn test_page_units_cite_their_own_page() {
    let index =
        docugraph::retrieval::Bm25Index::build_from_documents(&[multi_page_section_doc()], None);
    let hits = index.search("charlie", 10);

    let page_hit = hits
        .iter()
        .find(|h| h.section_id.is_none())
        .expect("the page unit must be found");
    assert_eq!(page_hit.snippet_page, page_hit.page_start);
    assert_eq!(page_hit.snippet_page, 3);
}

/// Asking for fewer results must return a prefix of asking for more.
///
/// Regression: BM25 was scored only over a window of `(limit * 3).max(20)`
/// candidates, so an admitted unit outside it took `bm25_score = 0.0` and was
/// ranked on a character n-gram hash plus a constant. The top 5 came back as a
/// different ordering of the top 20 rather than its prefix, and the `bm25_score`
/// returned for explainability was false for those hits.
#[test]
fn test_smaller_limits_return_a_prefix_of_larger_ones() {
    let mut doc = Document::new(DocumentMetadata {
        id: "wide".to_string(),
        title: "Wide".to_string(),
        total_pages: 60,
        content_hash: "hash-wide".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    // Enough pages mentioning the term that the admitted set exceeds any window.
    for page in 1..=60u32 {
        doc.add_page(Page::new(
            page,
            format!(
                "Pagina {page}. El procedimiento de calibracion del sensor se describe \
                 con detalle. Repeticion {page} del termino calibracion."
            ),
        ));
    }
    doc.sections.push(SectionNode::new(
        "todo",
        "Procedimiento de calibracion",
        1,
        1,
        60,
        None,
    ));

    let retriever = HybridRetriever::build(&[doc], None);
    let ids = |n: usize| -> Vec<String> {
        retriever
            .search(
                "procedimiento calibracion sensor",
                n,
                &HybridWeights::DEFAULT,
            )
            .expect("the corpus covers this")
            .iter()
            .map(|h| h.unit_id.clone())
            .collect()
    };

    let wide = ids(40);
    for narrow_limit in [1, 3, 5, 10, 20] {
        let narrow = ids(narrow_limit);
        assert!(
            wide.len() >= narrow.len() && wide[..narrow.len()] == narrow[..],
            "limit={narrow_limit} returned {narrow:?}, which is not a prefix of limit=40"
        );
    }
}

/// Every returned hit reports the BM25 score it was actually ranked with.
#[test]
fn test_reported_bm25_score_is_real() {
    let mut doc = Document::new(DocumentMetadata {
        id: "scores".to_string(),
        title: "Scores".to_string(),
        total_pages: 40,
        content_hash: "hash-scores".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    for page in 1..=40u32 {
        doc.add_page(Page::new(
            page,
            format!("Pagina {page} sobre el procedimiento de calibracion del sensor."),
        ));
    }
    doc.sections
        .push(SectionNode::new("s", "Calibracion", 1, 1, 40, None));

    let retriever = HybridRetriever::build(&[doc], None);
    let hits = retriever
        .search("procedimiento calibracion", 30, &HybridWeights::DEFAULT)
        .expect("the corpus covers this");

    assert!(
        hits.len() > 20,
        "the admitted set must exceed the old window"
    );
    for hit in &hits {
        assert!(
            hit.bm25_score > 0.0,
            "'{}' matched the query but reports bm25_score {}",
            hit.unit_id,
            hit.bm25_score
        );
    }
}

/// A missing accent must not turn a covered question into "no evidence".
///
/// Regression: an absent term takes the maximum IDF and the admission bar is the
/// mean of the query's term IDFs, so one dropped tilde lifted the bar above
/// anything the corpus could supply. `intención del patrón Strategy` returned
/// passages while `intencion del patron Strategy` was refused outright - the
/// strongest verdict the system can give, about a document that contains the text.
#[test]
fn test_accents_are_folded_on_both_sides() {
    let mut doc = Document::new(DocumentMetadata {
        id: "es-doc".to_string(),
        title: "Manual".to_string(),
        total_pages: 2,
        content_hash: "hash-es".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    doc.add_page(Page::new(
        1,
        "La intención del patrón Strategy es definir una familia de algoritmos \
         intercambiables dentro del diseño orientado a objetos.",
    ));
    doc.add_page(Page::new(
        2,
        "La configuración se describe en la sección de implementación.",
    ));
    doc.sections
        .push(SectionNode::new("s", "Intención", 1, 1, 2, None));

    let retriever = HybridRetriever::build(&[doc], None);
    for query in [
        "intención del patrón Strategy",
        "intencion del patron Strategy",
        "diseno orientado a objetos",
        "configuracion",
    ] {
        let hits = retriever
            .search(query, 5, &HybridWeights::DEFAULT)
            .unwrap_or_else(|_| panic!("'{query}' is covered by this document"));
        assert!(!hits.is_empty(), "'{query}' returned nothing");
    }
}

/// Typographic ligatures in the source must match a query that spells them out.
#[test]
fn test_ligatures_match_their_spelled_out_form() {
    let mut doc = Document::new(DocumentMetadata {
        id: "lig".to_string(),
        title: "Ligatures".to_string(),
        total_pages: 1,
        content_hash: "hash-lig".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    // What a LaTeX or InDesign PDF actually stores for "configuration flags".
    doc.add_page(Page::new(
        1,
        "The con\u{FB01}guration \u{FB02}ags control the sensor calibration.",
    ));
    doc.sections
        .push(SectionNode::new("s", "Setup", 1, 1, 1, None));

    let retriever = HybridRetriever::build(&[doc], None);
    let hits = retriever
        .search("configuration flags", 5, &HybridWeights::DEFAULT)
        .expect("the ligature forms must match the spelled-out query");
    assert!(!hits.is_empty());
}

/// `ñ` is a letter, not an accented `n`, and must stay distinct.
#[test]
fn test_enye_is_not_folded_into_n() {
    use docugraph::retrieval::Bm25Index;

    let mut doc = Document::new(DocumentMetadata {
        id: "enye".to_string(),
        title: "Enye".to_string(),
        total_pages: 1,
        content_hash: "hash-enye".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    doc.add_page(Page::new(
        1,
        "El año pasado se revisó el diseño. El ano es otra cosa completamente distinta.",
    ));
    doc.sections
        .push(SectionNode::new("s", "Texto", 1, 1, 1, None));

    let index = Bm25Index::build_from_documents(&[doc], None);
    let profile = index.profile_query("año");
    assert_eq!(
        profile.terms[0].term, "año",
        "folding ñ would make año and ano the same term"
    );
}

/// An interrogative carries no topic and must not raise the admission bar.
///
/// Regression: "what" and "does" are absent from a Spanish corpus, so they took
/// the maximum IDF and lifted the bar past what the real terms could supply.
#[test]
fn test_question_words_do_not_block_a_covered_query() {
    let mut doc = Document::new(DocumentMetadata {
        id: "qa".to_string(),
        title: "QA".to_string(),
        total_pages: 1,
        content_hash: "hash-qa".to_string(),
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        ..Default::default()
    });
    doc.add_page(Page::new(
        1,
        "El patrón Strategy define una familia de algoritmos y los hace intercambiables.",
    ));
    doc.sections
        .push(SectionNode::new("s", "Strategy", 1, 1, 1, None));

    let retriever = HybridRetriever::build(&[doc], None);
    let hits = retriever
        .search("What does Strategy do", 5, &HybridWeights::DEFAULT)
        .expect("the question words must not veto a covered topic");
    assert!(!hits.is_empty());
}

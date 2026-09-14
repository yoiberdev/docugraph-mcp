use docugraph::document::model::{Document, DocumentMetadata, Page, SectionNode};
use docugraph::knowledge::design_patterns::DesignPatternsAdapter;
use docugraph::retrieval::{
    Bm25Index, ContextBudget, ContextBuilder, DeterministicSubwordEmbedding, EmbeddingProvider,
    HybridRetriever,
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
    let retriever = HybridRetriever::build(&[doc], None, None);

    let hits = retriever.search("encapsulate interchangeable algorithms", 3);
    assert!(!hits.is_empty());
    assert!(hits[0].final_score > 0.3);
    assert!(hits[0].bm25_score > 0.0);
    assert!(hits[0].semantic_score > 0.0);
    assert!(hits[0].title == "Strategy" || hits[0].title == "Intent");
}

#[test]
fn test_context_budgeter_and_evidence() {
    let doc = create_sample_design_pattern_doc();
    let retriever = HybridRetriever::build(&[doc], None, None);
    let hits = retriever.search("interchangeable algorithms", 5);

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

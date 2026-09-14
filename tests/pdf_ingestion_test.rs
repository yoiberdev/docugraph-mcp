use docugraph::document::{
    DocumentId, Page, Provenance, load_pdf_from_path,
    structure::{detect_heading_level, infer_sections_from_pages, slugify_title},
};

#[test]
fn test_heading_detection_heuristics() {
    // Numbered patterns
    assert_eq!(
        detect_heading_level("1. Introduction"),
        Some((1, "Introduction"))
    );
    assert_eq!(
        detect_heading_level("1.2 Background"),
        Some((1, "Background"))
    );
    assert_eq!(
        detect_heading_level("1.2.3 Deep Detail"),
        Some((2, "Deep Detail"))
    );

    // Keyword patterns
    assert_eq!(
        detect_heading_level("Chapter 4: Architectural Styles"),
        Some((1, "Chapter 4: Architectural Styles"))
    );
    assert_eq!(
        detect_heading_level("Capítulo 2: Primeros Principios"),
        Some((1, "Capítulo 2: Primeros Principios"))
    );

    // Markdown syntax
    assert_eq!(detect_heading_level("# Top Level"), Some((1, "Top Level")));
    assert_eq!(
        detect_heading_level("## Second Level"),
        Some((2, "Second Level"))
    );
    assert_eq!(
        detect_heading_level("### Third Level"),
        Some((3, "Third Level"))
    );

    // All-caps headers (common in Gang of Four & RFCs)
    assert_eq!(detect_heading_level("INTENT"), Some((2, "INTENT")));
    assert_eq!(
        detect_heading_level("APPLICABILITY"),
        Some((2, "APPLICABILITY"))
    );

    // Regular prose sentences must NOT be detected as headings
    assert_eq!(
        detect_heading_level("This is an ordinary sentence explaining a concept."),
        None
    );
    assert_eq!(
        detect_heading_level("Another paragraph line that happens to mention Chapter 1 casually."),
        None
    );
    assert_eq!(detect_heading_level(""), None);
}

#[test]
fn test_slugify_title() {
    assert_eq!(slugify_title("Strategy Pattern"), "strategy-pattern");
    assert_eq!(
        slugify_title("1.2.3 Microservices & Event-Driven!"),
        "1-2-3-microservices-event-driven"
    );
    assert_eq!(slugify_title(""), "section");
}

#[test]
fn test_infer_sections_hierarchy() {
    let pages = vec![
        Page::new(
            1,
            "Chapter 1: Getting Started\nThis is introductory body text.\n## 1.1 Architecture\nDetails about architecture.",
        ),
        Page::new(
            2,
            "### 1.1.1 Subsystem Design\nDeep dive into subsystem.\n## 1.2 Summary\nConclusion of chapter.",
        ),
    ];

    let sections = infer_sections_from_pages(&pages);
    assert_eq!(sections.len(), 1, "Should have 1 top-level chapter");

    let ch1 = &sections[0];
    assert_eq!(ch1.level, 1);
    assert_eq!(ch1.title, "Chapter 1: Getting Started");
    assert_eq!(
        ch1.children.len(),
        2,
        "Chapter 1 should contain 1.1 and 1.2"
    );

    let sec_1_1 = &ch1.children[0];
    assert_eq!(sec_1_1.title, "1.1 Architecture");
    assert_eq!(sec_1_1.children.len(), 1, "1.1 should contain 1.1.1");
    assert_eq!(sec_1_1.children[0].title, "1.1.1 Subsystem Design");

    assert_eq!(
        ch1.total_count(),
        4,
        "Total section count (Ch1 + 1.1 + 1.1.1 + 1.2)"
    );
}

#[test]
fn test_provenance_citation_formatting() {
    let prov = Provenance::new(DocumentId("gof-patterns".to_string()), 315)
        .with_section("strategy-intent")
        .with_offset(142);

    let citation = prov.format_citation();
    assert_eq!(
        citation,
        "[Doc: gof-patterns, Page: 315, Section: strategy-intent, Offset: 142]"
    );
}

#[test]
fn test_real_pdf_loading_and_extraction() {
    let sample_pdf_path = "C:/proyectos/chaika-stage/docs/aprendiendo-git-pdf.pdf";
    if !std::path::Path::new(sample_pdf_path).exists() {
        eprintln!("Sample PDF not found; skipping real PDF loading test");
        return;
    }

    let doc = load_pdf_from_path(sample_pdf_path).expect("PDF should parse without error");

    assert!(doc.metadata.total_pages > 200, "Should have over 200 pages");
    assert!(
        !doc.metadata.content_hash.is_empty(),
        "SHA-256 hash must be present"
    );
    assert!(!doc.pages.is_empty(), "Extracted pages should not be empty");
    assert!(
        !doc.sections.is_empty(),
        "Document sections should be generated"
    );

    let page_1 = doc.get_page(1).expect("Page 1 must exist");
    assert_eq!(page_1.page_number, 1);

    println!(
        "Verified real PDF: '{}' with {} pages and {} sections",
        doc.metadata.title,
        doc.metadata.total_pages,
        doc.total_sections()
    );
}

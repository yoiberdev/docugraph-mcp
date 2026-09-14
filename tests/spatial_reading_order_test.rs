use docugraph::document::layout::{
    BoundingBox, Matrix2D, TextFragment, extract_page_text_spatial, select_reading_order_strategy,
};
use docugraph::document::load_pdf_from_path;
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use tempfile::NamedTempFile;

fn create_pdf_with_operations(ops: Vec<Operation>) -> (Document, (u32, u16)) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });

    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => font_id,
        },
    });

    let content = Content { operations: ops };
    let content_bytes = content.encode().expect("Content encoding should succeed");
    let content_id = doc.add_object(Stream::new(dictionary! {}, content_bytes));

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    (doc, page_id)
}

fn tm_op(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Operation {
    Operation::new(
        "Tm",
        vec![
            Object::Real(a),
            Object::Real(b),
            Object::Real(c),
            Object::Real(d),
            Object::Real(e),
            Object::Real(f),
        ],
    )
}

#[test]
fn test_matrix_2d_and_bounding_box_primitives() {
    let id = Matrix2D::IDENTITY;
    let (x, y) = id.transform_point(10.0, 20.0);
    assert_eq!((x, y), (10.0, 20.0));

    // Translation matrix
    let trans = Matrix2D {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 50.0,
        f: 100.0,
    };
    let (tx, ty) = trans.transform_point(5.0, 10.0);
    assert_eq!((tx, ty), (55.0, 110.0));

    // Composition
    let combined = trans.multiply(&Matrix2D {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 10.0,
        f: -20.0,
    });
    let (cx, cy) = combined.transform_point(0.0, 0.0);
    assert_eq!((cx, cy), (60.0, 80.0));

    // BoundingBox union
    let b1 = BoundingBox::new(10.0, 20.0, 30.0, 40.0);
    let b2 = BoundingBox::new(25.0, 15.0, 50.0, 10.0);
    let u = b1.union(&b2);
    assert_eq!(u.x_min(), 10.0);
    assert_eq!(u.x_max(), 75.0);
    assert_eq!(u.y_min(), 15.0);
    assert_eq!(u.y_max(), 60.0);
}

#[test]
fn test_multi_column_gutter_detection_strategy() {
    // 1. Synthetic fragments forming 2 distinct columns with a 50pt gutter (X in [180, 230])
    let mut fragments = Vec::new();

    // Left column: X in [50, 180]
    for i in 0..6 {
        fragments.push(TextFragment {
            bbox: BoundingBox::new(50.0, 700.0 - (i as f32 * 20.0), 120.0, 12.0),
            text: format!("Left column paragraph text line {}", i),
        });
    }

    // Right column: X in [240, 370]
    for i in 0..6 {
        fragments.push(TextFragment {
            bbox: BoundingBox::new(240.0, 700.0 - (i as f32 * 20.0), 120.0, 12.0),
            text: format!("Right column paragraph text line {}", i),
        });
    }

    let strategy = select_reading_order_strategy(&fragments);
    assert!(
        strategy.is_multi_column(),
        "Should detect multi-column layout when two columns have a distinct gutter"
    );

    // 2. Uniformly distributed single-column fragments
    let mut single_col_frags = Vec::new();
    for i in 0..8 {
        single_col_frags.push(TextFragment {
            bbox: BoundingBox::new(50.0, 700.0 - (i as f32 * 25.0), 400.0, 12.0),
            text: format!("Full width single column paragraph text {}", i),
        });
    }
    let single_strategy = select_reading_order_strategy(&single_col_frags);
    assert!(
        !single_strategy.is_multi_column(),
        "Should detect single column when text is full width"
    );
}

#[test]
fn test_multi_column_synthetic_interleaved_reading_order() {
    // Construct a PDF where operations are deliberately emitted interleaved between Column 1 and Column 2:
    // C1-L1, C2-L1, C1-L2, C2-L2, C1-L3, C2-L3
    // Standard naive extraction would produce: C1-L1 C2-L1 C1-L2 C2-L2 C1-L3 C2-L3 (garbled).
    // DocuGraph spatial extraction MUST produce: C1-L1, C1-L2, C1-L3, C2-L1, C2-L2, C2-L3.

    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 10.0.into()]),
        // Row 1
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 700.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Col1-Line1: High speed Rust.")],
        ),
        tm_op(1.0, 0.0, 0.0, 1.0, 320.0, 700.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Col2-Line1: Results evaluated.")],
        ),
        // Row 2
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 670.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal(
                "Col1-Line2: Deterministic low memory.",
            )],
        ),
        tm_op(1.0, 0.0, 0.0, 1.0, 320.0, 670.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Col2-Line2: Benchmarks are solid.")],
        ),
        // Row 3
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 640.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Col1-Line3: End of column one.")],
        ),
        tm_op(1.0, 0.0, 0.0, 1.0, 320.0, 640.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Col2-Line3: Final conclusions.")],
        ),
        Operation::new("ET", vec![]),
    ];

    let (mut doc, page_id) = create_pdf_with_operations(ops);

    // Direct spatial extraction test
    let spatial_opt = extract_page_text_spatial(&doc, page_id, true);
    assert!(
        spatial_opt.is_some(),
        "Spatial multi-column extraction should activate for 2-column layout"
    );
    let spatial_text = spatial_opt.unwrap();

    let pos_c1_l1 = spatial_text
        .find("Col1-Line1")
        .expect("Col1-Line1 must exist");
    let pos_c1_l2 = spatial_text
        .find("Col1-Line2")
        .expect("Col1-Line2 must exist");
    let pos_c1_l3 = spatial_text
        .find("Col1-Line3")
        .expect("Col1-Line3 must exist");

    let pos_c2_l1 = spatial_text
        .find("Col2-Line1")
        .expect("Col2-Line1 must exist");
    let pos_c2_l2 = spatial_text
        .find("Col2-Line2")
        .expect("Col2-Line2 must exist");
    let pos_c2_l3 = spatial_text
        .find("Col2-Line3")
        .expect("Col2-Line3 must exist");

    // All of Column 1 must appear strictly before any part of Column 2
    assert!(pos_c1_l1 < pos_c1_l2, "Col1-Line1 must precede Col1-Line2");
    assert!(pos_c1_l2 < pos_c1_l3, "Col1-Line2 must precede Col1-Line3");
    assert!(
        pos_c1_l3 < pos_c2_l1,
        "Crucial: Col1-Line3 must precede Col2-Line1 (Column 1 completes before Column 2 starts)"
    );
    assert!(pos_c2_l1 < pos_c2_l2, "Col2-Line1 must precede Col2-Line2");
    assert!(pos_c2_l2 < pos_c2_l3, "Col2-Line2 must precede Col2-Line3");

    // End-to-end load_pdf_from_path test with tempfile
    let mut temp_file = NamedTempFile::new().unwrap();
    doc.save_to(&mut temp_file).unwrap();

    let parsed_doc = load_pdf_from_path(temp_file.path()).unwrap();
    let page_text = &parsed_doc.pages[0].text;

    let p_c1_end = page_text
        .find("Col1-Line3")
        .expect("Col1-Line3 in parsed page");
    let p_c2_start = page_text
        .find("Col2-Line1")
        .expect("Col2-Line1 in parsed page");
    assert!(
        p_c1_end < p_c2_start,
        "End-to-end parser must preserve multi-column reading order"
    );
}

#[test]
fn test_multi_column_with_spanning_title_and_footer() {
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 14.0.into()]),
        // Spanning title at top of page (Y=760, X=50..500)
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 760.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal(
                "Document Title: Universal Document Knowledge Graph Engine for Multi-Agent AI Systems",
            )],
        ),
        // Switch font for body
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 10.0.into()]),
        // Column 1 line
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 680.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Left column introductory design.")],
        ),
        // Column 2 line (interleaved)
        tm_op(1.0, 0.0, 0.0, 1.0, 330.0, 680.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Right column analysis and speed.")],
        ),
        // Column 1 second line
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 650.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Left column further details.")],
        ),
        // Column 2 second line
        tm_op(1.0, 0.0, 0.0, 1.0, 330.0, 650.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Right column summary notes.")],
        ),
        // Spanning footer at bottom of page (Y=80, X=50..500)
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 80.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal(
                "Footer Notice: Copyright (c) 2026 DocuGraph MCP Project. All rights reserved.",
            )],
        ),
        Operation::new("ET", vec![]),
    ];

    let (doc, page_id) = create_pdf_with_operations(ops);

    let spatial_text = extract_page_text_spatial(&doc, page_id, true)
        .expect("Should extract 2-column layout with spanning headers");

    let p_title = spatial_text.find("Document Title").unwrap();
    let p_left_first = spatial_text.find("Left column introductory").unwrap();
    let p_left_second = spatial_text.find("Left column further details").unwrap();
    let p_right_first = spatial_text.find("Right column analysis").unwrap();
    let p_right_second = spatial_text.find("Right column summary notes").unwrap();
    let p_footer = spatial_text.find("Footer Notice").unwrap();

    // Human reading order: Title -> Left Col 1 -> Left Col 2 -> Right Col 1 -> Right Col 2 -> Footer
    assert!(p_title < p_left_first, "Title must appear before Left Col");
    assert!(
        p_left_first < p_left_second,
        "Left Col 1 must appear before Left Col 2"
    );
    assert!(
        p_left_second < p_right_first,
        "Left Col must complete before Right Col starts"
    );
    assert!(
        p_right_first < p_right_second,
        "Right Col 1 must appear before Right Col 2"
    );
    assert!(
        p_right_second < p_footer,
        "Footer must appear at the end after all columns"
    );
}

#[test]
fn test_single_column_preserves_linear_flow() {
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.0.into()]),
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 700.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Single column paragraph line 1.")],
        ),
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 670.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Single column paragraph line 2.")],
        ),
        tm_op(1.0, 0.0, 0.0, 1.0, 50.0, 640.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Single column paragraph line 3.")],
        ),
        Operation::new("ET", vec![]),
    ];

    let (mut doc, page_id) = create_pdf_with_operations(ops);

    // With only_if_multi_column: true, single-column document returns None, delegating to default stream extraction
    let spatial_only_multi = extract_page_text_spatial(&doc, page_id, true);
    assert!(
        spatial_only_multi.is_none(),
        "Should return None for single column when only_if_multi_column is true"
    );

    let mut temp_file = NamedTempFile::new().unwrap();
    doc.save_to(&mut temp_file).unwrap();

    let parsed_doc = load_pdf_from_path(temp_file.path()).unwrap();
    let text = &parsed_doc.pages[0].text;
    assert!(text.contains("Single column paragraph line 1."));
    assert!(text.contains("Single column paragraph line 2."));
    assert!(text.contains("Single column paragraph line 3."));
}

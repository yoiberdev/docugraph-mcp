use docugraph::document::load_pdf_from_path;
use docugraph::document::table::{
    MarkdownTableBuilder, TableAlignment, reconstruct_tables_in_text, split_line_into_cells,
};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use tempfile::NamedTempFile;

fn create_pdf_with_operations(ops: Vec<Operation>) -> Document {
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

    doc
}

fn tm_op(x: f32, y: f32) -> Operation {
    Operation::new(
        "Tm",
        vec![
            Object::Real(1.0),
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(1.0),
            Object::Real(x),
            Object::Real(y),
        ],
    )
}

#[test]
fn test_markdown_table_builder_basic_and_escaping() {
    let mut builder = MarkdownTableBuilder::new();
    builder
        .set_headers(vec![
            "Method".to_string(),
            "Path".to_string(),
            "Types | Format".to_string(),
            "Status".to_string(),
        ])
        .set_alignments(vec![
            TableAlignment::Left,
            TableAlignment::Center,
            TableAlignment::Right,
            TableAlignment::Left,
        ])
        .add_row(vec![
            "GET".to_string(),
            "/api/docs".to_string(),
            "JSON | XML".to_string(),
            "200 OK".to_string(),
        ])
        .add_row(vec![
            "POST".to_string(),
            "/api/query".to_string(),
            "Stream\nBinary".to_string(),
            "201 Created".to_string(),
        ]);

    let table = builder.build();

    // Verify pipe escaping inside cells
    assert!(
        table.contains("Types \\| Format"),
        "Header pipe must be escaped"
    );
    assert!(
        table.contains("JSON \\| XML"),
        "Row cell pipe must be escaped"
    );

    // Verify newline normalization inside cells
    assert!(
        !table.contains("Stream\nBinary"),
        "Newlines inside cell should be collapsed to spaces"
    );
    assert!(
        table.contains("Stream Binary"),
        "Cell with newline should become single-line"
    );

    // Verify alignment separators
    assert!(
        table.contains("|---|:---:|---:|---|"),
        "Separators must match specified column alignments"
    );
}

#[test]
fn test_split_line_into_cells() {
    // 1. Tab separated
    let tabs = "ID\tTitle\tStatus";
    assert_eq!(split_line_into_cells(tabs), vec!["ID", "Title", "Status"]);

    // 2. Multi-space separated (2+ spaces between columns)
    let spaces = "Param     Type      Default     Description";
    assert_eq!(
        split_line_into_cells(spaces),
        vec!["Param", "Type", "Default", "Description"]
    );

    // 3. Single space prose (must NOT split into multiple cells)
    let sentence = "This is a regular prose paragraph without tabular alignment.";
    let cells = split_line_into_cells(sentence);
    assert_eq!(
        cells.len(),
        1,
        "Single-space sentences must yield exactly 1 cell"
    );

    // 4. Pre-existing pipe line
    let pipe_line = "| Col A | Col B | Col C |";
    assert_eq!(
        split_line_into_cells(pipe_line),
        vec!["Col A", "Col B", "Col C"]
    );
}

#[test]
fn test_table_structure_visitor_inline_table() {
    let input = r#"
System API Reference Documentation

Method    Path           Description
GET       /v1/ping       Health check and heartbeat endpoint
POST      /v1/search     Perform hybrid BM25 and vector search
DELETE    /v1/cache      Invalidate knowledge graph document cache

Please refer to the authentication guide before making requests.
"#;

    let output = reconstruct_tables_in_text(input);

    // Assert that table was converted to GFM
    assert!(
        output.contains("| Method | Path | Description |"),
        "Header row must be formatted with GFM pipes"
    );
    assert!(
        output.contains("|---|---|---|"),
        "Separator row must be present"
    );
    assert!(
        output.contains("| GET | /v1/ping | Health check and heartbeat endpoint |"),
        "First data row must be formatted"
    );
    assert!(
        output.contains("| POST | /v1/search | Perform hybrid BM25 and vector search |"),
        "Second data row must be formatted"
    );
    assert!(
        output.contains("| DELETE | /v1/cache | Invalidate knowledge graph document cache |"),
        "Third data row must be formatted"
    );

    // Assert surrounding text is fully preserved
    assert!(
        output.contains("System API Reference Documentation"),
        "Preceding text must be preserved"
    );
    assert!(
        output.contains("Please refer to the authentication guide before making requests."),
        "Trailing text must be preserved"
    );
}

#[test]
fn test_prose_with_indentation_not_falsely_converted() {
    let input = r#"
Overview of Architecture:
  Here is an indented paragraph that provides some detailed explanation.
  It spans two lines with leading spaces for indentation.
The conclusion follows immediately.
"#;

    let output = reconstruct_tables_in_text(input);

    assert!(
        !output.contains("|---|"),
        "Prose with indentation must NOT be converted to a table"
    );
    assert!(
        output.contains("Overview of Architecture:"),
        "Content must remain intact"
    );
}

#[test]
fn test_pdf_loader_with_table_end_to_end() {
    // Construct a PDF document containing a 3-column table
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.0.into()]),
        // Header
        tm_op(50.0, 700.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Metric    Target    Achieved")],
        ),
        Operation::new("ET", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.0.into()]),
        // Row 1
        tm_op(50.0, 670.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Latency   < 15ms    4.2ms")],
        ),
        Operation::new("ET", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.0.into()]),
        // Row 2
        tm_op(50.0, 640.0),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Memory    < 25MB    18.5MB")],
        ),
        Operation::new("ET", vec![]),
    ];

    let mut doc = create_pdf_with_operations(ops);

    let mut temp_file = NamedTempFile::new().unwrap();
    doc.save_to(&mut temp_file).unwrap();

    let parsed = load_pdf_from_path(temp_file.path()).unwrap();
    let text = &parsed.pages[0].text;

    assert!(
        text.contains("| Metric | Target | Achieved |"),
        "Extracted page text must contain GFM table header"
    );
    assert!(
        text.contains("|---|---|---|"),
        "Extracted page text must contain GFM table separator"
    );
    assert!(
        text.contains("| Latency | < 15ms | 4.2ms |"),
        "Extracted page text must contain formatted Row 1"
    );
    assert!(
        text.contains("| Memory | < 25MB | 18.5MB |"),
        "Extracted page text must contain formatted Row 2"
    );
}

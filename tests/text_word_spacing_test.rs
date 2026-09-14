//! Word spacing in extracted text for PDFs that place words by position instead of
//! space glyphs: TJ arrays with large negative adjustments and Td/Tm moves between
//! words (as emitted by Prince, XeTeX and similar engines).

use docugraph::document::layout::{BoundingBox, TextFragment, join_fragments_in_stream_order};
use docugraph::document::load_pdf_from_path;
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use tempfile::NamedTempFile;

/// Build a one-page PDF whose font /F1 declares explicit glyph widths:
/// every printable glyph is 500/1000 em wide and the space glyph 250/1000 em.
fn load_page_text(ops: Vec<Operation>) -> String {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let widths: Vec<Object> = (32..=126)
        .map(|code| Object::Integer(if code == 32 { 250 } else { 500 }))
        .collect();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "TrueType",
        "BaseFont" => "DocuGraphTestSans",
        "FirstChar" => 32,
        "LastChar" => 126,
        "Widths" => widths,
        "Encoding" => "WinAnsiEncoding",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let content = Content { operations: ops };
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        content.encode().expect("content encoding should succeed"),
    ));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut temp_file = NamedTempFile::new().unwrap();
    doc.save_to(&mut temp_file).unwrap();
    let parsed = load_pdf_from_path(temp_file.path()).expect("synthetic PDF should load");
    assert!(
        !parsed.pages[0].untrusted_text_detected,
        "synthetic page must not trip the hidden text detector"
    );
    parsed.pages[0].text.clone()
}

fn op(operator: &str, operands: Vec<Object>) -> Operation {
    Operation::new(operator, operands)
}

fn text(s: &str) -> Object {
    Object::string_literal(s)
}

fn font(size: f32) -> Operation {
    op("Tf", vec![Object::Name(b"F1".to_vec()), size.into()])
}

fn td(tx: f32, ty: f32) -> Operation {
    op("Td", vec![tx.into(), ty.into()])
}

fn tm(scale: f32, x: f32, y: f32) -> Operation {
    op(
        "Tm",
        vec![
            scale.into(),
            0.into(),
            0.into(),
            scale.into(),
            x.into(),
            y.into(),
        ],
    )
}

#[test]
fn test_tj_adjustments_become_word_spaces() {
    let ops = vec![
        op("BT", vec![]),
        font(12.0),
        td(50.0, 700.0),
        // Word gaps as real and integer adjustments, with no space glyph in the strings
        op(
            "TJ",
            vec![Object::Array(vec![
                text("Sin"),
                Object::Real(-266.24),
                text("glifo"),
                Object::Integer(-300),
                text("de"),
                Object::Real(-317.93),
                text("espacio"),
            ])],
        ),
        op("ET", vec![]),
        op("BT", vec![]),
        font(12.0),
        td(50.0, 670.0),
        // Kerning adjustments inside a word must not split it
        op(
            "TJ",
            vec![Object::Array(vec![
                text("Pa"),
                Object::Integer(40),
                text("tro"),
                Object::Real(-15.5),
                text("nes"),
            ])],
        ),
        // A word continued by a second TJ right where the first one ends
        td(0.0, -30.0),
        op("TJ", vec![Object::Array(vec![text("crea")])]),
        op("TJ", vec![Object::Array(vec![text("cionales")])]),
        op("ET", vec![]),
    ];

    let page_text = load_page_text(ops);
    let lines: Vec<&str> = page_text.lines().collect();
    assert_eq!(
        lines,
        vec!["Sin glifo de espacio", "Patrones", "creacionales"],
        "unexpected text: {page_text:?}"
    );
}

#[test]
fn test_td_moves_between_words_become_word_spaces() {
    // At 12pt every glyph advances 6pt, so "Hola" ends 24pt after its origin.
    let ops = vec![
        op("BT", vec![]),
        font(12.0),
        td(50.0, 700.0),
        op("Tj", vec![text("Hola")]),
        td(27.0, 0.0), // 3pt gap: word space
        op("Tj", vec![text("mundo")]),
        td(33.0, 0.0), // 3pt gap: word space
        op("Tj", vec![text("sin")]),
        td(21.0, 0.0), // 3pt gap: word space
        op("Tj", vec![text("espa")]),
        td(24.5, 0.0), // 0.5pt gap: same word
        op("Tj", vec![text("cios")]),
        op("ET", vec![]),
        // One word per text object, placed with absolute moves on the next line
        op("BT", vec![]),
        font(12.0),
        td(50.0, 680.0),
        op("Tj", vec![text("Otra")]),
        op("ET", vec![]),
        op("BT", vec![]),
        font(12.0),
        td(77.0, 680.0),
        op("Tj", vec![text("linea")]),
        op("ET", vec![]),
        // Punctuation shown separately right after the word
        op("BT", vec![]),
        font(12.0),
        td(107.0, 680.0),
        op("Tj", vec![text(".")]),
        op("ET", vec![]),
        // A scaled text matrix: 6pt font drawn at 2x is 12pt in user space
        op("BT", vec![]),
        font(6.0),
        tm(2.0, 50.0, 660.0),
        op("Tj", vec![text("Texto")]),
        tm(2.0, 83.0, 660.0),
        op("Tj", vec![text("escalado")]),
        op("ET", vec![]),
    ];

    let page_text = load_page_text(ops);
    let lines: Vec<&str> = page_text.lines().collect();
    assert_eq!(
        lines,
        vec!["Hola mundo sin espacios", "Otra linea.", "Texto escalado"],
        "unexpected text: {page_text:?}"
    );
}

#[test]
fn test_stream_order_join_breaks_lines_and_words_by_position() {
    let frag = |x: f32, y: f32, width: f32, text: &str| TextFragment {
        bbox: BoundingBox::new(x, y, width, 10.0),
        text: text.to_string(),
    };
    let fragments = vec![
        frag(50.0, 700.0, 20.0, "pala"),
        frag(70.5, 700.0, 15.0, "bra"),    // 0.5pt gap: same word
        frag(88.0, 701.0, 20.0, "tres"),   // 2.5pt gap on a slightly raised baseline
        frag(50.0, 686.0, 20.0, "cuatro"), // next line
        frag(20.0, 686.0, 10.0, "cinco"),  // jump backwards on the same baseline
    ];

    assert_eq!(
        join_fragments_in_stream_order(&fragments),
        "palabra tres\ncuatro cinco"
    );
}

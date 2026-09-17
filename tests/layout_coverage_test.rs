//! Shapes of PDF that the text layer could read but the extractor was not reading,
//! and one it could not read but claimed it had.

use docugraph::document::layout::extract_page_text_spatial;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document as LopdfDoc, Object, Stream, StringFormat};

/// A content stream placing each string at an absolute position.
fn text_at(lines: &[(&str, f32, f32)]) -> Vec<u8> {
    let mut ops = vec![Operation::new("BT", vec![])];
    for (text, x, y) in lines {
        ops.push(Operation::new(
            "Tf",
            vec![Object::Name(b"F1".to_vec()), Object::Real(12.0)],
        ));
        ops.push(Operation::new(
            "Tm",
            vec![
                Object::Real(1.0),
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(1.0),
                Object::Real(*x),
                Object::Real(*y),
            ],
        ));
        ops.push(Operation::new(
            "Tj",
            vec![Object::String(
                text.as_bytes().to_vec(),
                StringFormat::Literal,
            )],
        ));
    }
    ops.push(Operation::new("ET", vec![]));
    Content { operations: ops }
        .encode()
        .expect("encode content")
}

fn simple_font(doc: &mut LopdfDoc) -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    let font_id = doc.add_object(Object::Dictionary(font));
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    fonts
}

/// A `/Type0` font with no `/ToUnicode`: its bytes cannot be turned into text.
fn undecodable_font(doc: &mut LopdfDoc) -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type0".to_vec()));
    font.set("BaseFont", Object::Name(b"AAAAAA+Custom".to_vec()));
    font.set("Encoding", Object::Name(b"Identity-H".to_vec()));
    let font_id = doc.add_object(Object::Dictionary(font));
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    fonts
}

fn finish_page(doc: &mut LopdfDoc, page: Dictionary) -> (u32, u16) {
    let pages_id = doc.new_object_id();
    let mut page = page;
    page.set("Parent", Object::Reference(pages_id));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
    pages.set("Count", Object::Integer(1));
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", Object::Reference(catalog_id));
    page_id
}

fn base_page() -> Dictionary {
    let mut page = Dictionary::new();
    page.set("Type", Object::Name(b"Page".to_vec()));
    page.set(
        "MediaBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(612.0),
            Object::Real(792.0),
        ]),
    );
    page
}

/// Text drawn through a Form XObject must be read.
///
/// Regression: the `Do` operator did not exist in the extractor - `grep -rn '"Do"'
/// src/` returned nothing - so every glyph inside a form was discarded. A page
/// whose body is one, which FrameMaker, InDesign, Word and PDF/A normalizers all
/// produce, ingested as empty and `document_read_pages` labelled it blank.
#[test]
fn test_text_inside_a_form_xobject_is_read() {
    let mut doc = LopdfDoc::with_version("1.7");
    let fonts = simple_font(&mut doc);

    let mut form_dict = Dictionary::new();
    form_dict.set("Type", Object::Name(b"XObject".to_vec()));
    form_dict.set("Subtype", Object::Name(b"Form".to_vec()));
    form_dict.set(
        "BBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(612.0),
            Object::Real(792.0),
        ]),
    );
    let form_body = text_at(&[
        ("PARAMETRO CRITICO", 72.0, 700.0),
        ("valor de umbral 42", 72.0, 680.0),
    ]);
    let form_id = doc.add_object(Object::Stream(Stream::new(form_dict, form_body)));

    let mut xobjects = Dictionary::new();
    xobjects.set("Fm0", Object::Reference(form_id));
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));
    resources.set("XObject", Object::Dictionary(xobjects));

    let body = Content {
        operations: vec![Operation::new("Do", vec![Object::Name(b"Fm0".to_vec())])],
    };
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        body.encode().expect("encode"),
    )));

    let mut page = base_page();
    page.set("Resources", Object::Dictionary(resources));
    page.set("Contents", Object::Reference(content_id));
    let page_id = finish_page(&mut doc, page);

    let text =
        extract_page_text_spatial(&doc, page_id, false).expect("the form's text is readable");
    assert!(text.contains("PARAMETRO CRITICO"), "got: {text}");
    assert!(text.contains("valor de umbral 42"), "got: {text}");
}

/// A rotated page's visual rows must come out as rows.
///
/// Regression: `/Rotate` was read nowhere in the codebase. On a `/Rotate 90` page
/// the rows a reader sees differ in stored x rather than stored y, so line
/// grouping built columns out of them and a landscape parameter table ingested
/// interleaved.
///
/// The expected order follows from the geometry: `/Rotate 90` displays the page
/// turned clockwise, so a larger stored x appears further down. The row authored
/// at x = 680 is therefore above the one at x = 700.
#[test]
fn test_rotated_page_rows_stay_rows() {
    let mut doc = LopdfDoc::with_version("1.7");
    let fonts = simple_font(&mut doc);
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));

    let body = text_at(&[("FILA UNO", 700.0, 72.0), ("FILA DOS", 680.0, 72.0)]);
    let content_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), body)));

    let mut page = base_page();
    page.set("Rotate", Object::Integer(90));
    page.set("Resources", Object::Dictionary(resources));
    page.set("Contents", Object::Reference(content_id));
    let page_id = finish_page(&mut doc, page);

    let text = extract_page_text_spatial(&doc, page_id, false).expect("the page has text");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        2,
        "the two visual rows must not be merged into one line: {text:?}"
    );
    assert!(lines[0].contains("FILA DOS"), "got: {lines:?}");
    assert!(lines[1].contains("FILA UNO"), "got: {lines:?}");
}

/// Text in a font this code cannot decode must not be emitted as if it were read.
///
/// Regression: a `/Type0` font with no `/ToUnicode` came back from lopdf as a
/// one-byte encoding, which walked that table over two-byte CIDs and dropped the
/// high byte. The result was plausible ASCII - no replacement characters, no
/// control bytes - so the page passed classification as digital text and the
/// corpus, the index and every citation drawn from it carried the mojibake. A
/// search for the words visibly on the page then answered "absent from the corpus".
#[test]
fn test_undecodable_font_text_is_not_invented() {
    let mut doc = LopdfDoc::with_version("1.7");
    let fonts = undecodable_font(&mut doc);
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));

    // What the CIDs of "SECURITY SETTINGS" look like through a one-byte table.
    let body = text_at(&[("SECURITY SETTINGS", 72.0, 700.0)]);
    let content_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), body)));

    let mut page = base_page();
    page.set("Resources", Object::Dictionary(resources));
    page.set("Contents", Object::Reference(content_id));
    let page_id = finish_page(&mut doc, page);

    assert!(
        extract_page_text_spatial(&doc, page_id, false).is_none(),
        "an undecodable font must yield no text, so the page is reported as unreadable \
         rather than as read"
    );
}

/// A font that can be decoded must still be read normally.
#[test]
fn test_decodable_font_is_unaffected() {
    let mut doc = LopdfDoc::with_version("1.7");
    let fonts = simple_font(&mut doc);
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));

    let body = text_at(&[("SECURITY SETTINGS", 72.0, 700.0)]);
    let content_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), body)));

    let mut page = base_page();
    page.set("Resources", Object::Dictionary(resources));
    page.set("Contents", Object::Reference(content_id));
    let page_id = finish_page(&mut doc, page);

    let text = extract_page_text_spatial(&doc, page_id, false).expect("readable font");
    assert!(text.contains("SECURITY SETTINGS"), "got: {text}");
}

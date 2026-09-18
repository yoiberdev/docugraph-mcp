//! Three faults that only appeared once real documents were run through the
//! parser: a public standard whose every section lost its name, a national legal
//! code whose every heading lost its accents, and a government publication that
//! aborted the process.
//!
//! All three are regressions against files anyone can download, named in each test.

use docugraph::document::layout::{BoundingBox, TextFragment, select_reading_order_strategy};
use docugraph::document::model::Page;
use docugraph::document::outline_strategy::{NativeOutlineExtractor, OutlineExtractor};
use docugraph::document::{decode_pdf_string, resolve_to_string};
use lopdf::{Dictionary, Document as LopdfDoc, Object, StringFormat};
use std::collections::HashMap;

/// A text string as a producer writes it when it has no byte-order mark to add:
/// one byte per character, PDFDocEncoding.
///
/// Regression: this arm did not exist, so the bytes went to `from_utf8_lossy`,
/// which cannot read them - `0xF3` is a continuation byte with nothing to
/// continue. Measured on the consolidated Spanish criminal code from the BOE: 946
/// of 953 outline titles were stored with `U+FFFD` where their accents belonged,
/// leaving the document indexed under headings no accented query could match.
#[test]
fn test_pdf_doc_encoding_keeps_accents() {
    // "Disposición final segunda." as PDFDocEncoding.
    let bytes = b"Disposici\xF3n final segunda.";
    assert_eq!(decode_pdf_string(bytes), "Disposición final segunda.");

    // The ligatures and typography that live where Latin-1 differs.
    assert_eq!(decode_pdf_string(b"\x93rma"), "ﬁrma");
    assert_eq!(decode_pdf_string(b"caf\xE9 \x85 t\xE9"), "café – té");
    assert!(
        !decode_pdf_string(b"Art\xEDculo 1\xBA").contains('\u{FFFD}'),
        "no accented byte may decode to the replacement character"
    );
}

/// A producer that writes UTF-8 against the specification must still be read as
/// UTF-8, and a byte-order mark must still win.
#[test]
fn test_other_encodings_are_unaffected() {
    assert_eq!(decode_pdf_string("Sección".as_bytes()), "Sección");

    let mut utf16 = vec![0xFE, 0xFF];
    for u in "Sección".encode_utf16() {
        utf16.extend_from_slice(&u.to_be_bytes());
    }
    assert_eq!(decode_pdf_string(&utf16), "Sección");

    assert_eq!(decode_pdf_string(b"plain ascii"), "plain ascii");
}

/// A dictionary entry typed as a string may be written as a reference to one.
///
/// Regression: the match accepted only `Object::String`, so such an entry read as
/// absent. Measured on the C++ working draft N4950, 2134 pages: all 3075 of its
/// outline entries store `/Title` this way, so every section in the document -
/// and every citation naming one - came back as "Untitled Section".
#[test]
fn test_outline_title_stored_as_a_reference_is_read() {
    let mut doc = LopdfDoc::with_version("1.7");

    let page_id = doc.add_object(Object::Dictionary({
        let mut p = Dictionary::new();
        p.set("Type", Object::Name(b"Page".to_vec()));
        p
    }));

    // The title lives in its own object, as a standards-generating toolchain emits it.
    let title_id = doc.add_object(Object::String(
        b"Overload resolution".to_vec(),
        StringFormat::Literal,
    ));

    let item_id = doc.new_object_id();
    let mut item = Dictionary::new();
    item.set("Title", Object::Reference(title_id));
    item.set("Dest", Object::Array(vec![Object::Reference(page_id)]));
    doc.objects.insert(item_id, Object::Dictionary(item));

    let outlines_id = doc.new_object_id();
    let mut outlines = Dictionary::new();
    outlines.set("Type", Object::Name(b"Outlines".to_vec()));
    outlines.set("First", Object::Reference(item_id));
    outlines.set("Last", Object::Reference(item_id));
    doc.objects
        .insert(outlines_id, Object::Dictionary(outlines));

    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Outlines", Object::Reference(outlines_id));
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", Object::Reference(catalog_id));

    // The resolver itself, and then the extractor that depends on it.
    assert_eq!(
        resolve_to_string(&doc, &Object::Reference(title_id)).as_deref(),
        Some("Overload resolution"),
    );

    let mut page_map = HashMap::new();
    page_map.insert(page_id, 1u32);
    let pages = vec![Page::new(1, "body")];

    let sections = NativeOutlineExtractor.extract(&doc, &page_map, &pages, 1);
    assert_eq!(sections.len(), 1, "one outline entry, one section");
    assert_eq!(
        sections[0].title, "Overload resolution",
        "an indirect title must be followed, not read as missing"
    );
}

/// A fragment whose coordinate runs away must not size an allocation.
///
/// Regression: column detection took its page extent over every fragment and used
/// it to size an occupancy array, so one runaway coordinate sized that array by
/// it. NIST SP 800-53r5, a public 500-page document, carries fragments near
/// 6.2e9 pt: the array came to 3.1e9 bins and the process aborted on a 24.8 GB
/// allocation after twelve minutes of work. Aborting is not a failure a caller
/// can catch - the whole server goes down with it.
#[test]
fn test_runaway_coordinate_does_not_size_an_allocation() {
    let mut fragments: Vec<TextFragment> = (0..40)
        .map(|i| TextFragment {
            bbox: BoundingBox::new(
                72.0 + (i % 2) as f32 * 300.0,
                700.0 - i as f32 * 12.0,
                200.0,
                10.0,
            ),
            text: format!("column body line {i}"),
        })
        .collect();

    // The offending fragment, at the magnitude the NIST document produced.
    fragments.push(TextFragment {
        bbox: BoundingBox::new(6.2e9, 400.0, 12.0, 10.0),
        text: "x".to_string(),
    });

    // Reaching the assertion at all is the test: the unfixed code aborted here.
    let strategy = select_reading_order_strategy(&fragments);
    let text = strategy.reconstruct_text(&fragments);
    assert!(
        text.contains("column body line 0"),
        "the fragments that are on the page must still be read: {text:.120}"
    );
}

/// A page whose fragments are all unusable must degrade, not saturate.
///
/// `f32::INFINITY as usize` saturates to `usize::MAX` rather than wrapping, so the
/// same array would have been asked for at the largest size the type can express.
#[test]
fn test_non_finite_coordinates_degrade_to_single_column() {
    let fragments: Vec<TextFragment> = (0..8)
        .map(|i| TextFragment {
            bbox: BoundingBox::new(f32::NAN, f32::INFINITY, f32::NAN, 10.0),
            text: format!("fragment {i}"),
        })
        .collect();

    let strategy = select_reading_order_strategy(&fragments);
    assert!(
        !strategy.is_multi_column(),
        "a page with no usable geometry cannot have columns detected on it"
    );
}

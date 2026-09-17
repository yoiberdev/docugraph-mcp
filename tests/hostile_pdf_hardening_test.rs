//! A PDF is untrusted input. These cover the shapes that used to abort the
//! process outright, or write outside the directory they were told to write into.

use docugraph::document::outline_strategy::{NativeOutlineExtractor, OutlineExtractor};
use docugraph::document::{
    extract_document_attachments, extract_document_forms, safe_output_name, safe_output_path,
};
use lopdf::{Dictionary, Document as LopdfDoc, Object, StringFormat};
use std::collections::HashMap;

/// A one-page document, its `/Pages` id, and the page id map extractors expect.
type Fixture = (LopdfDoc, (u32, u16), HashMap<(u32, u16), u32>);

fn minimal_doc() -> Fixture {
    let mut doc = LopdfDoc::with_version("1.7");
    let pages_id = doc.new_object_id();

    let mut page = Dictionary::new();
    page.set("Type", Object::Name(b"Page".to_vec()));
    page.set("Parent", Object::Reference(pages_id));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
    pages.set("Count", Object::Integer(1));
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut page_map = HashMap::new();
    page_map.insert(page_id, 1u32);
    (doc, pages_id, page_map)
}

fn attach_catalog(doc: &mut LopdfDoc, pages_id: (u32, u16), entries: Vec<(&str, Object)>) {
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    for (key, value) in entries {
        catalog.set(key, value);
    }
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", Object::Reference(catalog_id));
}

fn titled_item(title: &str) -> Dictionary {
    let mut item = Dictionary::new();
    item.set(
        "Title",
        Object::String(title.as_bytes().to_vec(), StringFormat::Literal),
    );
    item
}

/// An outline item whose `/Next` points back at itself must not be followed forever.
///
/// Regression: the sibling chain was recursed into with no visited set, so a cycle
/// overflowed the stack. That aborts the process rather than returning an error,
/// so `anyhow` and the per-file handling in `docugraph index <dir>` could not
/// contain it: one hostile PDF took the whole batch down, including the files
/// queued behind it.
#[test]
fn test_cyclic_outline_next_chain_terminates() {
    let (mut doc, pages_id, page_map) = minimal_doc();

    let item_id = doc.new_object_id();
    let mut item = titled_item("Loop");
    item.set("Next", Object::Reference(item_id));
    doc.objects.insert(item_id, Object::Dictionary(item));

    let mut outlines = Dictionary::new();
    outlines.set("First", Object::Reference(item_id));
    let outlines_id = doc.add_object(Object::Dictionary(outlines));
    attach_catalog(
        &mut doc,
        pages_id,
        vec![("Outlines", Object::Reference(outlines_id))],
    );

    let sections = NativeOutlineExtractor.extract(&doc, &page_map, &[], 1);
    assert_eq!(
        sections.len(),
        1,
        "the cycle must yield the item exactly once"
    );
}

/// An outline nested deeper than the cap must stop rather than recurse.
#[test]
fn test_deeply_nested_outline_terminates() {
    let (mut doc, pages_id, page_map) = minimal_doc();

    let ids: Vec<(u32, u16)> = (0..500).map(|_| doc.new_object_id()).collect();
    for (i, id) in ids.iter().enumerate() {
        let mut item = titled_item(&format!("Level {i}"));
        if i + 1 < ids.len() {
            item.set("First", Object::Reference(ids[i + 1]));
        }
        doc.objects.insert(*id, Object::Dictionary(item));
    }

    let mut outlines = Dictionary::new();
    outlines.set("First", Object::Reference(ids[0]));
    let outlines_id = doc.add_object(Object::Dictionary(outlines));
    attach_catalog(
        &mut doc,
        pages_id,
        vec![("Outlines", Object::Reference(outlines_id))],
    );

    let sections = NativeOutlineExtractor.extract(&doc, &page_map, &[], 1);
    assert_eq!(sections.len(), 1, "one root, truncated below the depth cap");
}

/// A long flat sibling chain must be walked, not recursed into.
///
/// Regression: recursing once per sibling overflowed the stack at around two
/// thousand, which is well inside the range of a real standards document or legal
/// code.
#[test]
fn test_long_sibling_chain_is_iterated_not_recursed() {
    let (mut doc, pages_id, page_map) = minimal_doc();

    const SIBLINGS: usize = 5000;
    let ids: Vec<(u32, u16)> = (0..SIBLINGS).map(|_| doc.new_object_id()).collect();
    for (i, id) in ids.iter().enumerate() {
        let mut item = titled_item(&format!("Sibling {i}"));
        if i + 1 < ids.len() {
            item.set("Next", Object::Reference(ids[i + 1]));
        }
        doc.objects.insert(*id, Object::Dictionary(item));
    }

    let mut outlines = Dictionary::new();
    outlines.set("First", Object::Reference(ids[0]));
    let outlines_id = doc.add_object(Object::Dictionary(outlines));
    attach_catalog(
        &mut doc,
        pages_id,
        vec![("Outlines", Object::Reference(outlines_id))],
    );

    let sections = NativeOutlineExtractor.extract(&doc, &page_map, &[], 1);
    assert_eq!(sections.len(), SIBLINGS, "every sibling must be visited");
}

/// Siblings share a parent; only the first one used to be told so.
#[test]
fn test_outline_siblings_keep_their_parent() {
    let (mut doc, pages_id, page_map) = minimal_doc();

    let child_a = doc.new_object_id();
    let child_b = doc.new_object_id();

    let mut first = titled_item("Child A");
    first.set("Next", Object::Reference(child_b));
    doc.objects.insert(child_a, Object::Dictionary(first));
    doc.objects
        .insert(child_b, Object::Dictionary(titled_item("Child B")));

    let root_id = doc.new_object_id();
    let mut root = titled_item("Root");
    root.set("First", Object::Reference(child_a));
    doc.objects.insert(root_id, Object::Dictionary(root));

    let mut outlines = Dictionary::new();
    outlines.set("First", Object::Reference(root_id));
    let outlines_id = doc.add_object(Object::Dictionary(outlines));
    attach_catalog(
        &mut doc,
        pages_id,
        vec![("Outlines", Object::Reference(outlines_id))],
    );

    let sections = NativeOutlineExtractor.extract(&doc, &page_map, &[], 1);
    let children = &sections[0].children;
    assert_eq!(children.len(), 2);
    for child in children {
        assert!(
            child.parent_id.is_some(),
            "every sibling must know its parent, not only the first: {} has none",
            child.title
        );
    }
}

/// A name tree whose `/Kids` points back at itself must terminate.
#[test]
fn test_cyclic_embedded_files_name_tree_terminates() {
    let (mut doc, pages_id, page_map) = minimal_doc();

    let node_id = doc.new_object_id();
    let mut node = Dictionary::new();
    node.set("Kids", Object::Array(vec![Object::Reference(node_id)]));
    doc.objects.insert(node_id, Object::Dictionary(node));

    let mut names = Dictionary::new();
    names.set("EmbeddedFiles", Object::Reference(node_id));
    let names_id = doc.add_object(Object::Dictionary(names));
    attach_catalog(
        &mut doc,
        pages_id,
        vec![("Names", Object::Reference(names_id))],
    );

    assert!(extract_document_attachments(&doc, &page_map).is_empty());
}

/// An AcroForm field whose `/Kids` points back at itself must terminate.
#[test]
fn test_cyclic_form_field_tree_terminates() {
    let (mut doc, pages_id, page_map) = minimal_doc();

    let field_id = doc.new_object_id();
    let mut field = Dictionary::new();
    field.set("T", Object::String(b"loop".to_vec(), StringFormat::Literal));
    field.set("FT", Object::Name(b"Tx".to_vec()));
    field.set("Kids", Object::Array(vec![Object::Reference(field_id)]));
    doc.objects.insert(field_id, Object::Dictionary(field));

    let mut acro = Dictionary::new();
    acro.set("Fields", Object::Array(vec![Object::Reference(field_id)]));
    let acro_id = doc.add_object(Object::Dictionary(acro));
    attach_catalog(
        &mut doc,
        pages_id,
        vec![("AcroForm", Object::Reference(acro_id))],
    );

    let forms = extract_document_forms(&doc, &page_map);
    assert!(forms.len() <= 1, "the cycle must not multiply the field");
}

/// An attachment's name cannot choose where its bytes land.
///
/// Regression: `Path::new(dir).join(&att.filename)`, with a name taken straight
/// from the PDF's `/UF`, wrote two directories above `--extract-dir` while the
/// command reported success into that directory. An absolute name ignored the
/// destination entirely.
#[test]
fn test_attachment_names_cannot_escape_the_output_directory() {
    assert_eq!(
        safe_output_name("../../ESCAPED.txt").as_deref(),
        Some("ESCAPED.txt")
    );
    assert_eq!(
        safe_output_name("C:/windows/system32/evil.dll").as_deref(),
        Some("evil.dll")
    );
    assert_eq!(safe_output_name("/etc/passwd").as_deref(), Some("passwd"));
    assert_eq!(safe_output_name(".."), None);
    assert_eq!(safe_output_name("."), None);
    assert_eq!(safe_output_name(""), None);

    let dir = tempfile::tempdir().expect("tempdir");
    let base = std::fs::canonicalize(dir.path()).expect("canonical base");
    for hostile in ["../../ESCAPED.txt", "C:/abs.txt", "/etc/passwd"] {
        let resolved = safe_output_path(dir.path(), hostile).expect("a usable name remains");
        assert!(
            resolved.starts_with(&base),
            "{hostile} resolved to {}, which is outside {}",
            resolved.display(),
            base.display()
        );
    }

    assert!(
        safe_output_path(dir.path(), "..").is_err(),
        "a name with no file component must be refused, not guessed at"
    );
}

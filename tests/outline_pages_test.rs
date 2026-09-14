//! Page resolution of native outlines whose items point to named destinations.
//!
//! The PDFs are synthesised in memory and mimic LaTeX/hyperref output: bookmarks use
//! `/A << /S /GoTo /D (name) >>` and the names live in a multi-level `/Names /Dests` tree.

use docugraph::document::{
    NativeOutlineExtractor, OutlineExtractor, SectionNode, load_pdf_from_path, resolve_dest,
};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document as LopdfDoc, Object, ObjectId, Stream, StringFormat, dictionary};
use std::collections::HashMap;
use tempfile::NamedTempFile;

/// Destination names and their 1-based target pages. Keys look numeric or hexadecimal on purpose.
const DESTS: [(&str, usize); 7] = [
    ("0", 3),
    ("1", 5),
    ("10", 11),
    ("1a", 10),
    ("2", 6),
    ("2c", 12),
    ("3", 8),
];

/// Outline items: (title, destination name, index of the parent item).
const OUTLINE: [(&str, &str, Option<usize>); 8] = [
    ("Front matter", "0", None),
    ("Chapter one", "1", None),
    ("Section one A", "2", Some(1)),
    ("Section one B", "3", Some(1)),
    ("Detail one B1", "3", Some(3)),
    ("Section one C", "1a", Some(1)),
    ("Chapter two", "10", None),
    ("Section two A", "2c", Some(6)),
];

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn catalog_mut(doc: &mut LopdfDoc, catalog_id: ObjectId) -> &mut Dictionary {
    match doc.objects.get_mut(&catalog_id) {
        Some(Object::Dictionary(dict)) => dict,
        _ => panic!("catalog must be a dictionary"),
    }
}

/// Build a PDF whose page `i` (1-based) shows `bodies[i - 1]`; returns the page ids and catalog id.
fn build_pdf(bodies: &[String]) -> (LopdfDoc, Vec<ObjectId>, ObjectId) {
    let mut doc = LopdfDoc::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let mut page_ids = Vec::new();
    for body in bodies {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![text(body)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        page_ids.push(doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }));
    }
    let kids: Vec<Object> = page_ids.iter().copied().map(Object::Reference).collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => page_ids.len() as i64,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    (doc, page_ids, catalog_id)
}

/// Store `DESTS` in a three-level name tree: a root with only /Kids, intermediate nodes and
/// leaves, both with /Limits. Values alternate between the three encodings the spec allows.
fn add_name_tree(doc: &mut LopdfDoc, catalog_id: ObjectId, page_ids: &[ObjectId]) {
    let mut sorted = DESTS.to_vec();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut leaves = Vec::new();
    for (chunk_idx, chunk) in sorted.chunks(2).enumerate() {
        let mut names = Vec::new();
        for (entry_idx, (name, page)) in chunk.iter().enumerate() {
            let dest: Vec<Object> = vec![
                Object::Reference(page_ids[page - 1]),
                Object::Name(b"XYZ".to_vec()),
                54.into(),
                580.into(),
                Object::Null,
            ];
            let value = match (chunk_idx + entry_idx) % 3 {
                0 => Object::Reference(doc.add_object(Object::Array(dest))),
                1 => Object::Reference(doc.add_object(dictionary! { "D" => dest })),
                _ => Object::Array(dest),
            };
            names.push(text(name));
            names.push(value);
        }
        let limits = vec![text(chunk[0].0), text(chunk[chunk.len() - 1].0)];
        let leaf_id = doc.add_object(dictionary! {
            "Limits" => limits.clone(),
            "Names" => names,
        });
        leaves.push((leaf_id, limits));
    }

    let mut intermediate = Vec::new();
    for chunk in leaves.chunks(2) {
        let limits = vec![chunk[0].1[0].clone(), chunk[chunk.len() - 1].1[1].clone()];
        let kids: Vec<Object> = chunk.iter().map(|(id, _)| Object::Reference(*id)).collect();
        intermediate.push(Object::Reference(doc.add_object(dictionary! {
            "Limits" => limits,
            "Kids" => kids,
        })));
    }

    let dests_id = doc.add_object(dictionary! { "Kids" => intermediate });
    let names_id = doc.add_object(dictionary! { "Dests" => dests_id });
    catalog_mut(doc, catalog_id).set("Names", names_id);
}

/// Add `OUTLINE` as native bookmarks using GoTo actions with string destinations.
fn add_outline(doc: &mut LopdfDoc, catalog_id: ObjectId) {
    let ids: Vec<ObjectId> = OUTLINE.iter().map(|_| doc.new_object_id()).collect();
    let outlines_id = doc.new_object_id();
    let children_of = |parent: Option<usize>| -> Vec<usize> {
        (0..OUTLINE.len())
            .filter(|&i| OUTLINE[i].2 == parent)
            .collect()
    };

    for (i, (title, dest, parent)) in OUTLINE.iter().enumerate() {
        let siblings = children_of(*parent);
        let pos = siblings.iter().position(|&s| s == i).unwrap();
        let mut item = dictionary! {
            "Title" => text(title),
            "Parent" => Object::Reference(parent.map_or(outlines_id, |p| ids[p])),
            "A" => dictionary! { "S" => "GoTo", "D" => text(dest) },
        };
        if let Some(&next) = siblings.get(pos + 1) {
            item.set("Next", Object::Reference(ids[next]));
        }
        if pos > 0 {
            item.set("Prev", Object::Reference(ids[siblings[pos - 1]]));
        }
        let kids = children_of(Some(i));
        if let (Some(&first), Some(&last)) = (kids.first(), kids.last()) {
            item.set("First", Object::Reference(ids[first]));
            item.set("Last", Object::Reference(ids[last]));
            item.set("Count", kids.len() as i64);
        }
        doc.objects.insert(ids[i], Object::Dictionary(item));
    }

    let roots = children_of(None);
    doc.objects.insert(
        outlines_id,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => Object::Reference(ids[roots[0]]),
            "Last" => Object::Reference(ids[roots[roots.len() - 1]]),
            "Count" => roots.len() as i64,
        }),
    );
    catalog_mut(doc, catalog_id).set("Outlines", outlines_id);
}

/// 12-page PDF with the hyperref-style name tree and outline. Page bodies never repeat a title,
/// so the title-search fallback cannot hide a wrong destination.
fn outline_fixture() -> (LopdfDoc, Vec<ObjectId>, ObjectId) {
    let bodies: Vec<String> = (1..=12)
        .map(|n| format!("Body paragraph printed on sheet number {n}"))
        .collect();
    let (mut doc, page_ids, catalog_id) = build_pdf(&bodies);
    add_name_tree(&mut doc, catalog_id, &page_ids);
    add_outline(&mut doc, catalog_id);
    (doc, page_ids, catalog_id)
}

fn page_map(doc: &LopdfDoc) -> HashMap<ObjectId, u32> {
    doc.get_pages()
        .into_iter()
        .map(|(number, id)| (id, number))
        .collect()
}

fn flatten(sections: &[SectionNode]) -> Vec<&SectionNode> {
    sections.iter().flat_map(|s| s.flatten()).collect()
}

#[test]
fn test_outline_pages_resolve_through_nested_name_tree() {
    let (mut doc, _, _) = outline_fixture();
    let mut file = NamedTempFile::new().unwrap();
    doc.save_to(&mut file).unwrap();

    let parsed = load_pdf_from_path(file.path()).unwrap();
    let pages: Vec<(&str, u32)> = flatten(&parsed.sections)
        .iter()
        .map(|s| (s.title.as_str(), s.page_start))
        .collect();

    assert_eq!(
        pages,
        vec![
            ("Front matter", 3),
            ("Chapter one", 5),
            ("Section one A", 6),
            ("Section one B", 8),
            ("Detail one B1", 8),
            ("Section one C", 10),
            ("Chapter two", 11),
            ("Section two A", 12),
        ]
    );
}

#[test]
fn test_named_destination_is_never_read_as_page_index() {
    let (mut doc, page_ids, catalog_id) = outline_fixture();

    // PDF 1.1 catalog /Dests dictionary whose value is a destination dictionary
    let legacy = dictionary! {
        "legacy" => dictionary! {
            "D" => vec![Object::Reference(page_ids[1]), Object::Name(b"Fit".to_vec())],
        },
    };
    catalog_mut(&mut doc, catalog_id).set("Dests", legacy);
    let map = page_map(&doc);

    assert_eq!(resolve_dest(&doc, &text("1a"), &map), Some(10));
    assert_eq!(resolve_dest(&doc, &text("3"), &map), Some(8));
    assert_eq!(
        resolve_dest(&doc, &Object::Name(b"2c".to_vec()), &map),
        Some(12)
    );
    assert_eq!(
        resolve_dest(&doc, &Object::Name(b"legacy".to_vec()), &map),
        Some(2)
    );
    // "7" is not a destination: it must not be guessed as page 8 (index 7 + 1)
    assert_eq!(resolve_dest(&doc, &text("7"), &map), None);
}

/// Assert that every node's parent_id names the node containing it; returns how many were checked.
fn assert_parent_links(nodes: &[SectionNode], parent: Option<&str>) -> usize {
    let mut checked = 0;
    for node in nodes {
        assert_eq!(
            node.parent_id.as_deref(),
            parent,
            "wrong parent_id for '{}'",
            node.title
        );
        checked += 1 + assert_parent_links(&node.children, Some(&node.id));
    }
    checked
}

#[test]
fn test_native_outline_siblings_keep_parent_id() {
    let (doc, _, _) = outline_fixture();
    let sections = NativeOutlineExtractor.extract(&doc, &page_map(&doc), &[], 12);

    let chapter_one = &sections[1];
    let titles: Vec<&str> = chapter_one
        .children
        .iter()
        .map(|child| child.title.as_str())
        .collect();
    assert_eq!(
        titles,
        vec!["Section one A", "Section one B", "Section one C"]
    );
    assert_eq!(assert_parent_links(&sections, None), OUTLINE.len());
}

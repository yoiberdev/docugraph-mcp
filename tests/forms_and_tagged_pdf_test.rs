//! Integration tests for Milestone 8: Interactive AcroForms & Tagged PDF Semantic Structure.

use docugraph::document::{
    Document, DocumentId, DocumentMetadata, FormField, FormFieldType, Page,
    detect_tagged_pdf_structure, extract_document_forms,
};
use docugraph::mcp::{
    DocuGraphServer, DocumentGetFormsParams, DocumentGetFormsResult, DocumentInfoParams,
    DocumentInfoResult,
};
use docugraph::storage::DocumentStore;
use lopdf::{Dictionary, Document as LopdfDoc, Object};
use rmcp::handler::server::wrapper::Parameters;
use std::collections::HashMap;

type TestDocFixture = (LopdfDoc, (u32, u16), (u32, u16), HashMap<(u32, u16), u32>);

/// Helper to create a minimal lopdf::Document with 2 pages for testing.
fn create_test_lopdf_doc() -> TestDocFixture {
    let mut doc = LopdfDoc::with_version("1.7");

    let pages_id = doc.new_object_id();
    let page1_id = doc.new_object_id();
    let page2_id = doc.new_object_id();

    let mut page1_dict = Dictionary::new();
    page1_dict.set("Type", Object::Name(b"Page".to_vec()));
    page1_dict.set("Parent", Object::Reference(pages_id));

    let mut page2_dict = Dictionary::new();
    page2_dict.set("Type", Object::Name(b"Page".to_vec()));
    page2_dict.set("Parent", Object::Reference(pages_id));

    doc.objects.insert(page1_id, Object::Dictionary(page1_dict));
    doc.objects.insert(page2_id, Object::Dictionary(page2_dict));

    let mut pages_dict = Dictionary::new();
    pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
    pages_dict.set("Count", Object::Integer(2));
    pages_dict.set(
        "Kids",
        Object::Array(vec![
            Object::Reference(page1_id),
            Object::Reference(page2_id),
        ]),
    );
    doc.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = doc.add_object(Object::Dictionary(catalog));

    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut page_map = HashMap::new();
    page_map.insert(page1_id, 1);
    page_map.insert(page2_id, 2);

    (doc, page1_id, page2_id, page_map)
}

#[test]
fn test_extract_text_form_field() {
    let (mut doc, page1_id, _, page_map) = create_test_lopdf_doc();

    let mut field_dict = Dictionary::new();
    field_dict.set("FT", Object::Name(b"Tx".to_vec()));
    field_dict.set(
        "T",
        Object::String(b"username".to_vec(), lopdf::StringFormat::Literal),
    );
    field_dict.set(
        "V",
        Object::String(b"alovelace".to_vec(), lopdf::StringFormat::Literal),
    );
    field_dict.set(
        "DV",
        Object::String(b"default_user".to_vec(), lopdf::StringFormat::Literal),
    );
    field_dict.set("P", Object::Reference(page1_id));
    field_dict.set(
        "Rect",
        Object::Array(vec![
            Object::Real(100.0),
            Object::Real(200.0),
            Object::Real(300.0),
            Object::Real(230.0),
        ]),
    );

    let field_id = doc.add_object(Object::Dictionary(field_dict));

    let mut acro_dict = Dictionary::new();
    acro_dict.set("Fields", Object::Array(vec![Object::Reference(field_id)]));
    let acro_id = doc.add_object(Object::Dictionary(acro_dict));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("AcroForm", Object::Reference(acro_id));

    let forms = extract_document_forms(&doc, &page_map);
    assert_eq!(forms.len(), 1, "Should extract 1 form field");

    let field = &forms[0];
    assert_eq!(field.name, "username");
    assert_eq!(field.fully_qualified_name, "username");
    assert_eq!(field.field_type, FormFieldType::Text);
    assert_eq!(field.value.as_deref(), Some("alovelace"));
    assert_eq!(field.default_value.as_deref(), Some("default_user"));
    assert_eq!(field.page_number, Some(1));
    assert_eq!(field.rect, Some([100.0, 200.0, 300.0, 230.0]));
    assert!(!field.read_only);
    assert!(!field.required);
}

#[test]
fn test_extract_checkbox_and_radio_form_fields() {
    let (mut doc, page1_id, page2_id, page_map) = create_test_lopdf_doc();

    // 1. Checkbox field: /FT /Btn without radio flag (bit 16)
    let mut check_dict = Dictionary::new();
    check_dict.set("FT", Object::Name(b"Btn".to_vec()));
    check_dict.set(
        "T",
        Object::String(b"terms_accepted".to_vec(), lopdf::StringFormat::Literal),
    );
    check_dict.set("V", Object::Name(b"Yes".to_vec()));
    check_dict.set("P", Object::Reference(page1_id));
    let check_id = doc.add_object(Object::Dictionary(check_dict));

    // 2. Radio button field: /FT /Btn with bit 16 (1 << 15 = 32768)
    let mut radio_dict = Dictionary::new();
    radio_dict.set("FT", Object::Name(b"Btn".to_vec()));
    radio_dict.set("Ff", Object::Integer(1 << 15));
    radio_dict.set(
        "T",
        Object::String(b"payment_method".to_vec(), lopdf::StringFormat::Literal),
    );
    radio_dict.set("V", Object::Name(b"CreditCard".to_vec()));
    radio_dict.set("P", Object::Reference(page2_id));
    let radio_id = doc.add_object(Object::Dictionary(radio_dict));

    let mut acro_dict = Dictionary::new();
    acro_dict.set(
        "Fields",
        Object::Array(vec![
            Object::Reference(check_id),
            Object::Reference(radio_id),
        ]),
    );
    let acro_id = doc.add_object(Object::Dictionary(acro_dict));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("AcroForm", Object::Reference(acro_id));

    let forms = extract_document_forms(&doc, &page_map);
    assert_eq!(forms.len(), 2, "Should extract 2 form fields");

    let check_field = forms.iter().find(|f| f.name == "terms_accepted").unwrap();
    assert_eq!(check_field.field_type, FormFieldType::Checkbox);
    assert_eq!(check_field.value.as_deref(), Some("Yes"));
    assert_eq!(check_field.page_number, Some(1));

    let radio_field = forms.iter().find(|f| f.name == "payment_method").unwrap();
    assert_eq!(radio_field.field_type, FormFieldType::Radio);
    assert_eq!(radio_field.value.as_deref(), Some("CreditCard"));
    assert_eq!(radio_field.page_number, Some(2));
}

#[test]
fn test_field_flags_readonly_and_required() {
    let (mut doc, page1_id, _, page_map) = create_test_lopdf_doc();

    // Read-only field (bit 1 = 1)
    let mut f1 = Dictionary::new();
    f1.set("FT", Object::Name(b"Tx".to_vec()));
    f1.set(
        "T",
        Object::String(b"invoice_id".to_vec(), lopdf::StringFormat::Literal),
    );
    f1.set("Ff", Object::Integer(1)); // ReadOnly
    f1.set("P", Object::Reference(page1_id));
    let f1_id = doc.add_object(Object::Dictionary(f1));

    // Required field (bit 2 = 2)
    let mut f2 = Dictionary::new();
    f2.set("FT", Object::Name(b"Tx".to_vec()));
    f2.set(
        "T",
        Object::String(b"signature_line".to_vec(), lopdf::StringFormat::Literal),
    );
    f2.set("Ff", Object::Integer(2)); // Required
    f2.set("P", Object::Reference(page1_id));
    let f2_id = doc.add_object(Object::Dictionary(f2));

    let mut acro_dict = Dictionary::new();
    acro_dict.set(
        "Fields",
        Object::Array(vec![Object::Reference(f1_id), Object::Reference(f2_id)]),
    );
    let acro_id = doc.add_object(Object::Dictionary(acro_dict));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("AcroForm", Object::Reference(acro_id));

    let forms = extract_document_forms(&doc, &page_map);
    assert_eq!(forms.len(), 2);

    let f1_res = forms.iter().find(|f| f.name == "invoice_id").unwrap();
    assert!(f1_res.read_only);
    assert!(!f1_res.required);

    let f2_res = forms.iter().find(|f| f.name == "signature_line").unwrap();
    assert!(!f2_res.read_only);
    assert!(f2_res.required);
}

#[test]
fn test_hierarchical_form_field_names_and_inherited_types() {
    let (mut doc, page1_id, _, page_map) = create_test_lopdf_doc();

    // Leaf field: name="zip", value="94107"
    let mut leaf = Dictionary::new();
    leaf.set(
        "T",
        Object::String(b"zip".to_vec(), lopdf::StringFormat::Literal),
    );
    leaf.set(
        "V",
        Object::String(b"94107".to_vec(), lopdf::StringFormat::Literal),
    );
    leaf.set("P", Object::Reference(page1_id));
    let leaf_id = doc.add_object(Object::Dictionary(leaf));

    // Intermediate parent field: name="address"
    let mut mid = Dictionary::new();
    mid.set(
        "T",
        Object::String(b"address".to_vec(), lopdf::StringFormat::Literal),
    );
    mid.set("Kids", Object::Array(vec![Object::Reference(leaf_id)]));
    let mid_id = doc.add_object(Object::Dictionary(mid));

    // Top root field: name="employee", specifies /FT /Tx (inherited by children)
    let mut top = Dictionary::new();
    top.set(
        "T",
        Object::String(b"employee".to_vec(), lopdf::StringFormat::Literal),
    );
    top.set("FT", Object::Name(b"Tx".to_vec()));
    top.set("Kids", Object::Array(vec![Object::Reference(mid_id)]));
    let top_id = doc.add_object(Object::Dictionary(top));

    let mut acro_dict = Dictionary::new();
    acro_dict.set("Fields", Object::Array(vec![Object::Reference(top_id)]));
    let acro_id = doc.add_object(Object::Dictionary(acro_dict));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("AcroForm", Object::Reference(acro_id));

    let forms = extract_document_forms(&doc, &page_map);
    assert_eq!(forms.len(), 1, "Only leaf node should be yielded");

    let field = &forms[0];
    assert_eq!(field.name, "zip");
    assert_eq!(field.fully_qualified_name, "employee.address.zip");
    assert_eq!(
        field.field_type,
        FormFieldType::Text,
        "Field type must be inherited from top"
    );
    assert_eq!(field.value.as_deref(), Some("94107"));
    assert_eq!(field.page_number, Some(1));
}

#[test]
fn test_page_resolution_via_page_annots() {
    let (mut doc, _, page2_id, page_map) = create_test_lopdf_doc();

    // Field dictionary without /P
    let mut field = Dictionary::new();
    field.set("FT", Object::Name(b"Tx".to_vec()));
    field.set(
        "T",
        Object::String(b"orphan_p".to_vec(), lopdf::StringFormat::Literal),
    );
    field.set(
        "V",
        Object::String(b"resolved_via_annot".to_vec(), lopdf::StringFormat::Literal),
    );
    let field_id = doc.add_object(Object::Dictionary(field));

    // Link field in page 2's /Annots
    if let Some(Object::Dictionary(p2_dict)) = doc.objects.get_mut(&page2_id) {
        p2_dict.set("Annots", Object::Array(vec![Object::Reference(field_id)]));
    }

    let mut acro_dict = Dictionary::new();
    acro_dict.set("Fields", Object::Array(vec![Object::Reference(field_id)]));
    let acro_id = doc.add_object(Object::Dictionary(acro_dict));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("AcroForm", Object::Reference(acro_id));

    let forms = extract_document_forms(&doc, &page_map);
    assert_eq!(forms.len(), 1);
    assert_eq!(
        forms[0].page_number,
        Some(2),
        "Must resolve page 2 from page /Annots"
    );
}

#[test]
fn test_detect_tagged_pdf_structure() {
    let (mut doc, _, _, _) = create_test_lopdf_doc();

    // Baseline: untagged document
    let info = detect_tagged_pdf_structure(&doc);
    assert!(!info.is_tagged);
    assert!(!info.struct_tree_root_present);
    assert!(!info.mark_info_marked);

    // 1. Add StructTreeRoot
    let struct_tree_id = doc.new_object_id();
    doc.objects
        .insert(struct_tree_id, Object::Dictionary(Dictionary::new()));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    if let Some(Object::Dictionary(root_dict)) = doc.objects.get_mut(&root_ref) {
        root_dict.set("StructTreeRoot", Object::Reference(struct_tree_id));
    }

    let info_tagged = detect_tagged_pdf_structure(&doc);
    assert!(info_tagged.is_tagged);
    assert!(info_tagged.struct_tree_root_present);
    assert!(!info_tagged.mark_info_marked);

    // 2. Add MarkInfo << /Marked true >>
    let mut mark_dict = Dictionary::new();
    mark_dict.set("Marked", Object::Boolean(true));
    if let Some(Object::Dictionary(root_dict)) = doc.objects.get_mut(&root_ref) {
        root_dict.set("MarkInfo", Object::Dictionary(mark_dict));
    }

    let info_fully_tagged = detect_tagged_pdf_structure(&doc);
    assert!(info_fully_tagged.is_tagged);
    assert!(info_fully_tagged.struct_tree_root_present);
    assert!(info_fully_tagged.mark_info_marked);
}

#[tokio::test]
async fn test_mcp_document_get_forms_and_filtering() {
    let store = DocumentStore::new(None);

    let doc = Document {
        id: DocumentId::from("doc_with_forms"),
        metadata: DocumentMetadata {
            id: "doc_with_forms".to_string(),
            title: "Tax Form 1040".to_string(),
            author: Some("IRS".to_string()),
            total_pages: 2,
            total_sections: 1,
            file_size_bytes: 4096,
            content_hash: "abcd1234hash".to_string(),
            indexed_at: "2026-09-14T00:00:00Z".to_string(),
            is_encrypted: false,
            untrusted_text_detected: false,
            scanned_pages_count: 0,
            source_path: None,
            total_links: 0,
            has_forms: true,
            total_form_fields: 3,
            is_tagged: true,
            ..Default::default()
        },
        pages: vec![Page::new(1, "Page 1"), Page::new(2, "Page 2")],
        sections: vec![],
        forms: vec![
            FormField {
                name: "first_name".to_string(),
                fully_qualified_name: "taxpayer.first_name".to_string(),
                field_type: FormFieldType::Text,
                value: Some("Grace".to_string()),
                default_value: None,
                read_only: false,
                required: true,
                page_number: Some(1),
                rect: Some([10.0, 20.0, 100.0, 40.0]),
            },
            FormField {
                name: "middle_initial".to_string(),
                fully_qualified_name: "taxpayer.middle_initial".to_string(),
                field_type: FormFieldType::Text,
                value: None, // Empty value
                default_value: None,
                read_only: false,
                required: false,
                page_number: Some(1),
                rect: Some([110.0, 20.0, 130.0, 40.0]),
            },
            FormField {
                name: "w2_attached".to_string(),
                fully_qualified_name: "attachments.w2_attached".to_string(),
                field_type: FormFieldType::Checkbox,
                value: Some("Yes".to_string()),
                default_value: None,
                read_only: true,
                required: false,
                page_number: Some(2),
                rect: Some([50.0, 80.0, 65.0, 95.0]),
            },
        ],
        attachments: vec![],
    };

    store.insert(doc).expect("insert should succeed");
    let server = DocuGraphServer::with_store(store);

    // 1. Get all forms (no filters)
    let json_str = server
        .document_get_forms(Parameters(DocumentGetFormsParams {
            document_id: "doc_with_forms".to_string(),
            page: None,
            filled_only: None,
        }))
        .await;

    let res: DocumentGetFormsResult = serde_json::from_str(&json_str).expect("parse JSON");
    assert_eq!(res.total_fields, 3);
    assert_eq!(res.fields.len(), 3);

    // 2. Filter by page 1
    let json_p1 = server
        .document_get_forms(Parameters(DocumentGetFormsParams {
            document_id: "doc_with_forms".to_string(),
            page: Some(1),
            filled_only: None,
        }))
        .await;

    let res_p1: DocumentGetFormsResult = serde_json::from_str(&json_p1).expect("parse JSON");
    assert_eq!(res_p1.total_fields, 2);
    assert!(res_p1.fields.iter().all(|f| f.page_number == Some(1)));

    // 3. Filter by filled_only: true
    let json_filled = server
        .document_get_forms(Parameters(DocumentGetFormsParams {
            document_id: "doc_with_forms".to_string(),
            page: None,
            filled_only: Some(true),
        }))
        .await;

    let res_filled: DocumentGetFormsResult =
        serde_json::from_str(&json_filled).expect("parse JSON");
    assert_eq!(res_filled.total_fields, 2);
    assert!(res_filled.fields.iter().all(|f| f.value.is_some()));
    assert!(!res_filled.fields.iter().any(|f| f.name == "middle_initial"));

    // 4. Test document_info includes forms & tagged metadata
    let info_str = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "doc_with_forms".to_string(),
        }))
        .await;

    let info_res: DocumentInfoResult = serde_json::from_str(&info_str).expect("parse info JSON");
    assert_eq!(info_res.title, "Tax Form 1040");
    assert!(info_res.has_forms);
    assert_eq!(info_res.total_form_fields, 3);
    assert!(info_res.is_tagged);
}

//! Integration tests for Milestone 9: Embedded Files & Attachments (/EmbeddedFiles, /AF, /FileAttachment).

use docugraph::document::{
    Document, DocumentId, DocumentMetadata, EmbeddedAttachment, Page, extract_document_attachments,
};
use docugraph::mcp::{
    DocuGraphServer, DocumentGetAttachmentsParams, DocumentGetAttachmentsResult,
    DocumentInfoParams, DocumentInfoResult, DocumentReadAttachmentParams,
    DocumentReadAttachmentResult,
};
use docugraph::storage::DocumentStore;
use lopdf::{Dictionary, Document as LopdfDoc, Object, Stream};
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
fn test_extract_attachment_from_embedded_files_name_tree() {
    let (mut doc, _, _, page_map) = create_test_lopdf_doc();

    let xml_data = b"<?xml version=\"1.0\"?><rsm:CrossIndustryInvoice><rsm:ID>INV-2026-001</rsm:ID></rsm:CrossIndustryInvoice>".to_vec();

    // 1. Create embedded file stream
    let mut stream_dict = Dictionary::new();
    stream_dict.set("Type", Object::Name(b"EmbeddedFile".to_vec()));
    stream_dict.set("Subtype", Object::Name(b"text#2Fxml".to_vec()));
    let mut params_dict = Dictionary::new();
    params_dict.set("Size", Object::Integer(xml_data.len() as i64));
    params_dict.set(
        "ModDate",
        Object::String(b"D:20260914000000Z".to_vec(), lopdf::StringFormat::Literal),
    );
    stream_dict.set("Params", Object::Dictionary(params_dict));

    let stream = Stream::new(stream_dict, xml_data.clone());
    let stream_id = doc.add_object(Object::Stream(stream));

    // 2. Create Filespec dictionary
    let mut ef_dict = Dictionary::new();
    ef_dict.set("UF", Object::Reference(stream_id));

    let mut filespec = Dictionary::new();
    filespec.set("Type", Object::Name(b"Filespec".to_vec()));
    filespec.set(
        "UF",
        Object::String(b"factur-x.xml".to_vec(), lopdf::StringFormat::Literal),
    );
    filespec.set(
        "Desc",
        Object::String(
            b"Factur-X / ZUGFeRD electronic invoice data".to_vec(),
            lopdf::StringFormat::Literal,
        ),
    );
    filespec.set("EF", Object::Dictionary(ef_dict));
    let filespec_id = doc.add_object(Object::Dictionary(filespec));

    // 3. Create Name Tree: /Root /Names /EmbeddedFiles
    let mut ef_name_tree = Dictionary::new();
    ef_name_tree.set(
        "Names",
        Object::Array(vec![
            Object::String(b"factur-x.xml".to_vec(), lopdf::StringFormat::Literal),
            Object::Reference(filespec_id),
        ]),
    );
    let ef_tree_id = doc.add_object(Object::Dictionary(ef_name_tree));

    let mut names_dict = Dictionary::new();
    names_dict.set("EmbeddedFiles", Object::Reference(ef_tree_id));
    let names_id = doc.add_object(Object::Dictionary(names_dict));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("Names", Object::Reference(names_id));

    let attachments = extract_document_attachments(&doc, &page_map);
    assert_eq!(attachments.len(), 1, "Should extract 1 embedded file");

    let att = &attachments[0];
    assert_eq!(att.filename, "factur-x.xml");
    assert_eq!(
        att.description.as_deref(),
        Some("Factur-X / ZUGFeRD electronic invoice data")
    );
    assert_eq!(att.mime_type.as_deref(), Some("text/xml"));
    assert_eq!(att.size_bytes, xml_data.len() as u64);
    assert_eq!(att.mod_date.as_deref(), Some("D:20260914000000Z"));
    assert!(att.is_text);
    assert_eq!(att.data, xml_data);
    assert!(att.text_content().unwrap().contains("INV-2026-001"));
}

#[test]
fn test_extract_attachment_from_associated_files_af() {
    let (mut doc, _, _, page_map) = create_test_lopdf_doc();

    let csv_data = b"id,sku,price\n1,WIDGET-A,19.99\n2,WIDGET-B,29.99\n".to_vec();

    let mut stream_dict = Dictionary::new();
    stream_dict.set("Type", Object::Name(b"EmbeddedFile".to_vec()));
    stream_dict.set("Subtype", Object::Name(b"text#2Fcsv".to_vec()));
    let stream = Stream::new(stream_dict, csv_data.clone());
    let stream_id = doc.add_object(Object::Stream(stream));

    let mut ef_dict = Dictionary::new();
    ef_dict.set("F", Object::Reference(stream_id));

    let mut filespec = Dictionary::new();
    filespec.set("Type", Object::Name(b"Filespec".to_vec()));
    filespec.set(
        "F",
        Object::String(b"items_catalog.csv".to_vec(), lopdf::StringFormat::Literal),
    );
    filespec.set("EF", Object::Dictionary(ef_dict));
    let filespec_id = doc.add_object(Object::Dictionary(filespec));

    // Attach to /Root /AF (Associated Files in PDF/A-3)
    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("AF", Object::Array(vec![Object::Reference(filespec_id)]));

    let attachments = extract_document_attachments(&doc, &page_map);
    assert_eq!(attachments.len(), 1);

    let att = &attachments[0];
    assert_eq!(att.filename, "items_catalog.csv");
    assert_eq!(att.mime_type.as_deref(), Some("text/csv"));
    assert!(att.is_text);
    assert_eq!(att.data, csv_data);
}

#[test]
fn test_extract_attachment_from_page_annotation() {
    let (mut doc, _, page2_id, page_map) = create_test_lopdf_doc();

    let notes_data = b"Meeting notes: Agreed on architecture specifications.".to_vec();

    let mut stream_dict = Dictionary::new();
    stream_dict.set("Type", Object::Name(b"EmbeddedFile".to_vec()));
    let stream = Stream::new(stream_dict, notes_data.clone());
    let stream_id = doc.add_object(Object::Stream(stream));

    let mut ef_dict = Dictionary::new();
    ef_dict.set("UF", Object::Reference(stream_id));

    let mut filespec = Dictionary::new();
    filespec.set(
        "UF",
        Object::String(b"minutes.txt".to_vec(), lopdf::StringFormat::Literal),
    );
    filespec.set("EF", Object::Dictionary(ef_dict));
    let filespec_id = doc.add_object(Object::Dictionary(filespec));

    // Page 2 FileAttachment annotation
    let mut annot = Dictionary::new();
    annot.set("Type", Object::Name(b"Annot".to_vec()));
    annot.set("Subtype", Object::Name(b"FileAttachment".to_vec()));
    annot.set("FS", Object::Reference(filespec_id));
    let annot_id = doc.add_object(Object::Dictionary(annot));

    let p2_dict = doc
        .objects
        .get_mut(&page2_id)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    p2_dict.set("Annots", Object::Array(vec![Object::Reference(annot_id)]));

    let attachments = extract_document_attachments(&doc, &page_map);
    assert_eq!(attachments.len(), 1);

    let att = &attachments[0];
    assert_eq!(att.filename, "minutes.txt");
    assert_eq!(att.page_number, Some(2), "Must resolve to page 2");
    assert!(att.is_text);
    assert_eq!(att.data, notes_data);
}

#[test]
fn test_extract_binary_attachment_and_checksum() {
    let (mut doc, _, _, page_map) = create_test_lopdf_doc();

    // Binary payload with simulated MD5 checksum
    let binary_data = vec![0x1f, 0x8b, 0x08, 0x00, 0xde, 0xad, 0xbe, 0xef, 0x00, 0x03];
    let md5_bytes = b"0123456789abcdef".to_vec(); // 16 bytes

    let mut stream_dict = Dictionary::new();
    stream_dict.set("Type", Object::Name(b"EmbeddedFile".to_vec()));
    stream_dict.set("Subtype", Object::Name(b"application#2Fgzip".to_vec()));
    let mut params_dict = Dictionary::new();
    params_dict.set("Size", Object::Integer(binary_data.len() as i64));
    params_dict.set(
        "CheckSum",
        Object::String(md5_bytes.clone(), lopdf::StringFormat::Literal),
    );
    stream_dict.set("Params", Object::Dictionary(params_dict));

    let stream = Stream::new(stream_dict, binary_data.clone());
    let stream_id = doc.add_object(Object::Stream(stream));

    let mut ef_dict = Dictionary::new();
    ef_dict.set("UF", Object::Reference(stream_id));

    let mut filespec = Dictionary::new();
    filespec.set(
        "UF",
        Object::String(b"firmware.bin.gz".to_vec(), lopdf::StringFormat::Literal),
    );
    filespec.set("EF", Object::Dictionary(ef_dict));
    let filespec_id = doc.add_object(Object::Dictionary(filespec));

    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let root_dict = doc
        .objects
        .get_mut(&root_ref)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    root_dict.set("AF", Object::Array(vec![Object::Reference(filespec_id)]));

    let attachments = extract_document_attachments(&doc, &page_map);
    assert_eq!(attachments.len(), 1);

    let att = &attachments[0];
    assert_eq!(att.filename, "firmware.bin.gz");
    assert_eq!(att.mime_type.as_deref(), Some("application/gzip"));
    assert!(
        !att.is_text,
        "Binary gzip data should not be flagged as valid text"
    );
    assert_eq!(att.text_content(), None);
    assert_eq!(att.checksum_md5, Some(hex::encode(md5_bytes)));
}

#[tokio::test]
async fn test_mcp_document_get_attachments_and_read_attachment() {
    let store = DocumentStore::new(None);

    let invoice_xml = b"<invoice><number>1042</number><amount>150.00</amount></invoice>".to_vec();
    let binary_blob = vec![0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE];

    let doc = Document {
        id: DocumentId::from("doc_with_attachments"),
        metadata: DocumentMetadata {
            id: "doc_with_attachments".to_string(),
            title: "Corporate Invoice with Attachments".to_string(),
            author: Some("Finance Corp".to_string()),
            total_pages: 1,
            total_sections: 1,
            file_size_bytes: 4096,
            content_hash: "att_doc_hash_123".to_string(),
            indexed_at: "2026-09-14T00:00:00Z".to_string(),
            is_encrypted: false,
            untrusted_text_detected: false,
            scanned_pages_count: 0,
            source_path: None,
            total_links: 0,
            has_forms: false,
            total_form_fields: 0,
            is_tagged: false,
            has_attachments: true,
            total_attachments: 2,
        },
        pages: vec![Page::new(1, "Invoice cover page")],
        sections: vec![],
        forms: vec![],
        attachments: vec![
            EmbeddedAttachment {
                id: "att_factur-x.xml".to_string(),
                filename: "factur-x.xml".to_string(),
                description: Some("Structured XML Invoice".to_string()),
                mime_type: Some("text/xml".to_string()),
                size_bytes: invoice_xml.len() as u64,
                checksum_md5: None,
                mod_date: Some("2026-09-14".to_string()),
                is_text: true,
                page_number: None,
                data: invoice_xml.clone(),
            },
            EmbeddedAttachment {
                id: "att_signature.p7s".to_string(),
                filename: "signature.p7s".to_string(),
                description: Some("Cryptographic signature".to_string()),
                mime_type: Some("application/pkcs7-signature".to_string()),
                size_bytes: binary_blob.len() as u64,
                checksum_md5: None,
                mod_date: None,
                is_text: false,
                page_number: Some(1),
                data: binary_blob.clone(),
            },
        ],
    };

    store.insert(doc).expect("insert should succeed");
    let server = DocuGraphServer::with_store(store);

    // 1. Call document_get_attachments
    let list_json = server
        .document_get_attachments(Parameters(DocumentGetAttachmentsParams {
            document_id: "doc_with_attachments".to_string(),
        }))
        .await
        .expect("attachments must succeed for an indexed document");

    let list_res: DocumentGetAttachmentsResult =
        serde_json::from_str(&list_json).expect("parse list JSON");
    assert_eq!(list_res.total_attachments, 2);
    assert_eq!(list_res.attachments.len(), 2);
    assert_eq!(list_res.attachments[0].filename, "factur-x.xml");
    assert_eq!(list_res.attachments[1].filename, "signature.p7s");

    // 2. Call document_read_attachment on XML text
    let read_xml_json = server
        .document_read_attachment(Parameters(DocumentReadAttachmentParams {
            document_id: "doc_with_attachments".to_string(),
            name_or_id: "factur-x.xml".to_string(),
            max_bytes: None,
            encoding: None,
        }))
        .await
        .expect("read XML attachment succeeds");

    let read_xml_res: DocumentReadAttachmentResult =
        serde_json::from_str(&read_xml_json).expect("parse read XML JSON");
    assert_eq!(read_xml_res.filename, "factur-x.xml");
    assert_eq!(read_xml_res.encoding, "text");
    assert!(!read_xml_res.truncated);
    assert!(read_xml_res.content.contains("<number>1042</number>"));

    // 3. Call document_read_attachment on binary payload
    let read_bin_json = server
        .document_read_attachment(Parameters(DocumentReadAttachmentParams {
            document_id: "doc_with_attachments".to_string(),
            name_or_id: "signature.p7s".to_string(),
            max_bytes: None,
            encoding: None,
        }))
        .await
        .expect("read binary attachment succeeds");

    let read_bin_res: DocumentReadAttachmentResult =
        serde_json::from_str(&read_bin_json).expect("parse read binary JSON");
    assert_eq!(read_bin_res.filename, "signature.p7s");
    assert_eq!(read_bin_res.encoding, "base64");
    assert!(!read_bin_res.truncated);

    // 4. Test max_bytes truncation
    let read_trunc_json = server
        .document_read_attachment(Parameters(DocumentReadAttachmentParams {
            document_id: "doc_with_attachments".to_string(),
            name_or_id: "factur-x.xml".to_string(),
            max_bytes: Some(10),
            encoding: None,
        }))
        .await
        .expect("read truncated attachment succeeds");

    let read_trunc_res: DocumentReadAttachmentResult =
        serde_json::from_str(&read_trunc_json).expect("parse read truncated JSON");
    assert!(read_trunc_res.truncated);
    assert_eq!(read_trunc_res.content.len(), 10);

    // 5. Test document_info reports attachments
    let info_json = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "doc_with_attachments".to_string(),
        }))
        .await
        .expect("info must succeed for an indexed document");

    let info_res: DocumentInfoResult = serde_json::from_str(&info_json).expect("parse info JSON");
    assert!(info_res.has_attachments);
    assert_eq!(info_res.total_attachments, 2);
}

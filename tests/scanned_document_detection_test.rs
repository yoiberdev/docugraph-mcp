use docugraph::document::{
    Document, DocumentMetadata, Page, PageKind, inspect_page_images, load_pdf_from_path,
};
use docugraph::mcp::{DocuGraphServer, tools::*};
use lopdf::content::{Content, Operation};
use lopdf::{Document as LopdfDoc, Object, Stream, dictionary};
use rmcp::handler::server::wrapper::Parameters;

fn create_synthetic_page_with_image(
    ops: Vec<Operation>,
    img_width: i64,
    img_height: i64,
) -> (LopdfDoc, (u32, u16)) {
    let mut doc = LopdfDoc::with_version("1.5");
    let pages_id = doc.new_object_id();

    // Create an Image XObject stream
    let image_stream_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => img_width,
            "Height" => img_height,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
        },
        vec![255; (img_width * img_height * 3).min(1024) as usize],
    ));

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });

    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => font_id,
        },
        "XObject" => dictionary! {
            "Im0" => Object::Reference(image_stream_id),
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

#[test]
fn test_inspect_page_images_detection() {
    let ops = vec![Operation::new("BT", vec![]), Operation::new("ET", vec![])];
    let (doc, page_id) = create_synthetic_page_with_image(ops, 800, 1200);

    let images = inspect_page_images(&doc, page_id);
    assert_eq!(images.len(), 1, "Should detect 1 image XObject");
    assert_eq!(images[0].0, 800, "Image width should match");
    assert_eq!(images[0].1, 1200, "Image height should match");
}

#[test]
fn test_scanned_page_detection_without_text() {
    let ops = vec![
        // Empty text stream with an image XObject invocation
        Operation::new("q", vec![]),
        Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
        Operation::new("Q", vec![]),
    ];
    let (mut doc, _) = create_synthetic_page_with_image(ops, 1600, 2400);

    let temp_dir = std::env::temp_dir();
    let temp_pdf_path = temp_dir.join("docugraph_test_scanned_page.pdf");
    doc.save(&temp_pdf_path)
        .expect("Saving synthetic scanned PDF should succeed");

    let parsed =
        load_pdf_from_path(&temp_pdf_path).expect("Loading synthetic scanned PDF should succeed");
    let _ = std::fs::remove_file(&temp_pdf_path);

    assert_eq!(parsed.metadata.scanned_pages_count, 1);
    assert_eq!(parsed.pages.len(), 1);

    let page = &parsed.pages[0];
    assert_eq!(page.kind, PageKind::ScannedImage);
    assert_eq!(page.image_count, 1);
    assert!(
        page.text
            .contains("es una imagen escaneada sin capa de texto"),
        "Should contain actionable warning message for LLM, found: '{}'",
        page.text
    );
    assert!(
        page.text.contains("OCR"),
        "Warning message should recommend OCR"
    );
}

#[test]
fn test_digital_page_with_diagram_not_falsely_flagged() {
    let prose = "Capítulo 3: Arquitectura y Especificación de Componentes.\n\
                 El sistema utiliza una topología en capas distribuidas donde los servicios comunican \
                 eventos a través de un bus transaccional seguro y desacoplado.";

    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.0.into()]),
        Operation::new("Tj", vec![Object::string_literal(prose)]),
        Operation::new("ET", vec![]),
        Operation::new("q", vec![]),
        Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
        Operation::new("Q", vec![]),
    ];
    let (mut doc, _) = create_synthetic_page_with_image(ops, 400, 300);

    let temp_dir = std::env::temp_dir();
    let temp_pdf_path = temp_dir.join("docugraph_test_digital_with_image.pdf");
    doc.save(&temp_pdf_path)
        .expect("Saving synthetic PDF should succeed");

    let parsed = load_pdf_from_path(&temp_pdf_path).expect("Loading synthetic PDF should succeed");
    let _ = std::fs::remove_file(&temp_pdf_path);

    assert_eq!(
        parsed.metadata.scanned_pages_count, 0,
        "Page with digital text should NOT be counted as scanned"
    );
    assert_eq!(parsed.pages[0].kind, PageKind::DigitalText);
    assert_eq!(parsed.pages[0].image_count, 1);
    assert!(
        !parsed.pages[0].text.contains("es una imagen escaneada"),
        "Normal digital text should not be wrapped in scanned warning"
    );
}

#[test]
fn test_blank_page_classified_as_empty() {
    let mut doc = LopdfDoc::with_version("1.5");
    let pages_id = doc.new_object_id();

    let content = Content { operations: vec![] };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let temp_dir = std::env::temp_dir();
    let temp_pdf_path = temp_dir.join("docugraph_test_empty_page.pdf");
    doc.save(&temp_pdf_path)
        .expect("Saving blank PDF should succeed");

    let parsed = load_pdf_from_path(&temp_pdf_path).expect("Loading blank PDF should succeed");
    let _ = std::fs::remove_file(&temp_pdf_path);

    assert_eq!(parsed.pages[0].kind, PageKind::Empty);
    assert_eq!(parsed.metadata.scanned_pages_count, 0);
}

#[tokio::test]
async fn test_mcp_server_scanned_warnings_in_tools() {
    let cache = tempfile::tempdir().expect("create temp cache dir");
    let server = DocuGraphServer::with_cache_dir(cache.path());
    let mut doc = Document::new(DocumentMetadata {
        id: "doc-with-scans".to_string(),
        title: "Manual Escaneado".to_string(),
        author: None,
        total_pages: 2,
        total_sections: 0,
        file_size_bytes: 5000,
        content_hash: "scan_hash123".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 1,
        source_path: None,
        total_links: 0,
        ..Default::default()
    });

    let mut page_1 = Page::new(
        1,
        "[Aviso: La página 1 es una imagen escaneada sin capa de texto digital. Se requiere OCR.]",
    );
    page_1.kind = PageKind::ScannedImage;
    page_1.image_count = 1;
    doc.add_page(page_1);

    let page_2 = Page::new(2, "Página 2 contiene texto digital perfectamente legible.");
    doc.add_page(page_2);

    server.register_document(doc).await;

    // Test document_info returns scan warning
    let info_json = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "doc-with-scans".to_string(),
        }))
        .await
        .expect("tool call should succeed");
    let info: DocumentInfoResult =
        serde_json::from_str(&info_json).expect("valid DocumentInfoResult");
    assert_eq!(info.scanned_pages_count, 1);
    assert!(info.scan_warning.is_some());
    assert!(info.scan_warning.unwrap().contains("OCR"));

    // Test document_read_pages header formatting
    let read_json = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "doc-with-scans".to_string(),
            page_start: 1,
            page_end: 2,
            max_chars: Some(4000),
        }))
        .await
        .expect("tool call should succeed");

    assert!(
        read_json.contains("--- Página 1 [📷 Imagen Escaneada / Sin Capa de Texto] ---"),
        "Read pages should header-tag scanned page, found: {}",
        read_json
    );
    assert!(read_json.contains("--- Página 2 ---"));
}

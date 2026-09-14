use docugraph::document::model::{Document, DocumentMetadata, Page, PageKind};
use docugraph::mcp::DocuGraphServer;
use docugraph::mcp::tools::RenderPageParams;
use docugraph::multimodal::{CachedPageRendererProxy, buffer::RgbaImageBuffer, png::encode_png};
use docugraph::storage::DocumentStore;
use rmcp::handler::server::wrapper::Parameters;
use tempfile::tempdir;

#[test]
fn test_png_encoder_validity() {
    // 2x2 RGBA test image: red, green, blue, white
    let width = 2;
    let height = 2;
    let data = vec![
        255, 0, 0, 255, // Top-left: red
        0, 255, 0, 255, // Top-right: green
        0, 0, 255, 255, // Bottom-left: blue
        255, 255, 255, 255, // Bottom-right: white
    ];

    let png_bytes = encode_png(width, height, &data).expect("Encoding PNG must succeed");

    // Verify 8-byte standard PNG magic header
    assert!(png_bytes.len() > 8);
    assert_eq!(&png_bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);

    // Verify IHDR chunk exists immediately after magic header
    assert_eq!(&png_bytes[12..16], b"IHDR");

    // Verify dimensions in IHDR
    let w = u32::from_be_bytes(png_bytes[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png_bytes[20..24].try_into().unwrap());
    assert_eq!(w, 2);
    assert_eq!(h, 2);

    // Verify bit depth 8 and color type 6 (RGBA)
    assert_eq!(png_bytes[24], 8);
    assert_eq!(png_bytes[25], 6);

    // Verify file ends with IEND chunk
    assert!(png_bytes.ends_with(b"IEND\xaeB`\x82"));
}

#[test]
fn test_rgba_image_buffer_drawing_and_base64() {
    let mut buffer = RgbaImageBuffer::new(50, 50, [255, 255, 255, 255]);

    // Draw filled rect (blue)
    buffer.draw_filled_rect(5, 5, 20, 20, [0, 0, 255, 255]);

    // Draw outline rect (red)
    buffer.draw_rect(30, 5, 15, 15, 2, [255, 0, 0, 255]);

    // Blit 2x2 bitmap
    let icon_data = vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
    buffer.draw_bitmap(40, 40, 2, 2, 2, 2, &icon_data, 4);

    let png_bytes = buffer.to_png_bytes().expect("PNG generation");
    assert!(!png_bytes.is_empty());

    let base64_str = buffer.to_base64_png().expect("Base64 PNG generation");
    // Standard PNG base64 starts with iVBORw0KGgo
    assert!(
        base64_str.starts_with("iVBORw0KGgo"),
        "Base64 PNG header mismatch: {base64_str}"
    );
}

#[test]
fn test_cached_page_renderer_proxy_hit_and_miss() {
    let temp = tempdir().expect("Failed to create tempdir");
    let cache_dir = temp.path().to_path_buf();
    let proxy = CachedPageRendererProxy::new(Some(&cache_dir));

    let mut doc = Document::new(DocumentMetadata {
        id: "proxy-test-doc".to_string(),
        title: "Test Architecture Doc".to_string(),
        author: Some("Author".to_string()),
        total_pages: 2,
        total_sections: 1,
        file_size_bytes: 4096,
        content_hash: "mockhash1234".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
    });

    let mut p1 = Page::new(1, "Page 1 digital text with high density content.");
    p1.kind = PageKind::DigitalText;
    doc.add_page(p1);

    // First render: cache miss
    let res1 = proxy
        .render_document_page(&doc, 1, 600)
        .expect("First render succeeds");
    assert!(!res1.from_cache, "First call must be a fresh render");
    assert_eq!(res1.width, 600);
    assert!(res1.height > 600);
    assert!(!res1.png_bytes.is_empty());

    // Verify cache file was written to disk
    let cached_file = cache_dir.join("renders").join("mockhash1234_p1_w600.png");
    assert!(
        cached_file.exists(),
        "Cached file must exist on disk: {}",
        cached_file.display()
    );

    // Second render: cache hit
    let res2 = proxy
        .render_document_page(&doc, 1, 600)
        .expect("Second render succeeds");
    assert!(res2.from_cache, "Second call must hit disk cache");
    assert_eq!(res2.png_bytes, res1.png_bytes);
    assert_eq!(res2.base64_data, res1.base64_data);
}

#[tokio::test]
async fn test_mcp_document_render_page_tool() {
    let store = DocumentStore::new(None);
    let temp = tempdir().expect("Failed to create tempdir");
    let proxy = CachedPageRendererProxy::new(Some(temp.path()));
    let server = DocuGraphServer::with_store_and_renderer(store, proxy);

    let mut doc = Document::new(DocumentMetadata {
        id: "multimodal-doc".to_string(),
        title: "Multimodal Visual Document".to_string(),
        author: None,
        total_pages: 1,
        total_sections: 0,
        file_size_bytes: 2048,
        content_hash: "multi12345".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
    });
    doc.add_page(Page::new(1, "Page 1 Content with architecture diagram."));
    server.register_document(doc).await;

    // Call document_render_page MCP tool
    let params = RenderPageParams {
        document_id: "multimodal-doc".to_string(),
        page_number: 1,
        max_width: Some(800),
    };
    let json_resp = server.document_render_page(Parameters(params)).await;

    let parsed: serde_json::Value =
        serde_json::from_str(&json_resp).expect("Valid JSON response from tool");
    assert_eq!(parsed["document_id"], "multimodal-doc");
    assert_eq!(parsed["page_number"], 1);
    assert_eq!(parsed["mime_type"], "image/png");
    assert_eq!(parsed["width"], 800);
    assert!(
        parsed["base64_image"]
            .as_str()
            .unwrap()
            .starts_with("iVBORw0KGgo")
    );
    assert!(
        parsed["data_uri"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,iVBORw0KGgo")
    );

    // Test requesting non-existent document
    let invalid_params = RenderPageParams {
        document_id: "non-existent-doc".to_string(),
        page_number: 1,
        max_width: None,
    };
    let err_json = server
        .document_render_page(Parameters(invalid_params))
        .await;
    let err_parsed: serde_json::Value = serde_json::from_str(&err_json).expect("Valid error JSON");
    assert!(err_parsed["error"].as_str().unwrap().contains("not found"));
}

#[test]
fn test_native_page_renderer_with_lopdf() {
    use docugraph::multimodal::renderer::{NativePageRenderer, PageRenderer};
    use lopdf::{Object, dictionary};

    let mut pdf_doc = lopdf::Document::with_version("1.5");
    let pages_id = pdf_doc.new_object_id();
    let page_id = pdf_doc.new_object_id();

    let page_dict = dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    };
    pdf_doc
        .objects
        .insert(page_id, Object::Dictionary(page_dict));

    let pages_dict = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    pdf_doc
        .objects
        .insert(pages_id, Object::Dictionary(pages_dict));

    let renderer = NativePageRenderer::new();
    let buffer = renderer
        .render_page(&pdf_doc, page_id, 1, 612)
        .expect("Render page succeeds");

    assert_eq!(buffer.width, 612);
    assert_eq!(buffer.height, 792);
    let png = buffer.to_png_bytes().expect("PNG bytes generation");
    assert!(!png.is_empty());
}

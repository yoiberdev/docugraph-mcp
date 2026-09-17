//! PDF document parsing, modelling and structural extraction.

pub mod attachments;
pub mod forms;
pub mod layout;
pub mod links;
pub mod model;
pub mod outline_strategy;
pub mod parser;
pub mod provenance;
pub mod structure;
pub mod table;
pub mod tagged;

/// Ceiling on how many bytes one PDF stream may expand to when decompressed.
///
/// PDFs are untrusted here, and a Flate stream can expand by a factor of a
/// thousand or more: a 329 KB file drove peak memory past 15 GB before this,
/// because nothing bounded the expansion and each page is decoded more than once
/// during ingestion. 64 MB is far above any real page's content stream and far
/// below anything that threatens the host.
pub const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

pub use attachments::{extract_document_attachments, safe_output_name, safe_output_path};
pub use forms::extract_document_forms;
pub use layout::{
    BoundingBox, MultiColumnSpatialFlow, ReadingOrderStrategy, SingleColumnFlow, TextFragment,
    TextLine, extract_page_text_spatial,
};
pub use links::{
    decode_pdf_string, extract_page_links, object_to_string, resolve_dest,
    resolve_named_destination,
};
pub use model::{
    Document, DocumentId, DocumentLink, DocumentMetadata, EmbeddedAttachment, FormField,
    FormFieldType, LinkTarget, Page, PageKind, SectionNode,
};
pub use outline_strategy::{
    FallbackOutlineStrategy, NativeOutlineExtractor, OutlineExtractor, TypographicOutlineExtractor,
};
pub use parser::{
    PageSecurityScan, inspect_page_images, load_pdf_from_path, load_pdf_from_path_with_password,
    scan_page_security,
};
pub use provenance::Provenance;
pub use table::{
    MarkdownTableBuilder, TableAlignment, TableStructureVisitor, reconstruct_tables_in_text,
    split_line_into_cells,
};
pub use tagged::{TaggedPdfInfo, detect_tagged_pdf_structure};

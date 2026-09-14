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

pub use attachments::extract_document_attachments;
pub use forms::extract_document_forms;
pub use layout::{
    BoundingBox, MultiColumnSpatialFlow, ReadingOrderStrategy, SingleColumnFlow, TextFragment,
    TextLine, extract_page_text, extract_page_text_spatial, join_fragments_in_stream_order,
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

//! Document ingestion, layout models, and provenance tracking.

pub mod model;
pub mod parser;
pub mod provenance;
pub mod structure;

pub use model::{Document, DocumentId, DocumentMetadata, Page, PageKind, SectionNode};
pub use parser::{
    PageSecurityScan, inspect_page_images, load_pdf_from_path, load_pdf_from_path_with_password,
    scan_page_security,
};
pub use provenance::Provenance;

//! Document ingestion, layout models, and provenance tracking.

pub mod model;
pub mod parser;
pub mod provenance;
pub mod structure;

pub use model::{Document, DocumentId, DocumentMetadata, Page, SectionNode};
pub use parser::{
    PageSecurityScan, load_pdf_from_path, load_pdf_from_path_with_password, scan_page_security,
};
pub use provenance::Provenance;

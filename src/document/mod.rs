//! Document ingestion, layout models, and provenance tracking.

pub mod model;
pub mod parser;
pub mod provenance;
pub mod structure;

pub use model::{Document, DocumentId, DocumentMetadata, Page, SectionNode};
pub use parser::load_pdf_from_path;
pub use provenance::Provenance;

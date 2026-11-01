//! Anāhata Library: Database, Migrations, Scanner, and Metadata

pub mod db;
pub mod error;
pub mod metadata;
pub mod queries;
pub mod scanner;

pub use db::connection::{Database, ReaderPool, WriterHandle};
pub use db::migrations::run_migrations;
pub use error::{LibraryError, LibraryResult};
pub use metadata::{extract_metadata, TrackMetadata};
pub use queries::{AlbumRow, Queries, TrackRow};
pub use scanner::{compute_fingerprint, ScanSummary, Scanner};

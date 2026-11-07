//! Institution parsers. Each module implements
//! [`financi_core::StatementParser`] for one export format.
//!
//! Simplii Visa is still to come (needs a sample export).

use financi_core::StatementParser;

pub mod amex;
pub mod simplii_chequing;

/// Every known parser, in auto-detect order.
pub fn all() -> Vec<Box<dyn StatementParser + Send + Sync>> {
    vec![
        Box::new(amex::Amex),
        Box::new(simplii_chequing::SimpliiChequing),
    ]
}

/// The first parser whose `detect` accepts this header.
pub fn detect(header: &[String]) -> Option<Box<dyn StatementParser + Send + Sync>> {
    all().into_iter().find(|p| p.detect(header))
}

/// Look a parser up by its stable id (as stored in `imports.parser_id`).
pub fn by_id(id: &str) -> Option<Box<dyn StatementParser + Send + Sync>> {
    all().into_iter().find(|p| p.id() == id)
}

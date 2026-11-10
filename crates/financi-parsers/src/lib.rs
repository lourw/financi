//! Institution parsers. Each module implements
//! [`financi_core::StatementParser`] for one export format.

use financi_core::StatementParser;

pub mod amex;
pub mod simplii;

/// Every known parser, in auto-detect order.
pub fn all() -> Vec<Box<dyn StatementParser + Send + Sync>> {
    vec![Box::new(amex::Amex), Box::new(simplii::Simplii)]
}

/// The first parser whose `detect` accepts this header.
pub fn detect(header: &[String]) -> Option<Box<dyn StatementParser + Send + Sync>> {
    all().into_iter().find(|p| p.detect(header))
}

/// Look a parser up by its stable id (as stored in `imports.parser_id`).
/// Retired ids keep resolving to the parser that replaced them, so rows
/// already in the database still work.
pub fn by_id(id: &str) -> Option<Box<dyn StatementParser + Send + Sync>> {
    let id = match id {
        // Task 4.1 folded the chequing parser into the one `simplii` parser.
        "simplii-chequing" => "simplii",
        other => other,
    };
    all().into_iter().find(|p| p.id() == id)
}

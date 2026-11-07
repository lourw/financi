//! Institution parsers. Each module implements
//! [`financi_core::StatementParser`] for one export format.
//!
//! M1 adds `amex`; M4 adds `simplii_chequing` and `simplii_visa`.

use financi_core::StatementParser;

/// Every known parser, in auto-detect order.
pub fn all() -> Vec<Box<dyn StatementParser>> {
    Vec::new()
}

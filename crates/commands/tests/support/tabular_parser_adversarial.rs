//! Independent parser cancellation canaries. These exercise source coordinates
//! that advance two bytes at a time and therefore skip modulo checkpoints.
use super::*;
use std::cell::Cell;

#[test]
fn independent_escaped_quotes_cannot_skip_all_cancellation_checkpoints() {
    // Quoted content begins at odd byte 1. Every escaped quote advances by 2,
    // so absolute offsets never equal a multiple of 4096 before the close.
    let text = format!("\"{}\"", "\"\"".repeat(128 * 1024));
    let mut parser = Records::new(&text, b',', b'"');
    let checks = Cell::new(0);
    let result = parser.next(&|| {
        checks.set(checks.get() + 1);
        true
    });
    assert!(
        result.is_err(),
        "cancelled quoted parsing must not process the complete long cell; checks={}, offset={}",
        checks.get(),
        parser.offset
    );
    assert!(
        parser.offset <= 4097,
        "cancellation must be checked within bounded parsing work"
    );
}

#[test]
fn independent_quoted_crlf_cannot_skip_all_cancellation_checkpoints() {
    let text = format!("\"{}\"", "\r\n".repeat(128 * 1024));
    let mut parser = Records::new(&text, b',', b'"');
    let result = parser.next(&|| true);
    assert!(
        result.is_err(),
        "cancelled CRLF parsing must not process the complete long cell"
    );
    assert!(parser.offset <= 4097);
}

# M3 — backend queue: scale, then correctness

Runs beside the UI queue, in the order listed. Backend only: don't read the UI docs for these. 5.1 and 5.2 (`docs/tasks/m5.md`) follow 3.90.

## Backend queue: scale, then correctness (in this order)

Runs beside the UI queue. 5.1 and 5.2 (in M5) follow 3.90.

- [x] **3.71 Row hashes don't depend on the encoding guess.** (Code
  review, 2026-11-11.) Parsers hash `record.fields`, which are decoded
  strings, and the encoding is guessed once per file: one invalid UTF-8
  byte makes the whole file Windows-1252. Then every correctly encoded
  `É` in that file decodes as `Ã‰`, those rows hash differently from the
  same rows imported earlier from a clean file, and they're imported
  twice (near-match misses them too). With a BOM, `from_utf8_lossy` maps
  different bad bytes to the same U+FFFD, so distinct rows can share a
  hash and real rows are dropped.
  - Keep each field's undecoded bytes on `RawRecord` (quotes removed) and
    hash those, length-prefixed as now.
  - This changes the hash of rows with non-ASCII characters already in
    the database. **Stop and ask the user** with two options and the
    count of affected rows in a copy of their DB (counts only): (a) a
    migration that re-hashes every row from its stored `raw_bytes` and
    rewrites `row_aliases`, or (b) keep decoded hashing for Amex and
    Simplii and use byte hashing only for new parsers, documented in
    `plan.md`.
  - Tests: an all-ASCII row's hash is unchanged; the same accented row in
    a clean UTF-8 file and in a file with one stray invalid byte hashes
    the same; two rows differing only in invalid bytes hash differently.

- [ ] **3.94 CSV loading runs off the UI thread.** (Follow-up split from
  3.67, 2026-11-12; run after 3.74 has merged — it touches
  `ui/imports.rs`.) `Shell::open_file` calls `flow::load_file` on the UI
  thread, so a large or pathological file freezes the window while it
  parses. Run it on the background executor with an update hop back to
  the entity (the pattern the other DbWorker call sites in
  `ui/imports.rs` use), keeping the toast and error reporting as they
  are. While there, check the file's size against the 50 MB limit via
  `path.metadata()` *before* `std::fs::read`, so a huge drop is
  rejected without loading it into memory (3.67 review note).

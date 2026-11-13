# M3 — backend queue: scale, then correctness

Runs beside the UI queue, in the order listed. Backend only: don't read the UI docs for these. 5.1 and 5.2 (`docs/tasks/m5.md`) follow 3.90.

## Backend queue: scale, then correctness (in this order)

Runs beside the UI queue. 5.1 and 5.2 (in M5) follow 3.90.

- [x] **3.94 CSV loading runs off the UI thread.** (Follow-up split from
  3.67, 2026-11-12; run after 3.74 has merged — it touches
  `ui/imports.rs`.) `Shell::open_file` calls `flow::load_file` on the UI
  thread, so a large or pathological file freezes the window while it
  parses. Run it on the background executor with an update hop back to
  the entity (the pattern the other DbWorker call sites in
  `ui/imports.rs` use), keeping the toast and error reporting as they
  are. While there, check the file's size against the 50 MB limit via
  `path.metadata()` *before* `std::fs::read`, so a huge drop is
  rejected without loading it into memory (3.67 review note).

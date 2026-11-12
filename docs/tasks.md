# financi — implementation tasks (index)

This is the entry point for the rest of v1. It turns `plan.md` (the
requirements) into tasks; where the two disagree, the task files win for
M2–M5 detail. Read this file whole; then read only the file that holds
the task you're doing.

| File | What | Read it |
|---|---|---|
| `docs/tasks.md` | this index: how to work, the order, where things are | always |
| `docs/tasks/gpui.md` | GPUI target architecture (step by step), coding practices, pitfalls | before any UI task |
| `docs/tasks/m3-ui-queue.md` | the architecture refactor, in order | when doing one of them |
| `docs/tasks/m3-backend-queue.md` | backend scale and correctness fixes, in order | when doing one of them |
| `docs/tasks/m3-features.md` | feature lanes A / B / C, then 3.26 | when doing one of them |
| `docs/tasks/m5.md` | export / restore | when doing one of them |
| `docs/tasks/final.md` | 3.55, performance at scale (last) | when doing it |
| `docs/tasks/done.md` | archive of finished tasks | only grep it for a task number an open task refers to |

To find a task: `grep -rn '\*\*3.64 ' docs/tasks/`.

## How to work

1. Pick tasks exactly as *Picking the next task* says. When every box in a
   milestone is ticked, continue to the next without waiting for a
   go-ahead. Stop and report to the user when v1 is complete (the *Final*
   task done).
2. Keep **two sub-agents in flight** whenever two tasks can run in parallel
   (`AGENTS.md` → *Keep the pool full*). Tick a box (`- [x]`) in the same
   commit that completes it.
3. **Ask before deviating.** If a task is unclear, an API doesn't fit, or you
   would have to guess at behaviour, stop and ask. Don't improvise.
4. Every task lands with tests and synthetic data (see `CLAUDE.md`). Never
   put real bank data in the repo.
5. Commit after each logical piece (usually one task), Conventional Commits,
   then verify that commit in a clean checkout (see `CLAUDE.md`).
6. When a task is merged, move its text from its file to
   `docs/tasks/done.md` (under its milestone, in number order) in the same
   merge commit or a `docs:` commit right after. Only open tasks stay here.
7. New user requests and bug reports become numbered tasks (next free
   number) in the file for the step they belong to, and are added to the
   order below.
8. **Writing tasks:** name the component and token (`DataList` with
   columns …, `InlineForm`) instead of reciting pixels; restate the one
   behaviour a task depends on instead of pointing at a done task's
   number; split a task that mixes backend, UI and performance work. When
   a task contradicts `docs/ui-conventions.md`, change the convention (in
   the same commit, reviewed for consistency) instead of adding an
   exception.

## Picking the next task

Task numbers record when a task was written, not when to do it. Each step names the file its tasks live in. **The
architecture and scalability work comes first** (user decision,
2026-11-11): feature tasks wait until it has landed. Two queues run side by
side, one task from each, so both slots stay busy. Start every task's
worktree from the current `main`.

1. **Two queues, in order:**
   - **UI queue** (`docs/tasks/m3-ui-queue.md`; architecture and GPUI best
     practices): 3.62 → 3.74 →
     3.75 → 3.64 → 3.65 → 3.76 → 3.78 → 3.56 → 3.89 → 3.79 → 3.80 → 3.81
     → 3.82 → 3.83 → 3.91 → 3.57 → 3.84 → 3.85 → 3.86. One at a time, except that
     once 3.79 has merged, two of 3.80–3.83 may run together (they live in
     different modules).
   - **Backend queue** (`docs/tasks/m3-backend-queue.md`; scale, then
     correctness): 3.77 → 3.73 → 3.88 →
     3.87 → 3.66 → 3.67 → 3.68 → 3.69 → 3.70 → 3.72 → 3.71 → 3.90 → 5.1
     → 5.2. These don't touch the UI, so one always runs beside the UI
     queue (or a second one when the UI queue is waiting).
   - **Dependencies:** 3.78 needs 3.77 merged; 3.56 needs 3.77, 3.78 and
     3.88; 3.84 needs 3.64. If the next UI task is blocked, the slot takes
     the next backend task.
2. **Then the feature lanes** (`docs/tasks/m3-features.md`), one task from each of two lanes at a time:
   - **Lane A, dashboard:** 3.36 → 3.45 → 3.46.
   - **Lane B, Categories and Rules:** 3.58 → 3.48 → 3.50 + 3.51 (one
     branch) → 3.49 → 3.43 → 3.60 → 3.52 → 3.47 + 3.54 (one branch).
   - **Lane C, small tasks:** 3.63 → 3.37 → 3.61 → 3.59 (after 3.51 and
     3.58).
   - Backend tasks left over from step 1 fill any idle slot.
3. **Last in M3: 3.26** (scrollbars, in `docs/tasks/m3-features.md`), once
   the lanes are empty.
4. **Then M5** (`docs/tasks/m5.md`: 5.3, 5.4 and whatever of 5.1 / 5.2 is left), **then the
   Final task** (`docs/tasks/final.md`: 3.55, performance at scale).

## Where things are

| Path | What |
|---|---|
| `crates/financi-core` | `raw` (`RawFile`/`RawRecord`, `read_raw_file`), `parse` (cents, dates, merchant), `dates`, `rules` (`CompiledRules`, `pick_category`, `winning_rule`), `StatementParser` trait, `row_hash` |
| `crates/financi-parsers` | `amex`, `simplii` (old id `simplii-chequing` still resolves); `all()`, `detect()`, `by_id()` |
| `crates/financi-db` | `migrations/0001`–`0004` (0001 frozen), `accounts`, `import`, `master_csv`, `categories`, `rules`, `overrides`, `queue`, `views`, `transfers`, `settings`, `worker::DbWorker`, `seed` (feature `dev-seed`) |
| `crates/financi/src/flow.rs` | GPUI-free helpers (formatting, import flow), unit-tested |
| `crates/financi/src/ui/` | every view: `mod.rs` holds `Shell`, one module per page plus `widgets.rs` (shared components after 3.89) |
| `crates/financi/src/ui/tests/` | headless UI tests, one module per page plus shared helpers in `mod.rs` |
| `crates/financi/src/version.rs`, `build.rs` | build version from `git describe` (3.39) |
| `assets/icon/` | app icon source (3.63) |
| `fixtures/demo/` | synthetic exports + `expected.json` reference totals |
| `spikes/gpui-spike` | sandbox for UI experiments; keep, don't ship |
| `docs/ui-conventions.md` | design system: tokens, components, patterns, pages; read before any UI task |

## After v1 (not now)

v1.1 spend-over-time line + top merchants · v1.2 flags (large, new merchant,
FX/fees) · v1.3 recurring/subscriptions · default categories and merchant
rules on first run (`plan.md` → *Categorization*; moved out of v1
2026-11-11) · more institutions (first widen the parser trait: `detect`
sees the file, not just the header row; the parser controls header /
preamble rows; it supplies its institution's display name, replacing the
match in `flow::format_label`) · CSV export of filtered rows. Real Simplii
Visa sample → revisit 4.1.

# financi — implementation tasks (index)

This is the entry point for the rest of v1. It turns `plan.md` (the
requirements) into tasks; where the two disagree, the task files win for
M2–M5 detail. Read this file whole; then read only the file that holds
the task you're doing.

| File | What | Read it |
|---|---|---|
| `docs/tasks.md` | this index: how to work, the order, where things are | always |
| `docs/tasks/gpui.md` | GPUI target architecture, coding practices, pitfalls | before any UI task |
| `docs/tasks/m3-features.md` | the rest of M3: lanes A / B / C, housekeeping, then 3.26 | when doing one of them |
| `docs/tasks/m5.md` | 5.4, the v1 acceptance tests | when doing it |
| `docs/tasks/final.md` | 3.55, performance at scale (last of v1) | when doing it |
| `docs/tasks/post-v1.md` | the roadmap after v1: M6–M10 (v1.1–v1.5) | only once v1 is done |
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

Task numbers record when a task was written, not when to do it. The
architecture and backend queues and M4 are done, and so are 5.1–5.3; what's
left of v1 is below (re-planned 2026-11-17 from the user's latest
reports). Start every task's worktree from the current `main`.

1. **The M3 lanes** (`docs/tasks/m3-features.md`), one task from each
   of two lanes at a time, in this order within each lane:
   - **Lane A, dashboard:** 3.98 → 3.97 → 3.100 → 3.101 → 3.99.
     (Labels fix, panels as entities, dock layout, saved layout, pie
     chart.)
   - **Lane B, Categories and Rules:** 3.102 → 3.103 → 3.47 + 3.54 (one
     branch).
   - **Lane C, small tasks:** 3.105 → 3.95 → 3.104. Start lane C first:
     3.105 is a broken page, and 3.95 is a repeat user request.
   - **Housekeeping:** H5 fills an idle slot when nothing in flight
     touches its files (pure clean-up, own commit).
   - **Dependencies:** 3.103 needs 3.102; 3.100 needs 3.97; 3.101 and
     3.99 need 3.100. 3.105 and 3.95 both edit `ui/queue.rs`, so they
     never run together.
2. **Last in M3: 3.26** (scrollbars), once the lanes and H5 are done.
3. **Then M5: 5.4** (`docs/tasks/m5.md`), **then the Final task**
   (`docs/tasks/final.md`: 3.55, performance at scale). v1 is complete
   when 3.55 merges: stop and report to the user.
4. **After v1** (`docs/tasks/post-v1.md`): M6 → M10, only once the user
   says to start.

## Where things are

| Path | What |
|---|---|
| `crates/financi-core` | `raw` (`RawFile`/`RawRecord`, `read_raw_file`), `parse` (cents, dates, merchant), `dates`, `rules` (`CompiledRules`, `pick_category`, `winning_rule`), `StatementParser` trait, `row_hash` |
| `crates/financi-parsers` | `amex`, `simplii` (old id `simplii-chequing` still resolves); `all()`, `detect()`, `by_id()` |
| `crates/financi-db` | `migrations/0001`–`0006` (0001 frozen), `accounts`, `import`, `master_csv`, `categories`, `rules`, `overrides`, `queue`, `views`, `transfers`, `settings`, `export` (5.1 / 5.2), `worker::DbWorker`, `seed` (feature `dev-seed`) |
| `crates/financi/src/flow.rs` | GPUI-free helpers (formatting, import flow), unit-tested |
| `crates/financi/src/ui/` | every view: `mod.rs` holds `Shell`, one module per page plus `widgets.rs` (shared components after 3.89); a page that outgrows one file becomes a `ui/<page>/` directory (`AGENTS.md` → *File size*) |
| `crates/financi/src/ui/tests/` | headless UI tests, one module per page (or a `tests/<page>/` directory split by feature) plus shared helpers in `mod.rs` |
| `crates/financi/src/version.rs`, `build.rs` | build version from `git describe` (3.39) |
| `assets/icon/` | app icon source (3.63) |
| `fixtures/demo/` | synthetic exports + `expected.json` reference totals |
| `spikes/gpui-spike` | sandbox for UI experiments; keep, don't ship |
| `docs/ui-conventions.md` | design system: tokens, components, patterns, pages; read before any UI task |

## After v1 (not now)

The roadmap and its tasks are in `docs/tasks/post-v1.md`: v1.1 spending
over time and top merchants, v1.2 flags, v1.3 recurring and
subscriptions, v1.4 first-run defaults and CSV export of filtered rows,
v1.5 more institutions (widen the parser trait first; revisit 4.1 if a
real Simplii Visa sample turns up).

# financi

Personal finance desktop app: imports bank CSV exports (Amex, Simplii) into
SQLite and shows spending in a GPUI window. Single user, offline. macOS and
Linux.

**Paths:** every path in these docs is relative to the root of the worktree
you're working in, not the main checkout. In a feature worktree, read docs
and code from that worktree, never from the main checkout's copy.

**Read first:** `plan.md` (requirements and decisions) and `docs/tasks.md`
(the task index: how to work, the order, and which file under `docs/tasks/`
holds each task; the task files win over plan.md on M2–M5 detail). Read
only the task file you need. Done tasks are archived in
`docs/tasks/done.md`; don't read it unless an open task points to one. Work one milestone at a time (with the
exceptions *Picking the next task* lists) and continue to the next without
checking in; stop when v1 is complete. Ask before deviating from a task.

**Agent workflow:** `AGENTS.md` defines the roles (one orchestrator and a
shared pool of at most two implementer or reviewer sub-agents), the
130k-token context budget per agent, when tasks may run in parallel, and
the handoff templates. Follow it.

**UI docs are for UI work only.** Read `docs/ui-conventions.md` and the
`docs/tasks/gpui.md` (target architecture, coding practices, pitfalls)
only when the task changes the GPUI app
(`crates/financi/src/ui*.rs`, `crates/financi/src/ui/` after task 3.62, or
other view code). Skip them for backend
work (parsers, `financi-core`, `financi-db`, migrations, `flow.rs`
helpers); they cost context and don't apply.

## Commands

Toolchain is pinned in `mise.toml` (`mise install`; tasks: `mise run
build|test|lint|fmt|run`). Run `git config core.hooksPath .githooks` once
per clone so commits on `main` get their semver tag. Whenever `main` is
pushed, push its tags too: `git push --follow-tags`.

```sh
cargo build --workspace
cargo test --workspace --features financi-db/dev-seed      # all tests
cargo clippy --workspace --all-targets --features financi-db/dev-seed
cargo fmt --all
cargo run -p financi                                          # the app
```

Dev switches (debug builds only):

- `FINANCI_DB=/tmp/dev.sqlite` — use a scratch DB, never the real one in the
  OS app data dir.
- `FINANCI_DEV_SEED=1` — fill an empty DB with synthetic data.
- `FINANCI_DEV_OPEN=fixtures/demo/amex-sep-nov.csv` — open a file's import
  preview at startup.

Opt-in check against real exports on this machine (prints counts only):
`FINANCI_SAMPLE_AMEX=… FINANCI_SAMPLE_SIMPLII_CHEQUING=… cargo test -p
financi-parsers --test real_samples -- --ignored --nocapture`

## Rules

- **Tests and synthetic data with every change.** Unit tests for logic,
  fixture files for parsers, UI tests in the style of
  `crates/financi/src/ui/tests/`. Never commit real bank data (CSV files
  are git-ignored except under `**/fixtures/**`).
- **Commit after each logical piece** (usually one task), Conventional
  Commits: `feat(scope): …`, `fix: …`, `test: …`, `docs: …`, `refactor: …`,
  `chore: …`. Keep every commit building with tests passing. Never push.
- **Verify each commit in a clean checkout** with its own target dir
  (sharing the main `target/` reuses test binaries with the worktree's
  `CARGO_MANIFEST_DIR` baked in and breaks fixture tests later). Use the
  git-ignored `.cache/` of the **main checkout**, not `/tmp` (a small tmpfs
  here; a fresh target is ~5 GB) and not the feature worktree's own
  `.cache/` (a new worktree means a cold build every time). The checkout
  path and target dir are fixed per pool slot (`AGENTS.md`; use slot 1
  when working alone), so every verify builds incrementally and the baked
  paths always match:

  ```sh
  MAIN=$(git worktree list | head -1 | cut -d' ' -f1)  # main checkout
  V=$MAIN/.cache/slot-1-verify   # slot-<N>-verify; reviewers slot-<N>-review
  git worktree add --detach "$V" HEAD
  (cd "$V" && CARGO_TARGET_DIR="$V-target" \
     cargo test --workspace --features financi-db/dev-seed)
  git worktree remove --force "$V"
  ```

- Migrations: `0001_init.sql` is frozen. Add new numbered files.
- Money is integer cents; negative = money out. Row hashes use raw exported
  strings; don't normalize before hashing.
- All DB access from the UI goes through `DbWorker`.
- GPUI practices and pitfalls are in `docs/tasks/gpui.md`; read it before UI work.
- Layout, spacing, colors and formats for every view are in
  `docs/ui-conventions.md`; follow it for any UI change.

# financi

Personal finance desktop app: imports bank CSV exports (Amex, Simplii) into
SQLite and shows spending in a GPUI window. Single user, offline. macOS and
Linux.

**Read first:** `plan.md` (requirements and decisions) and `docs/tasks.md`
(the task checklist; it wins over plan.md on M2–M5 detail). Work one
milestone at a time and stop after each. Ask before deviating from a task.

**Agent workflow:** `AGENTS.md` defines the roles (one orchestrator, one
implementer sub-agent, one reviewer sub-agent), the 130k-token context
budget, and the handoff templates. Follow it.

**UI docs are for UI work only.** Read `docs/ui-conventions.md` and the
*GPUI pitfalls* in `docs/tasks.md` only when the task changes the GPUI app
(`crates/financi/src/ui*.rs` or other view code). Skip them for backend
work (parsers, `financi-core`, `financi-db`, migrations, `flow.rs`
helpers); they cost context and don't apply.

## Commands

Toolchain is pinned in `mise.toml` (`mise install`; tasks: `mise run
build|test|lint|fmt|run`). Run `git config core.hooksPath .githooks` once
per clone so commits on `main` get their semver tag.

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
  `crates/financi/src/ui_tests.rs`. Never commit real bank data (CSV files
  are git-ignored except under `**/fixtures/**`).
- **Commit after each logical piece** (usually one task), Conventional
  Commits: `feat(scope): …`, `fix: …`, `test: …`, `docs: …`, `refactor: …`,
  `chore: …`. Keep every commit building with tests passing. Never push.
- **Verify each commit in a clean checkout** with its own target dir
  (sharing the main `target/` reuses test binaries with the worktree's
  `CARGO_MANIFEST_DIR` baked in and breaks fixture tests later). Use the
  git-ignored `.cache/` in the repo, not `/tmp` (a small tmpfs here; a
  fresh target is ~5 GB). Keep the target dir between runs so later
  verifies build incrementally:

  ```sh
  git worktree add --detach .cache/verify HEAD
  (cd .cache/verify && CARGO_TARGET_DIR=../verify-target \
     cargo test --workspace --features financi-db/dev-seed)
  git worktree remove --force .cache/verify
  ```

- Migrations: `0001_init.sql` is frozen. Add new numbered files.
- Money is integer cents; negative = money out. Row hashes use raw exported
  strings; don't normalize before hashing.
- All DB access from the UI goes through `DbWorker`.
- GPUI pitfalls are listed in `docs/tasks.md`; read them before UI work.
- Layout, spacing, colors and formats for every view are in
  `docs/ui-conventions.md`; follow it for any UI change.

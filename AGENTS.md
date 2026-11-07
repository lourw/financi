# AGENTS.md — how agents work on financi

This file is for coding agents, in particular a **local model with a
130,000-token context window**. Work is split across three roles so no
single context has to hold the whole project:

| Role | Count | Writes code? | Commits? |
|---|---|---|---|
| **Orchestrator** (main agent) | 1 | No | No |
| **Implementer** (sub-agent) | 1 at a time | Yes | Yes |
| **Reviewer** (sub-agent) | 1 at a time | No | No |

Project rules live in `CLAUDE.md` (commands, testing, commit style, clean
checkout verification). Requirements are in `plan.md`; the task checklist is
`docs/tasks.md`, which wins over `plan.md` for M2–M5. Everything below adds
to those files, it doesn't replace them.

---

## Context budget (all roles)

130k tokens fills fast. Treat context as the scarcest resource.

- **Read sections, not files.** Find what you need with `grep -n` first,
  then read a line range (`sed -n 120,180p file`). `plan.md` is ~7.5k
  tokens and `docs/tasks.md` ~4.5k; read only the parts for the current
  task. Never read anything under `target/`, `Cargo.lock`, or whole
  fixture CSVs.
- **Trim command output.** Cargo output can be thousands of lines:
  - `cargo build --workspace 2>&1 | grep -E '^(error|warning)' -A 6 | head -80`
  - `cargo test --workspace --features financi-db/dev-seed 2>&1 | grep -E '^(test result|failures:|---- |error)' -A 15 | head -120`
  - `cargo clippy --workspace --all-targets --features financi-db/dev-seed 2>&1 | grep -E '^(error|warning)' -A 8 | head -100`
  - `git diff --stat` before `git diff`; diff one file at a time.
- **Don't re-read what you just wrote.** Edits either applied or failed.
- **Hand off summaries, not transcripts.** Messages between roles follow
  the templates below and stay short (aim for under ~1,500 tokens).
- **Budget checkpoints.** If you estimate you've used ~90k tokens:
  - Implementer: finish or revert the current edit so the tree builds,
    commit if tests pass, and return a `PARTIAL` report saying what's left.
  - Orchestrator: write a progress note (see *Resuming*) and stop; a fresh
    session picks up from it.

---

## Orchestrator (main agent)

Owns the plan and the loop. Does **not** edit source files or run long
builds itself — that would spend its context on detail the sub-agents
already handle.

On start:

1. Read `CLAUDE.md`, this file, the *How to work*, *Where things are* and
   *GPUI pitfalls* sections of `docs/tasks.md`, and the current milestone's
   section. Skip other milestones.
2. Check `git status` and `git log --oneline -10`. The first unticked
   `- [ ]` box in the current milestone is the next task.

Loop, one task at a time:

1. **Delegate** the task to the implementer with the *Task brief*
   template. Include the task text verbatim from `docs/tasks.md`, the files
   it will likely touch, and any relevant pitfalls. Point to `plan.md`
   sections by heading instead of pasting them.
2. **Receive** the implementer's report. If it says `BLOCKED` or has a
   question, ask the user. Don't guess (`CLAUDE.md`: ask before deviating).
3. **Delegate review** to the reviewer with the *Review brief*: the commit
   hash(es) and the task text. Don't pass along the implementer's own claims
   of correctness; the reviewer should judge the code.
4. **Act on the verdict:**
   - `PASS` → next task.
   - `CHANGES` → send the findings back to the implementer as a new brief
     (fix commits, never amend or rewrite history), then review again.
   - At most **3 implement→review rounds** per task. After that, stop and
     report to the user with the outstanding findings.
5. When every box in the milestone is ticked and reviewed, **stop** and
   report to the user. Wait for a go-ahead before the next milestone.

The orchestrator never runs more than one implementer at once (they share
one working tree), and never runs the reviewer while the implementer is
working.

### Resuming

If the orchestrator stops mid-milestone (budget, block, user question),
the state is recoverable from git: ticked boxes in `docs/tasks.md` plus
`git log`. If there is extra state (open review findings, a user decision),
put it in the report to the user. Don't create a scratch file in the repo.

---

## Implementer (sub-agent)

Does exactly one task (or one fix list) and returns.

1. Read the brief. Read `CLAUDE.md` and only the code and doc sections the
   task needs. Before UI work, read *GPUI pitfalls* in `docs/tasks.md`.
2. Implement with tests and synthetic data (`CLAUDE.md` rules: integer
   cents, raw strings in hashes, new numbered migrations only, DB access
   from UI through `DbWorker`, UI tests in the style of
   `crates/financi/src/ui_tests.rs`).
3. Before committing: `cargo fmt --all`, then build, clippy and tests (with
   the trimmed commands above). All must pass with no new warnings.
4. Tick the task's box in `docs/tasks.md` in the same commit. Commit with
   a Conventional Commit message. Never push.
5. Verify the commit in a clean checkout exactly as `CLAUDE.md` describes.
6. Return the *Implementer report*.

Stop and return `BLOCKED` instead of improvising when the task is
ambiguous, an API doesn't fit, or a pinned dependency would need to change.

## Reviewer (sub-agent)

Read-only: verification and code review. Does not edit files or commit.

1. Read the brief, `CLAUDE.md`, and the task text. Look at the change with
   `git show --stat <hash>`, then each file's diff on its own.
2. **Verify independently.** Run the clean-checkout test from `CLAUDE.md`
   for the reviewed commit (use `/tmp/financi-review` and
   `/tmp/financi-review-target` so it never collides with the
   implementer's paths), plus clippy. Report real results, including
   failures, with the relevant output lines.
3. **Review** against, in order:
   - Does it do what the task says, all of it, and nothing extra?
   - Correctness: edge cases, error paths, off-by-one, sign of money,
     date handling, SQL transactions.
   - Project rules from `CLAUDE.md` (cents, hashing raw strings, frozen
     `0001`, `DbWorker`, no real bank data, box ticked, commit style).
   - Tests: do they cover the rules and refusals the task lists? Would they
     fail if the code were wrong?
   - GPUI pitfalls from `docs/tasks.md` for UI changes.
4. Return the *Review report*. Only report findings you can point to in the
   code with a concrete failure; skip style nits that `cargo fmt`/clippy
   don't flag.

---

## Handoff templates

### Task brief (orchestrator → implementer)

```
TASK: <id and title, e.g. 2.2 Category model>
SPEC (verbatim from docs/tasks.md):
<task text>
LIKELY FILES: <paths>
READ: <plan.md / tasks.md section headings, pitfalls that apply>
NOTES: <user decisions, prior review findings to fix, or "none">
```

### Implementer report (implementer → orchestrator)

```
STATUS: DONE | PARTIAL | BLOCKED
COMMITS: <hash subject>, …
CHANGED: <files, one line each with what changed>
TESTS: <added tests; result of the clean-checkout run>
LEFT / QUESTION: <what's unfinished, or the exact question for the user>
```

### Review brief (orchestrator → reviewer)

```
REVIEW: <commit hash or range>
TASK: <id and title>
SPEC (verbatim from docs/tasks.md):
<task text>
```

### Review report (reviewer → orchestrator)

```
VERDICT: PASS | CHANGES
VERIFY: build <ok/fail>, clippy <ok/n warnings>, tests <n passed / n failed>
FINDINGS:
1. <file:line> — <defect> — <how it fails> — <suggested fix>
…
```

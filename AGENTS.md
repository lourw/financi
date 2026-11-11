# AGENTS.md — how agents work on financi

This file is for coding agents, in particular a **local model with a
130,000-token context window**. Work is split across three roles so no
single context has to hold the whole project:

| Role | Count | Writes code? | Commits? |
|---|---|---|---|
| **Orchestrator** (main agent) | 1 | Only to resolve merge conflicts | Only the merge to `main` |
| **Implementer** (sub-agent) | Shared pool of 2 | Yes | Yes |
| **Reviewer** (sub-agent) | Shared pool of 2 | No | No |

The model server has three slots: one for the orchestrator and two for the
sub-agent pool. Never run more than two sub-agents at once, counting
implementers and reviewers together: two implementers, two reviewers, or one
of each. Every agent has the same 130k-token context cap.

Project rules live in `CLAUDE.md` (commands, testing, commit style, clean
checkout verification). Requirements are in `plan.md`; the task checklist is
`docs/tasks.md`, which wins over `plan.md` for M2–M5. The UI design
(layout, spacing, colors, formats, confirmed decisions) is
`docs/ui-conventions.md`; every UI change follows it. Everything below adds
to those files, it doesn't replace them, with one exception: the
orchestrator pushes `main` as described in *Feature worktrees*, despite
`CLAUDE.md`'s "Never push". Implementers and reviewers still never push.

---

## Feature worktrees

Every feature (one task from `docs/tasks.md`, or one piece of work the user
asks for) gets its own branch and worktree. Never commit feature work
directly on `main` or in the main checkout. The orchestrator runs these
steps from the main checkout.

**Before starting**, bring `main` up to date, then create the worktree from
it:

```sh
git switch main                # in the main checkout
git pull --ff-only --tags origin main
git worktree add ../worktrees/financi/<slug> -b feat/<slug> main
```

`<slug>` is short and kebab-case, e.g. `2-3-category-model`. If the pull
can't fast-forward, stop and ask the user. Uncommitted changes in the main
checkout aren't yours: leave them alone, and stop and ask if they block a
step.

**While working**, the implementer works and commits only in that worktree.
Review fix commits go on the same branch. The worktree starts with no build
output, so its first build is a full one.

**When the reviewer passes it**, merge locally and push. `git merge` doesn't
run the `post-commit` hook, so run the hook by hand to tag the merge. It
picks the version from the merge commit's subject, so that subject must be a
Conventional Commit describing the feature:

```sh
git pull --ff-only --tags origin main      # in the main checkout
git merge --no-ff -m "feat(scope): <what the feature does>" feat/<slug>
.githooks/post-commit
git push --follow-tags origin main
git worktree remove --force ../worktrees/financi/<slug>
git branch -d feat/<slug>
```

Use `feat`, `fix`, etc. to match the change, since that sets the version
bump. If the merge conflicts, the orchestrator resolves it (see *Orchestrator*).
If the push is rejected, stop and ask the user.
Don't force-push. Remove the worktree only after the push succeeds.

---

## Context budget (all roles)

130k tokens fills fast. Treat context as the scarcest resource.

- **Read sections, not files.** Find what you need with `grep -n` first,
  then read a line range (`sed -n 120,180p file`). `plan.md` is ~7.5k
  tokens, `docs/tasks.md` ~4.5k and `docs/ui-conventions.md` ~3.5k; read
  only the parts for the current task. Never read anything under `target/`, `Cargo.lock`, or whole
  fixture CSVs.
- **UI docs only for UI tasks.** `docs/ui-conventions.md` and the *GPUI
  pitfalls* in `docs/tasks.md` are read only when a task touches GPUI view
  code. Backend tasks (parsers, `financi-core`, `financi-db`, migrations)
  skip them, and their briefs don't name them.
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
already handle. The one exception is resolving merge conflicts between
sub-agents' branches.

On start:

1. Read `CLAUDE.md`, this file, the *How to work*, *Where things are* and
   *GPUI pitfalls* sections of `docs/tasks.md`, and the current milestone's
   section. Skip other milestones.
2. Check `git status` and `git log --oneline -10`. The first unticked
   `- [ ]` box in the current milestone is the next task.

### Keep the pool full

The orchestrator doesn't work one task at a time. It runs a pool of two
slots and refills a slot as soon as it frees. Each feature goes through the
steps below, but two features move through them at the same time, each at
its own pace.

- **Never block on a sub-agent.** Start every sub-agent with
  `background: true` on the subagent/task tool call. The tool blocks by
  default: without that flag the orchestrator's turn stays inside the call
  until that agent returns, so it can't start or answer anything else, and
  only one slot is busy. With it, the call returns at once and the report
  arrives later as a notification.
- **Fill both slots in one turn.** At the start, and whenever both slots
  are free, choose two parallel-safe tasks and start both sub-agents in the
  same message. Don't start one and then wait for its report before
  starting the second.
- **Refill as soon as a slot frees.** When a sub-agent's report arrives,
  act on it right away and start the next sub-agent in the same turn: the
  review for that feature, a fix round, or the next parallel-safe task.
  Then go back to waiting. Don't wait for the other slot's agent first.
- **Wait for notifications, don't poll.** With both slots busy and nothing
  to merge, end the turn and wait for the next completion notice. Don't
  check on a running agent's progress in a loop or start a blocking wait
  on one agent.
- **If a background call fails** (the option is rejected or unavailable),
  start both sub-agents in a single message (two blocking calls in the same
  reply) so they still run together. That turn returns only when both
  finish, so pair tasks of similar size where you can.

A slot may sit empty only while the orchestrator is merging, waiting on the
user, or when no remaining task in the milestone can run alongside the one
in flight (see below).

Steps for each feature:

1. **Delegate** the task to the implementer with the *Task brief*
   template, after syncing `main` and creating the task's worktree (see
   *Feature worktrees*). Include the task text verbatim from `docs/tasks.md`, the files
   it will likely touch, and any relevant pitfalls. Point to `plan.md`
   and `docs/ui-conventions.md` sections by heading instead of pasting
   them; every UI task's brief names the `docs/ui-conventions.md` sections
   that apply.
2. **Receive** the implementer's report. If it says `BLOCKED` or has a
   question, ask the user. Don't guess (`CLAUDE.md`: ask before deviating).
3. **Delegate review** to the reviewer with the *Review brief*: the commit
   hash(es) and the task text. Don't pass along the implementer's own claims
   of correctness; the reviewer should judge the code.
4. **Act on the verdict:**
   - `PASS` → merge to `main`, push and remove the worktree (see *Feature
     worktrees*), then next task.
   - `CHANGES` → send the findings back to the implementer as a new brief
     (fix commits, never amend or rewrite history), then review again.
   - At most **3 implement→review rounds** per task. After that, stop and
     report to the user with the outstanding findings.
5. When every box in the milestone is ticked and reviewed, move on to the
   next milestone without waiting for a go-ahead. Stop and report to the
   user when v1 is complete (M5 done).

The orchestrator runs at most two sub-agents at once, so at most two
features are in flight. Each feature has its own branch and worktree, and a
sub-agent works only in the worktree its brief names. Never run an
implementer and a reviewer on the same feature at once.

Before delegating, choose tasks that can safely run in parallel, so the two
branches don't conflict at merge. Two features may run together only if
neither depends on the other and they don't change the same functions or
regions. Sharing a file is fine: most UI work lands in
`crates/financi/src/ui.rs` and `ui_tests.rs`, and git merges separate parts
of a file cleanly. Name the regions in each brief, and have implementers add
new tests next to related tests, not at the end of the file. If you can't
tell which regions a task will change, don't run it alongside another.

If the next task can't run alongside the one in flight, don't wait for that
feature to merge. Look further down the current milestone's unticked tasks
and start the first one that can run alongside it. Skip any task that
depends on unfinished work or that its text says to do last. Only leave the
pool slot empty if no remaining task in the milestone qualifies. Tasks that
were skipped go next, each from a worktree created from the updated `main`.

Merge finished features one at a time. If a merge conflicts, the
orchestrator resolves it itself instead of sending it back to a sub-agent:

1. Union the changes, keeping the behaviour and tests from both sub-agents'
   implementations. Edit only what the conflict requires.
2. Finish the merge with `git commit --no-edit`, which keeps the Conventional
   Commit subject and tags the merge through the `post-commit` hook.
3. Verify the merge commit in a clean checkout, as `CLAUDE.md` describes,
   before pushing.

If the two changes contradict each other, so both can't be kept, stop and ask
the user which should win.

### Resuming

If the orchestrator stops mid-milestone (budget, block, user question),
the state is recoverable from git: ticked boxes in `docs/tasks.md`,
`git log`, and `git worktree list` for unmerged features, which are
continued in their existing worktrees. If there is extra state (open review findings, a user decision),
put it in the report to the user. Don't create a scratch file in the repo.

---

## Implementer (sub-agent)

Does exactly one task (or one fix list) and returns.

1. Read the brief and `cd` to the worktree it names; all reads, edits,
   builds and commits happen there. `cd` doesn't move file tools, so give
   them `<WORKTREE>/<path>` for every repo path (`CLAUDE.md`, `plan.md`,
   `docs/…`, code). Read `CLAUDE.md` and only the code and doc sections the
   task needs. Before UI work, read *GPUI pitfalls* in `docs/tasks.md`
   and the relevant sections of `docs/ui-conventions.md` (always
   *Decisions*, plus whatever the brief names).
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

1. Read the brief, `cd` to the worktree it names, and read `CLAUDE.md` and
   the task text from there, giving file tools `<WORKTREE>/<path>` for every
   repo path. Look at the change with
   `git show --stat <hash>`, then each file's diff on its own.
2. **Verify independently.** Run the clean-checkout test from `CLAUDE.md`
   for the reviewed commit (use `.cache/review` and
   `.cache/review-target` so it never collides with the
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
   - UI changes match `docs/ui-conventions.md` (layout, spacing, theme
     tokens instead of raw colors, number/date formats, decisions).
4. Return the *Review report*. Only report findings you can point to in the
   code with a concrete failure; skip style nits that `cargo fmt`/clippy
   don't flag.

---

## Handoff templates

### Task brief (orchestrator → implementer)

```
TASK: <id and title, e.g. 2.3 Category model>
WORKTREE: <absolute path> (branch feat/<slug>); all repo paths below are relative to it
SPEC (verbatim from docs/tasks.md):
<task text>
LIKELY FILES: <paths>
REGIONS: <functions or regions this task may change; the other in-flight feature's regions, or "none in flight">
READ: <plan.md / docs/tasks.md / docs/ui-conventions.md section headings, pitfalls that apply>
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
WORKTREE: <absolute path> (branch feat/<slug>); all repo paths below are relative to it
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

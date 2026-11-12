# financi — implementation tasks

This is the working checklist for the rest of v1. It turns `plan.md` (the
requirements) into concrete tasks. Where the two disagree, **this file
wins** for M2–M5 detail.

Finished tasks are archived in `docs/tasks-done.md`. Don't read it at the
start of a session; grep it for a task number only when an open task
refers to one.

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
6. When a task is merged, move its text from this file to
   `docs/tasks-done.md` (under its milestone, in number order) in the same
   merge commit or a `docs:` commit right after. Only open tasks stay here.
7. New user requests and bug reports become numbered tasks here (next free
   number), placed in the order below.
8. **Writing tasks:** name the component and token (`DataList` with
   columns …, `InlineForm`) instead of reciting pixels; restate the one
   behaviour a task depends on instead of pointing at a done task's
   number; split a task that mixes backend, UI and performance work. When
   a task contradicts `docs/ui-conventions.md`, change the convention (in
   the same commit, reviewed for consistency) instead of adding an
   exception.

## Picking the next task

Task numbers record when a task was written, not when to do it. **The
architecture and scalability work comes first** (user decision,
2026-11-11): feature tasks wait until it has landed. Two queues run side by
side, one task from each, so both slots stay busy. Start every task's
worktree from the current `main`.

1. **Now:** finish the two features the paused run left in flight, each in
   its existing worktree: **3.34** (uncommitted work) and **3.53 + 3.42**
   (see 3.53).
2. **Then two queues, in order:**
   - **UI queue (architecture and GPUI best practices):** 3.62 → 3.74 →
     3.75 → 3.64 → 3.65 → 3.76 → 3.78 → 3.56 → 3.89 → 3.79 → 3.80 → 3.81
     → 3.82 → 3.83 → 3.91 → 3.57 → 3.84 → 3.85 → 3.86. One at a time, except that
     once 3.79 has merged, two of 3.80–3.83 may run together (they live in
     different modules).
   - **Backend queue (scale, then correctness):** 3.77 → 3.73 → 3.88 →
     3.87 → 3.66 → 3.67 → 3.68 → 3.69 → 3.70 → 3.72 → 3.71 → 3.90 → 5.1
     → 5.2. These don't touch the UI, so one always runs beside the UI
     queue (or a second one when the UI queue is waiting).
   - **Dependencies:** 3.78 needs 3.77 merged; 3.56 needs 3.77, 3.78 and
     3.88; 3.84 needs 3.64. If the next UI task is blocked, the slot takes
     the next backend task.
3. **Then the feature lanes**, one task from each of two lanes at a time:
   - **Lane A, dashboard:** 3.36 → 3.45 → 3.46.
   - **Lane B, Categories and Rules:** 3.58 → 3.48 → 3.50 + 3.51 (one
     branch) → 3.49 → 3.43 → 3.60 → 3.52 → 3.47 + 3.54 (one branch).
   - **Lane C, small tasks:** 3.63 → 3.37 → 3.61 → 3.59 (after 3.51 and
     3.58).
   - Backend tasks left over from step 2 fill any idle slot.
4. **Last in M3: 3.26** (scrollbars), once the lanes are empty.
5. **Then M5** (5.3, 5.4 and whatever of 5.1 / 5.2 is left), **then the
   Final task** (3.55, performance at scale).

## GPUI best practices: the target, step by step

The UI grew as one `Shell` entity (~200 fields, every page's state) that
re-queries every page after every action. Zed-style GPUI is the target
(user decision, 2026-11-11). Each step below is the GPUI rule, what
changes, and the task that does it, in implementation order. When a step's
task merges, the rule becomes binding (see *Coding practices*).

1. **Small files, one per page** (3.62). `ui.rs` / `ui_tests.rs` become
   `ui/<page>.rs` modules and `ui/tests/<page>.rs`. Pure move.
2. **State belongs to the page that owns it** (3.74). Page fields move
   into per-page state structs; `Shell::new` shrinks to wiring.
3. **Tests drive the real UI and wait for real idleness** (3.75). A
   `click` helper through rendered controls, `wait_idle` before negative
   assertions, one bounds helper, `page:thing[:id]` selectors.
4. **A `Task` is owned by its purpose** (3.64). Dropping a `Task` cancels
   it, so never share one slot between unrelated actions: mutations
   `.detach()` and update through the weak entity; only work that a newer
   request should replace keeps an `Option<Task>` slot.
5. **State changes notify at once** (3.65). Every handler that changes
   what renders calls `cx.notify()` right away; only data waits for the
   DB. Controlled widgets (`Checkbox`, `Select`) depend on it.
6. **Global wiring once; no strong cycles** (3.76). Key bindings, menus
   and global actions are registered once in `ui::init(cx)`, not in a view
   constructor. Callbacks capture `WeakEntity`; child views talk to
   parents through `EventEmitter` events, not stored strong handles.
7. **Shared data is a model entity that emits change events** (3.77 +
   3.78). A `Catalog` entity owns accounts, categories, rules and their
   derived lists; mutations report the `Domain`s they changed; readers
   subscribe to `DataChanged`. DB reads run as keyed latest-wins jobs on a
   read connection, so stale work is skipped and writes don't block reads.
8. **Each view loads its own data** (3.56): the dashboard's one
   `DashboardFilter` value and its own keyed loader, debounced search.
9. **Shared components are `RenderOnce`s** (3.89): `Panel`,
   `PageHeader`, `DataList`, `FilterRow`, `InlineForm` and small pieces in
   `ui/widgets.rs`. Pages compose them instead of styling by hand.
10. **One entity per page** (3.79–3.83): `TransfersPage`, `ImportsPage`,
    `RulesPage`, `CategoriesPage`, `DashboardPage`, `QueuePage`. Each owns
    its state, renders itself, declares `const DEPENDS: Domains`, reloads
    on matching `DataChanged` (or when shown, if hidden), and has a
    `FocusHandle` + `key_context` with a `Cancel` action for Escape (no
    global key handlers). `Shell` keeps only navigation, the sidebar and the toast layer.
11. **Every page is built from the components** (3.91): To categorize,
    Categories (header, New category form, tree) and the Dashboard (filter
    row, panels) move onto 3.89's components; Rules (3.58) and the
    Categories rule list (3.49) are restyled on them in the feature lanes.
12. **Dependencies declared, not chosen per click** (3.57): one table of
    mutations → domains and pages → domains, tested.
13. **Every click answers within a frame** (3.84): optimistic update with
    rollback, or `.loading(true)` + disabled until done.
14. **Render reads caches** (3.85): no work in `render` that grows with
    the data; derived view models are built when inputs change. Toasts
    are their own entity so their ticks don't re-render pages.
15. **Layout from measured bounds** (3.86): `canvas` / `on_prepaint`
    bounds and `window.text_system()` measurements, not window width minus
    constants.

## Where things are

| Path | What |
|---|---|
| `crates/financi-core` | `raw` (`RawFile`/`RawRecord`, `read_raw_file`), `parse` (cents, dates, merchant), `dates`, `rules` (`CompiledRules`, `pick_category`, `winning_rule`), `StatementParser` trait, `row_hash` |
| `crates/financi-parsers` | `amex`, `simplii` (old id `simplii-chequing` still resolves); `all()`, `detect()`, `by_id()` |
| `crates/financi-db` | `migrations/0001`–`0004` (0001 frozen), `accounts`, `import`, `master_csv`, `categories`, `rules`, `overrides`, `queue`, `views`, `transfers`, `settings`, `worker::DbWorker`, `seed` (feature `dev-seed`) |
| `crates/financi/src/flow.rs` | GPUI-free helpers (formatting, import flow), unit-tested |
| `crates/financi/src/ui.rs` | every view: `Shell` and all pages (per-page `ui/` modules after 3.62; `ui/widgets.rs` components after 3.89) |
| `crates/financi/src/ui_tests.rs` | headless UI tests (`ui/tests/` after 3.62) |
| `crates/financi/src/version.rs`, `build.rs` | build version from `git describe` (3.39) |
| `assets/icon/` | app icon source (3.63) |
| `fixtures/demo/` | synthetic exports + `expected.json` reference totals |
| `spikes/gpui-spike` | sandbox for UI experiments; keep, don't ship |
| `docs/ui-conventions.md` | design system: tokens, components, patterns, pages; read before any UI task |

## Coding practices (all UI tasks)

Follow these without being told. Rules marked *(from 3.xx)* apply once
that task has merged; until then, don't make the old pattern worse.

- **State lives on the page that owns it** *(from 3.74; page entities from
  3.79)*; shared data lives on `Catalog` *(from 3.78)*. Don't add fields to
  `Shell` for one page's needs.
- **Handles and widget state are created once,** in the owner's `new`,
  never in `render` (a scroll handle made in `render` resets every frame).
- **A `Task` belongs to one purpose.** Mutations `.detach()` and update
  through the weak entity; only work a newer request should replace keeps
  an `Option<Task>` slot. Never reuse a slot across actions *(3.64)*.
- **Notify on every state change** that affects rendering, at once, not
  after the DB returns *(3.65)*.
- **Writers report domains, readers declare them** *(from 3.78)*: a
  mutation reports the `Domain`s it changed; a page declares `DEPENDS`.
  Never call a full reload for a local action.
- **Reads that a newer request replaces are keyed jobs** on the reader
  connection (`call_keyed`, `call_read`) *(from 3.77)*.
- **Feedback within a frame:** a click updates the screen optimistically
  (with rollback on error) or shows `.loading(true)` and disables the
  control until the DB returns.
- **No data-sized work in `render`.** Build derived lists, labels,
  compiled rules and formatted strings when their inputs change; render
  reads the cache.
- **Keyboard through actions and key contexts:** each page root has a
  `FocusHandle`, `track_focus` and a `key_context`; Escape is the page's
  `Cancel` action. No global key handlers in views *(from 3.79)*.
- **No strong cycles:** callbacks capture `WeakEntity`; child views talk to
  parents through `EventEmitter` events *(3.76)*.
- **Compose the shared components** (`Panel`, `PageHeader`, `DataList`,
  `FilterRow`, `InlineForm`, …) instead of styling rows and panels by hand
  *(from 3.89)*. Column widths come from the component's typed columns.
- **Text never pushes its container wider:** `min_w_0()` plus ellipsis and a
  full-text tooltip.
- **Layout from measured bounds,** not window width minus constants
  *(from 3.86)*.
- **Tests:** click through rendered controls with the `click` helper and
  wait with `wait_idle` before asserting nothing happened *(from 3.75)*;
  selectors are `page:thing[:id]`; put new tests next to related ones; when
  a layout changes, update old tests, don't delete them.
- **Pure logic goes in pure functions** (`flow.rs`, or a free fn next to the
  view) so it can be unit-tested without a window.
- **Format through the helpers** (`flow::money`, `flow::count`, ISO dates)
  and color through theme tokens only.
- **Dependencies:** don't bump pinned versions. A crate already in
  `Cargo.lock` may be added as a direct dependency at its locked version
  (say so in the handoff); anything new, ask first.

## GPUI / gpui-kit pitfalls (already hit — don't rediscover)

- Pinned: `gpui-kit =0.9.0` on `gpui-pre =0.3.11`, with
  `gpui-pre-platform` feature `runtime_shaders` (no full Xcode needed). Don't
  bump versions without asking.
- Do **not** enable `gpui-kit/test-support`: a Cargo weak-dependency bug pulls
  in `gpui-fast` and the lockfile fails to resolve. UI tests use GPUI's own
  `TestAppContext` (`gpui` dev-dependency with `test-support`).
- In test modules, never `use gpui_kit::*;` — it imports GPUI's `test` macro,
  which shadows `#[test]` and makes `#[gpui::test]` recurse. Import names
  explicitly (see `ui_tests.rs`).
- UI tests talk to a real `DbWorker` thread: call
  `cx.executor().allow_parking()` and poll with the `wait_until` helper.
- `BarChart` has no click handler. `SelectState::set_items` and
  `set_selected_index` need a `&mut Window`, so async updates use
  `cx.spawn_in(window, …)` + `this.update_in(cx, |this, window, cx| …)`.
- All DB work goes through `DbWorker` via the `on_db` helper in `ui.rs`.
- gpui-base's `virtual_list()` makes a new scroll handle on every call;
  pass the list's own `VirtualListScrollHandle` from `Shell` with
  `.track_scroll(&handle)` or it won't scroll (3.21).
- A scroll container (`overflow_y_scroll`) gives its children no height to
  fill: `flex_1` children collapse. Give every panel inside one an
  explicit height (3.45).
- Screenshots (macOS): `FINANCI_DB=/tmp/x.sqlite FINANCI_DEV_SEED=1
  FINANCI_DEV_OPEN=fixtures/demo/amex-sep-nov.csv cargo run -p financi`,
  then capture the window by id (`screencapture -o -l<id>`; find the id with
  CoreGraphics `CGWindowListCopyWindowInfo`). Window capture may need the
  user to grant Screen Recording permission. On this Linux machine no
  screenshot tool is installed: rely on the UI tests' bounds checks, and
  ask the user for a screenshot if a task needs one.

---

## M3 — Core views (one dashboard)

Layout, top to bottom: filter row; then the category chart and the monthly
summary; then the transaction table. Done M3 tasks are in
`docs/tasks-done.md`. Sections below follow *Picking the next task*.

### Now: finish the in-flight features (3.34, 3.53 + 3.42)

- [ ] **3.34 To categorize looks cleaner.** The queue is loose rows on the
  page background: no panel, no row separators, a semibold merchant, and a
  bright primary Categorize button on every row, so the page is a column
  of blue buttons. Keep the columns, widths and behavior of 3.22; change
  the look:
  - **Header:** "To categorize" and a muted summary "N transactions · M
    merchants" (`flow::count`).
  - **One bordered panel** (D7, `radius`, no fill) that fills the rest of
    the page and holds the list. Column headings are a 32 px heading row
    inside the panel (`text_sm` muted, `px_3`), with a bottom `border`.
    Rows are 40 px, `px_3`, each with a bottom `border` divider (none
    after the last), `list_hover` on hover. Headings keep sharing the row
    widths.
  - **Merchant** in normal weight `foreground`; Transactions and Total
    right-aligned; Dates `text_sm` muted (as now).
  - **Categorize button:** `.small()` and not primary (outline/ghost
    style) and **disabled until the row's picker has a category**, then enabled
    (still outline: one primary per page, `ui-conventions.md` →
    *Patterns*), so only rows ready to apply are clickable. The heading row's invisible spacer matches the new button
    size.
  - Empty state unchanged, centered inside the panel.
  - Still fits 1200 × 720 with no horizontal overflow (3.22 tests stay).
  - Tests: the panel paints inside the page; a row's Categorize is disabled with no pick and enabled (outline) after
    picking; the summary
    text shows both counts; 3.22's heading-alignment and fit tests pass.
    Update the *To categorize* row of `ui-conventions.md`.
  - **In progress:** worktree `../worktrees/financi/3-34-to-categorize`
    (branch `feat/3-34-to-categorize`) has ~170 uncommitted lines in
    `ui.rs` from the paused run (panel, heading row, Categorize button
    mirroring its picker). Continue from them; don't start over.

- [ ] **3.53 No category picker where the page already says it.** The
  Categories page's add-rule row and rule editor each carry a Category
  picker, but the rule list sits inside a selected category, so the
  picker mostly repeats the selection (user request, 2026-11-11).
  Replaces 3.41.
  - **In progress, with 3.42.** Branch `feat/3-41-42-rule-rows`
    (worktree `../worktrees/financi/3-41-42-rule-rows`) holds 3.42's
    commits 7d879e6 and 31dec49, plus cdeff74, a 3.41 commit (full
    category names in the rule pickers). User decision 2026-11-11: keep
    the branch for 3.53 + 3.42. First `git revert cdeff74` (keep any
    menu-width code the parent-scoped picker below can reuse), then
    implement 3.53, then review 3.42 and 3.53 together.
  - **Subcategory selected:** the add-rule row has no picker. `add_rule`
    targets the selected subcategory. The row is Pattern, regex switch,
    Add rule.
  - **Parent selected:** rules must target a child (`ensure_assignable`),
    so the add-rule row keeps a picker. It lists **only that parent's
    subcategories by their own names** (no "Parent › Child"), placeholder
    "Subcategory…", no default pick. The trigger is wide enough for the
    longest child name up to 200 px; names past that ellipsize with a
    tooltip, and the open menu is at least as wide as the longest name
    (`menu_width`, max 320 px), so no item is cut off. A parent with no
    subcategories shows "Add a subcategory first" (muted) in place of the
    add-rule row.
  - **Rule editor:** no picker on either. Edit changes pattern and regex
    only; `update_rule` keeps the rule's `category_id`. Moving a rule to
    another category is the Rules page's job (3.28's Move bar). Add a
    muted one-line hint under the rule list on this page: "To move rules
    to another category, use the Rules page."
  - Drop the now-unused `rule_edit_target`. Keep `child_choices` for the
    places that still list every child (transactions, queue, Rules page).
  - Fits 1200 × 720 and 1600 × 900 with a long subcategory name; Add
    rule / Save / Cancel paint inside the detail panel (3.23's fit rule).
  - Tests:
    - on a subcategory, no picker paints and Add rule creates a rule
      targeting it;
    - on a parent, the picker lists only its children by short name, and
      Add rule without a pick shows "Choose the subcategory the rule
      assigns.";
    - with a 40-character child name, the menu item paints unclipped;
    - a parent with no children shows the hint;
    - Edit has no picker and Save keeps the category;
    - 3.24's add/edit tests are updated, not deleted.

    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.42 The regex switch lines up with the rule fields.** In the
  add-rule row (`cat:add-rule-row`) and the rule editor row, the
  **regex** `Switch` sits lower than the Pattern input, the Category
  picker and the buttons (user report, 2026-11-11). Cause: both rows are
  `h_flex().items_end()`, the labelled fields are a label line over a
  32 px control, and the Switch (no label line, shorter than 32 px) is a
  bare child, so its bottom edge, not its centre, lines up with the
  controls.
  - Wrap the Switch in a box the height of a control: `h_flex().h(px(32.))
    .items_center().flex_shrink_0()`, in both rows, so its centre matches
    the controls' centre while the row stays `items_end`. Better: factor
    a small `control_slot(el)` helper used by both rows, so they can't
    drift apart.
  - Done on 3.53's branch; see 3.53.
  - Tests: in both rows the Switch's vertical centre equals the Pattern
    input's centre within 1 px, and the Save / Add rule button's centre
    too. Note the rule in `ui-conventions.md` (*Forms*: an unlabelled
    control in a labelled row sits in a 32 px slot, centred).

### UI queue: architecture and GPUI best practices (in this order)

Steps 1–15 of *GPUI best practices* above, task by task.

- [ ] **3.62 Split `ui.rs` and `ui_tests.rs` by page.** Pure refactor,
  no behaviour change (planned 2026-11-11 in a docs review). `ui.rs` is
  ~7 500 lines, one `impl Shell` block of ~5 500 of them, and
  `ui_tests.rs` is ~8 500 lines. Every UI task edits both, so two UI
  branches in flight conflict at merge, and one file costs an agent most
  of its 130k context to navigate.
  - **Review update (2026-11-11):** Pure move, as written. The test-helper dedupe and selector renames are 3.75; moving key bindings, menus and global actions out of `Shell::new` is 3.76 — not part of the move.
  - `git mv crates/financi/src/ui.rs crates/financi/src/ui/mod.rs` first,
    in its own commit, so history follows the file. Then move code into
    sibling modules under `crates/financi/src/ui/`:
    - `mod.rs`: `Shell` (the struct and `new`), `Page`, `Render for
      Shell`, the page switch, `refresh` and `on_db`;
    - `widgets.rs`: shared helpers (`checkbox`, `field_label`,
      `region_title`, `filter_tip`, `category_palette`, toasts, shared
      width consts);
    - `sidebar.rs`, `dashboard.rs` (filters, chart, summary,
      `TxTable`, row menu), `queue.rs`, `categories.rs`, `rules.rs`,
      `transfers.rs`, `imports.rs` (`ImportPanel`,
      `ImportHistoryTable`).
  - Each page module adds its own `impl Shell { … }` block. Child modules
    can read the parent module's private fields, so `Shell`'s fields stay
    private. Items moved out of `mod.rs` that siblings use become
    `pub(super)`; nothing becomes `pub` that wasn't.
  - Tests: `ui_tests.rs` → `ui/tests/mod.rs` (shared helpers:
    `wait_until`, seeding, bounds helpers) plus one file per page, the
    same split as the code. Every test keeps its name. The number of
    tests listed by `cargo test --workspace --features
    financi-db/dev-seed -- --list` is equal before and after (put both
    counts in the commit message).
  - Aim for no file over ~1 500 lines. Move code; don't rewrite it.
    `git diff --stat` should show adds ≈ removes.
  - Update the docs that name the files: `CLAUDE.md` (*UI docs are for UI
    work only*: `crates/financi/src/ui/`), `AGENTS.md` (*Keep the pool
    full*: UI work lands in per-page modules), `docs/tasks.md` (*Where
    things are*), `docs/ui-conventions.md` (`ui_tests.rs` mentions). Open
    task texts that say "`ui.rs`" mean the matching module from now on;
    don't rewrite them.
  - **Scheduling:** the only UI branch in flight while it runs (the
    other slot does M5 backend work). Start it after 3.53's branch merges,
    and merge it before any other UI branch starts.

- [ ] **3.74 Group `Shell`'s state by page.** (Architecture, user decision
  2026-11-11: move to page entities.) Pure refactor, no behaviour change,
  right after 3.62. Today every page module reaches into ~200 shared
  `Shell` fields and a 450-line `Shell::new`, so two lanes still collide
  on the struct, the constructor and `refresh`.
  - Move each page's fields into a struct in its module: `DashboardState`,
    `QueueState`, `CategoriesState`, `RulesPageState`, `TransfersState`,
    `ImportsState`, each with its own `new(window, cx)`; `Shell` holds
    them as `self.dashboard`, `self.rules_page`, …. Shared data (accounts,
    categories, rules, `child_choices`, counts) stays on `Shell` until
    3.78 moves it to `Catalog`.
  - Split `Shell::new` the same way, and the refresh continuation into a
    per-page `apply_*` method.
  - Done when: every page field lives in its page struct, `Shell::new`
    is under ~100 lines, no file over ~1 500 lines, no new `pub`, and
    the test count is unchanged.

- [ ] **3.75 A reliable UI test harness.** (Code review, 2026-11-11.)
  Tests only; rename selectors in the UI code where needed.
  - **Wait for real idleness.** 11 tests wait for "initial load" with
    `refresh_task.is_some()`, which is already true when `Shell::new`
    returns, and `run_until_parked` doesn't wait for the `DbWorker`
    thread, so "nothing happened" assertions can pass vacuously. Add a
    loaded / applied-generation marker set in the refresh continuation
    and a `wait_idle(shell)` helper (no DB job in flight, all results
    applied). Use it before every negative assertion; delete the
    `refresh_task.is_some()` waits.
  - **Click through controls.** ~160 places call `Shell` methods or set
    fields directly, so wiring bugs (like 3.65's missing notify) can't
    show. Add `click(cx, window, selector)` (bounds + `simulate_click`)
    and convert at least one test per control kind (checkbox, button,
    select, legend row, summary cell) to use it.
  - **One bounds helper.** `bounds_of`, `centre_y`, `rules_sel_y` and
    `paints` are the same helper; `wheel_over` / `wheel_over_x` are near
    copies; `leaked()` works around `&'static str` selectors. Make one
    `bounds(window, impl Into<String>)` and one wheel helper in
    `ui/tests/mod.rs`.
  - **Realistic defaults.** `open_shell_at` defaults to 1100 × 720, below
    the app's 1200 × 720 minimum; use 1200 × 720. Give each test its own
    temp DB dir (today one per process is shared by parallel tests that
    write master CSVs).
  - **Selectors follow `page:thing[:id]`.** Rename the stragglers
    (`cat-tree-row-{id}`, `rule-row-{id}`, `add-rule-preview`,
    `rule-edit-error`, `transfer-review:{txn}`, `rules:group-{id}`,
    `summary:cell-{ix}-{col}`, and `rule:` vs `rules:`), and add a test
    that walks a rendered frame's selectors against the pattern.
  - Done when: no `refresh_task.is_some()` waits remain, every negative
    assertion follows `wait_idle`, and the test count is unchanged or
    higher.

- [ ] **3.64 Actions never cancel each other.** (Code review, 2026-11-11.)
  About 20 unrelated actions store their async work in one
  `Shell::task: Option<Task<()>>` (assigned in ~21 places: pick file,
  undo import, category edits, rule add/save/delete/move, queue
  Categorize, change category, hide, pair, unpair, apply rule offer).
  `ImportPanel` does the same with its own `task`. Replacing the slot drops
  the earlier `Task`, which cancels its continuation, while the queued
  `DbWorker` job still commits. So a second click loses the first action's
  follow-up: e.g. Save a rule, then hide a row before it returns → the
  rule is saved but `pending_rule_action` never clears (Save stays
  loading and disabled), the editor never closes, errors and toasts are
  lost, and nothing refreshes.
  - Mutations whose result must always be applied run as
    `cx.spawn_in(..).detach()` and update through the weak entity
    (`this.update_in(..)` already returns `Err` once the view is gone);
    their result handler checks the state it touches is still current
    (e.g. the editor is still on that rule) before changing it.
  - Only work a newer request should replace keeps an `Option<Task>`
    slot, one per purpose: the refresh, the live preview count, settings
    writes. Delete the shared `task` field.
  - **Import while busy:** disable the preview's Cancel and show
    `.loading(true)` on Import while `busy`, and ignore `open_file` (drop,
    ⌘O) while an import is committing; today Cancel drops the panel and
    its task while the commit still runs, so the rows are imported with no
    toast and no master-CSV error.
  - Store the import panel's `Subscription` with the panel
    (`import: Option<(Entity<ImportPanel>, Subscription)>`); today every
    open pushes a new one into `_subs` and never removes it.
  - Tests: start a rule Save and a row Hide in the same frame, then
    both complete (pending cleared, editor closed, row hidden, two
    refreshes); Cancel is disabled while an import commits and a second
    drop is ignored; opening ten files leaves one import subscription.

- [ ] **3.65 Filter clicks redraw at once.** (Code review, 2026-11-11.)
  The dashboard setters (`set_account_checked`, `set_include_transfers`,
  `apply_search`, `set_uncategorized_only`, `set_show_hidden`,
  `set_tx_sort`, `click_legend_row`, `show_all_categories`,
  `clear_category_selection`, `click_summary_cell`) change state and call
  `refresh`, which only spawns a task; the only `cx.notify()` is in the
  task's continuation. gpui-component's `Checkbox` is controlled ("the
  owner must write the requested value and call `cx.notify()`",
  `gpui-component-0.9.0/src/checkbox.rs:105`), so a ticked box, the
  "Options · N" label, the legend highlight and the summary cell
  highlight stay unchanged until every page's queries return. This is a
  large part of the "really slow filters" report.
  - Every setter calls `cx.notify()` right after changing state; only
    the data waits for the DB.
  - **One refresh at startup:** `Shell::new` starts `load_date_range`,
    `load_include_transfers` and `refresh` separately, so launch runs up
    to three full refreshes and the first frame shows the default range.
    Read both settings in the first refresh's DB job (or in `main.rs`
    before the window opens, like `open_db`), apply them, and run exactly
    one refresh.
  - Tests: click the real Options checkbox (`simulate_click` on its
    bounds), draw once without letting the DB job finish, and the box
    paints checked and the label reads "Options · 1"; the same for an
    account checkbox and a legend row; with a saved Custom range, launch
    issues one refresh job (count jobs with a test hook) and the first
    data frame uses the saved range.

- [ ] **3.76 App wiring done once, without reference cycles.** (Code
  review, 2026-11-11.)
  - **Global setup out of the view.** `Shell::new` calls `cx.bind_keys`,
    `cx.set_menus` and registers global `cx.on_action` listeners holding
    a strong `Entity<Shell>`; GPUI has no way to remove those listeners
    (`gpui-pre-0.3.11/src/app.rs:2758`), so a second Shell (a second
    window, or a test opening two) duplicates every handler (⌘O opens two
    pickers). Move them into `ui::init(cx)`, called once from `main.rs`;
    the handlers find the window's Shell through a `Global` holding a
    `WeakEntity<Shell>` (or the active window's root).
  - **Weak captures.** The six `TxTable` callbacks capture a strong
    `cx.entity()`, a cycle that keeps `Shell` alive forever. Capture
    `cx.weak_entity()` and `update(..).ok()`. Better: have `TxTable` emit
    events the Shell subscribes to, which also removes the re-entrancy
    risk of updating the Shell from inside the table's own update.
  - **Stale row indexes.** `TxTable::context_menu` and `render_td` index
    `self.rows[row_ix]`; use `.get()` and return the menu unchanged when
    the row is gone (the index can be stale across a refresh).
  - **macOS reopen.** The default quit mode leaves the app running with
    no window after the window closes, and there's no `on_reopen`. Add
    `on_reopen` to open the window again (or quit on last window closed;
    pick reopen, the macOS norm).
  - **Startup.** `main.rs` runs `master_csv::self_check` and the dev seed
    before the window opens; move `self_check` into the first `DbWorker`
    job so the window shows at once. A startup error (`open_db()?`) only
    prints to stderr, so on macOS the app just vanishes: also write it to
    `<data dir>/financi.log` and show it in a minimal error window.
  - Tests: two Shells register one set of handlers (⌘O fires once);
    dropping the window drops the Shell (a `WeakEntity` upgrade fails); a
    context menu on a row index past the end doesn't panic.

- [ ] **3.78 A shared data model that announces changes.** (Architecture,
  user decision 2026-11-11.) Today one `refresh` re-queries every page
  (a 13-part tuple) and ~30 call sites decide when to call it; each new
  view (v1.1–v1.3) would have to be added to every path. Invert it:
  writers say what they changed, readers say what they depend on.
  - `enum Domain { Accounts, Categories, Rules, Transactions, Imports,
    Transfers, Settings }` as a bitset type `Domains`.
  - Each mutation's UI handler reports the domains it changed, from one
    table in code (doc comment on `Domain`): rule add/edit/delete/move →
    `Rules | Transactions`; set category, hide, unhide →
    `Transactions`; category add/rename/move/delete → `Categories |
    Rules | Transactions`; pair/unpair/review → `Transfers |
    Transactions`; import/undo/restore → all.
  - A `Catalog` entity (`impl EventEmitter<DataChanged>`) owns the shared
    data: accounts, the category tree, `child_choices` and their `Names`
    (built once, shared via `Rc`/`SharedString`), rules, rule-use stats,
    sidebar counts. `Catalog::changed(domains, cx)` reloads only the
    parts tied to those domains (keyed jobs, 3.77), then emits
    `DataChanged(domains)`.
  - Pages still on `Shell` react through one mapping `Shell` keeps until
    they're extracted (3.79–3.83): each page section declares the
    domains it depends on and reloads only on a matching event. Delete
    the full `refresh` except for import/undo/restore.
  - Tests: a rule edit reloads rules, rule stats and the dashboard but
    not imports history or transfer reviews (query-count hook); an import
    reloads everything; a page switch issues no query.

- [ ] **3.56 Dashboard filters apply at once.** Changing a dashboard
  filter (date range, accounts, search, the Options toggles, a legend /
  summary click) takes effect "really, really slowly" (user report,
  2026-11-11). Rewritten 2026-11-11 for the page-entity architecture:
  3.65 already redraws the control at once, 3.77 adds keyed latest-wins
  DB jobs and timing logs, and 3.78 adds `Catalog` and `Domain` events.
  This task gives the dashboard its own loader.
  - **One filter value.** Today the filter is spread over separate
    fields, partly duplicated (`include_transfers` on `Shell` and in
    `view_filter`; accounts in `excluded_accounts` and
    `view_filter.account_ids`), and `refresh` assembles a `TxTableFilter`
    each time. Put one `DashboardFilter` struct (`Clone + PartialEq +
    Default`-ish: date range, accounts, search, the three toggles, category
    selection, month, drill-down, sort) on the dashboard state.
  - **A dashboard-only loader.** Every filter, selection and sort change
    runs one keyed read job (`call_keyed("dashboard", …)` on the reader,
    3.77) for the filtered transactions, chart rows, monthly summary and
    count only, using 3.88's shared `dashboard()` query. A burst of
    changes runs at most the in-flight job plus the newest; a superseded
    result is dropped.
  - **Search waits for a pause:** debounce 200 ms (a `Timer` task
    replaced on each keystroke); Enter applies at once. Other controls
    apply at once.
  - **Stale selection:** when a result shows the selected category is
    gone, correct the selection and start a new load, and return without
    applying the stale rows (today `refresh` calls itself from its own
    continuation and then applies the stale rows anyway).
  - A dashboard-only result updates only the table, chart and summary,
    never the history table, queue pickers, rule lists or category
    pickers.
  - Measure with 3.77's `FINANCI_TIMINGS=1` on the dev seed and a
    throwaway local 10 000-row / 300-rule seed (don't commit it); numbers
    in the commit message. Target: a filter change shows its result
    within 100 ms on a year of data (the timed test is written in 3.55).
  - Tests:
    - a filter change runs only the dashboard job (query-count hook);
    - five rapid search keystrokes run one job, after the debounce;
    - a burst of three filter changes ends with the last filter's rows,
      and a late superseded result doesn't overwrite them;
    - an import still reloads every page;
    - the existing filter, sort and selection tests pass.

- [ ] **3.89 Shared UI components.** (Design system, user decision
  2026-11-11.) Build the components `docs/ui-conventions.md` →
  *Components* describes, in `crates/financi/src/ui/widgets.rs`, so pages
  stop being styled by hand: `Panel`, `PageHeader`, `DataList` (heading
  row + virtual list; typed columns `Fixed(px) | Flex` with alignment
  shared by heading and rows; 40 px rows with dividers and hover; 36 px
  group headers; a hover-revealed actions column whose width is
  reserved), `FilterRow`, `InlineForm`, `Swatch`, `TypeTag` (on
  gpui-component's outline `Tag`), `control_slot`, `row_action_button`.
  - Each is a `RenderOnce` (or builder fn) with no page state, tested on
    its own in a headless window: heading and column left edges match,
    rows are the token heights, actions don't shift the row, a long label
    ellipsizes.
  - Prove them on the smallest pages: move the Transfers list and the
    Imports page header and history onto them, and set the DataTables to
    dividers instead of stripes (`stripe(false)`, row borders). The
    invisible `.opacity(0.0)` spacer buttons that align headings today go
    away where a `DataList` replaces them.
  - Later restyle tasks (3.34, 3.49, 3.51, 3.52, 3.58) build on these;
    don't restyle those pages here.
  - Remove the "(built in task 3.89; until then, match this by hand)"
    note from `docs/ui-conventions.md` → *Components*.

- [ ] **3.79 Transfers and Imports become their own views.** (Architecture,
  user decision 2026-11-11.) The first page extractions, chosen because
  they're the smallest; they set the pattern the others copy.
  - `TransfersPage` and `ImportsPage` entities in their modules, owning
    the state from 3.74's `TransfersState` / `ImportsState`, rendering
    themselves, and subscribing to `Catalog`'s `DataChanged` for the
    domains they depend on (`const DEPENDS: Domains`). On a matching
    event a visible page reloads; a hidden one marks itself dirty and
    reloads when shown. `Shell` keeps an `Entity` for each and renders it.
  - Pages talk to `Shell` (navigation, toasts) through events, not by
    holding the `Shell`.
  - **Keyboard:** the page root has a `FocusHandle`, `track_focus` and a
    `key_context("Transfers" | "Imports")`. Define one app-wide `Cancel`
    action bound to `escape` per context (as gpui-component does,
    `select.rs:35`); each page's `Cancel` handler closes its own
    transient UI (the Imports page: the armed Undo confirm).
  - Write the pattern down in `docs/tasks.md` → *Coding practices* (page
    entity, `DEPENDS`, events to `Shell`, `Cancel`) so the next
    extractions follow it.
  - No behaviour change otherwise; existing tests pass (selectors may
    move to the page).

- [ ] **3.80 The Rules page becomes its own view.** (Architecture.) Same
  pattern as 3.79, for `RulesPage`, before 3.58 restyles it.
  - Owns search, filters, collapse, sort, checked rows, the editor and
    pending actions; `DEPENDS = Rules | Categories | Transactions`.
  - Moves the "focus the search when the page opens" side effect out of
    `render` (`rules_focus_pending` + `window.defer`) into the page's
    show hook.
  - Show rule (from the dashboard's row menu) reaches it through an event
    carrying the rule id.
  - `Cancel` closes the editor, disarms a delete, clears the checked rows.

- [ ] **3.81 The Categories page becomes its own view.** (Architecture.)
  Same pattern as 3.79, for `CategoriesPage`, before 3.48. Owns the
  selection, the new-category form, rename, the rule list's editor and
  armed deletes; `DEPENDS = Categories | Rules | Transactions`. The rule
  rows it shares with the Rules page come from one row renderer used by
  both. `Cancel` closes the innermost open editor or form, then disarms.

- [ ] **3.82 The Dashboard becomes its own view.** (Architecture.) Same
  pattern as 3.79, for `DashboardPage`, after 3.56 and before 3.36. Owns
  the `DashboardFilter` (3.56), chart, summary, table and their loaders;
  `DEPENDS = Transactions | Categories | Accounts | Rules`. Its `Cancel`
  clears the innermost selection (chip, then drill-down), replacing the
  global `ClearCategorySelection` escape binding, which today also runs
  on other pages and clears hidden dashboard filters.

- [ ] **3.83 To categorize becomes its own view.** (Architecture.) Same
  pattern as 3.79, for `QueuePage`. Owns the queue and its lazily built
  pickers; `DEPENDS = Transactions | Categories | Rules`. Prune pickers
  for merchants outside the last rendered range plus a margin (today
  `queue_pickers` keeps one `SelectState` per merchant ever scrolled
  into view, each with a full copy of the category names).

- [ ] **3.91 Every existing page uses the shared components.**
  (Architecture, 2026-11-11.) After 3.89 and the page extractions
  (3.79–3.83); one commit per page, so it can be reviewed in parts. Move
  what's built by hand today onto 3.89's components, keeping each page's
  behaviour and tests (update selectors, don't delete tests):
  - **To categorize** (`QueuePage`): the list from 3.34 becomes a
    `DataList` (columns as today) inside a `Panel`, under a `PageHeader`.
  - **Categories** (`CategoriesPage`): the header becomes a `PageHeader`
    and the New category form an `InlineForm`; the tree becomes a
    `DataList` with indented subcategory rows (`Swatch`, name, count
    column), dropping its hand-rolled row layout.
  - **Dashboard** (`DashboardPage`): the filter row becomes a `FilterRow`
    (its Options popover is the shared one); the chart and summary panels
    become `Panel`s with title rows.
  - **Rules** stays for 3.58, and the Categories rule list for 3.49 (both
    already planned on the components).
  - **Sidebar:** replace the ad-hoc `sidebar_foreground.opacity(0.7)` tint
    with the `muted_foreground` token.
  - Done when no page builds its own heading row, divider rows, or
    invisible `.opacity(0.0)` alignment spacers (grep shows none outside
    `widgets.rs`), and every page fits 1200 × 720 as before.

- [ ] **3.57 Every page reloads only what it depends on.** Rewritten
  2026-11-11: the old 3.57 is split into this task, 3.84 (feedback within
  a frame) and 3.85 (render caches). Do it after the page extractions
  (3.79–3.83), as the check that the architecture holds.
  - Every page entity declares `const DEPENDS: Domains`; every mutation
    reports its `Domains` through `Catalog` (3.78). Write both lists as
    one table in the `Domain` doc comment and remove any remaining direct
    `refresh` calls; the full reload is left only behind import, undo and
    restore (all domains).
  - Hidden pages mark themselves dirty and reload when shown; a page
    switch with nothing dirty issues no query.
  - Tests: one per row of the table (the action reloads exactly the pages
    that depend on its domains, via the query-count hook); a page switch
    issues no query; a dirty hidden page reloads once when shown.

- [ ] **3.84 Every click answers within a frame.** (From the old 3.57,
  split 2026-11-11.) After 3.64 (per-action tasks), so a rollback can't
  be cancelled. Every button that starts DB work either updates the
  screen optimistically with rollback on error (row category change,
  hide / unhide, queue Categorize) or shows `.loading(true)` and is
  disabled until it finishes (rule and category edits; 3.47 / 3.54 make
  those optimistic later). Tests: change category, hide and Categorize
  update the row in the same frame before the DB job runs, and roll back
  on error; every other DB-backed button shows loading until done.

- [ ] **3.85 No per-frame work that grows with the data.** (From the old
  3.57, split 2026-11-11; code review list.) Cache derived view data on
  the page that owns it, rebuilt only when its inputs change:
  - **Compiled rules:** "Test a description" and Show rule call
    `winning_rule` / `pattern_matches` in `render` and on every menu open,
    recompiling every regex each frame. Keep one `CompiledRules` (and
    per-rule `CompiledPattern`s) in `Catalog`, rebuilt when rules change.
  - **Summary:** `summary_rows` runs every render with a linear
    `parent_of` per row and clones every row's months; the grid builds
    rows × (months + 1) cells, each with `format!` ids and a listener,
    though most rows are clipped. Build `SummaryRowView`s with
    pre-formatted `SharedString` cells when the summary loads, virtualize
    the rows, and key element ids by category id, not row index (index
    ids shift when a category expands).
  - **Colors and labels:** `category_colors` is built twice per dashboard
    frame and again on Categories; `rule_target_label` scans
    `child_choices` per row. Build both once per category change in
    `Catalog` (`HashMap<i64, SharedString>` labels).
  - **Rules page items:** build the grouped items and their sizes when
    rules, stats, filters, collapse or sort change; group with a map, not
    a linear `find`.
  - **Transactions table:** precompute the merchant and formatted cells
    per row when rows load, instead of `parse::merchant` and `format!` per
    visible cell per frame.
  - **Toasts:** move them into their own child entity so a toast tick
    re-renders only the toast layer, and stop the 100 ms ticker when no
    toast is animating (a sticky error toast ticks forever today).
  - Tests: each cache rebuilds on its input change and not on an
    unrelated notify (counter hooks); a toast tick doesn't re-render the
    page.

- [ ] **3.86 Layout from measured sizes.** (Code review, 2026-11-11.)
  Before 3.36 and 3.26. The chart's label budget and the table's flexible
  column widths are computed from the window width minus constants
  (`chart_plot_width = window_width − SIDEBAR_W − 2·PAGE_PAD …`,
  `tx_flex_widths(page_w)`, `history_flex_widths`) and from guessed
  character widths (`CHART_CHAR_PX`, `TEXT_SM_CHAR_PX`). Every layout
  change (3.36's legend, 3.45's full-width chart, 3.26's scrollbar
  gutter, a different font on Linux) silently breaks them.
  - Measure the plot area and the tables' containers: read their bounds
    in a `canvas` / `on_prepaint`, store them on the page, and
    `cx.notify()` when they change. Derive the chart's label plan and the
    tables' column widths from the measured bounds. DataTable has no flex
    columns, so widths are still computed, but from measured bounds.
  - Measure text with `window.text_system()` instead of per-character
    estimates where labels are truncated or thinned.
  - Delete `SIDEBAR_W`-based arithmetic outside the sidebar itself.
  - Tests: the label plan and column widths follow the measured width at
    1200 and 1600 px; changing the sidebar width constant changes no
    chart or table test result.

### Backend queue: scale, then correctness (in this order)

Runs beside the UI queue. 5.1 and 5.2 (in M5) follow 3.90.

- [ ] **3.77 A DbWorker that keeps reads fast.** (Architecture, code
  review 2026-11-11.) Backend only (`financi-db/src/worker.rs`, `lib.rs`);
  runs in a free slot any time after the Fixes lane. Today it is one FIFO
  thread with one connection: queued stale jobs still run, and a slow
  write (an import with pairing, a rule re-run, 5.1's snapshot) blocks
  every dashboard read behind it.
  - **Keyed latest-wins jobs:** `call_keyed(key, f)` keeps at most one
    *pending* job per key; a replaced job resolves `Superseded`, which
    callers drop quietly. This replaces the generation counter 3.56 used
    to plan.
  - **A reader:** a second thread with a read-only connection (WAL allows
    reads during a write) for list and aggregate queries (`call_read`).
    Writes, and reads that must see a write just made, stay on the
    writer. Set `busy_timeout = 5000` on both connections; run `PRAGMA
    optimize` after migrations.
  - **Interrupt stale reads:** keep the reader's
    `rusqlite::InterruptHandle`, so superseding a running keyed read
    interrupts it.
  - **Timings in one place:** with `FINANCI_TIMINGS=1` (debug builds),
    log each job's label, queue wait and run time. Tasks that used to
    plan their own timing log use this.
  - **Typed errors:** add `on_db_typed` returning `DbError` for new code,
    so the UI can tell "rule gone" from "constraint failed"; leave the
    existing `on_db` signature alone.
  - Tests: three keyed jobs queued behind a slow one run only the last;
    a read on the reader completes while a slow write runs; an
    interrupted read returns `Superseded`; panics still reach the caller
    as `JobPanicked` (3.70).

- [ ] **3.73 Imports and undo stay fast as data grows.** (Code review,
  2026-11-11; cheap fixes now, timed budgets in 3.55.)
  - **Compile rules once per import.** `categorize_new_rows`
    (`import.rs`) and `overrides.rs` call `pick_category` per row, which
    builds a new `CompiledRules` each time: every regex rule is compiled
    once per imported row, twice per import (preview and commit). Build
    `CompiledRules` once per call. Mark `pick_category` and
    `pattern_matches` as one-off helpers in their doc comments.
  - **Pairing compares only plausible pairs.** `run_transfer_pairing`
    loads nearly every unpaired row and `adjacency` tests every pair
    (billions of checks at 100 000 rows), on every import, unpair and
    review. Select only rows that can pair (unlocked chequing / savings
    rows; card rows in Card Payment), bucket them by `abs(amount_cents)`,
    compare within a bucket inside the date window, and lowercase each
    description once.
  - **Indexes,** in one new migration: `transactions(transfer_pair_id)
    WHERE transfer_pair_id IS NOT NULL` (deleting a row scans the whole
    table for rows pointing at it, once per deleted row, so undo is
    quadratic); `transactions(category_id)` (category reassign, counts
    and the category filter; 3.54 no longer needs to add it);
    `transactions(posted_date, import_id, source_line)` (the table's
    default order). Migration test checks all three.
  - **Rewrite only the touched years.** `rewrite_years` loads the
    account's rows for every year; filter `account_rows` to the years
    being written.
  - Tests: results unchanged (existing pairing, import and master-CSV
    tests pass); a pairing test with 20 000 unrelated unpaired rows plus
    one matching pair still pairs it and finishes in well under a second
    in a debug build.

- [ ] **3.88 One filter WHERE builder for every dashboard query.** (Code
  review, 2026-11-11.) Backend only (`financi-db/src/views.rs`,
  `lib.rs`); runs in a free slot. 3.46 depends on it.
  - The filter conditions are written out four times
    (`list_transactions_filtered` with its own `arg!` macro,
    `grouped_rows` with hand-numbered placeholders,
    `spend_by_parent_category`, `sidebar_counts`). Add one `FilterSql {
    sql, args }` built from `(&ViewFilter, Option<&TxTableFilter>)` and
    use it in all of them. Delete `spend_by_parent_category` (kept only
    for a test).
  - `spend_by_category` and `monthly_summary` each run the same
    aggregate. Add `dashboard(conn, filter) -> (Vec<CategorySpend>,
    MonthlySummary)` that aggregates once; keep the two as thin wrappers
    if tests need them.
  - The month filter uses `substr(posted_date, 1, 7) = ?`, which can't
    use the index; use `posted_date >= 'YYYY-MM-01' AND posted_date <
    <next month>`.
  - Tests: all existing view tests pass unchanged; one test checks the
    three queries agree on totals for the same filter.

- [ ] **3.87 Rule use stats.** (Split out of 3.58, 2026-11-11.) Backend
  only (`financi-db/src/rules.rs`); runs in a free slot before 3.58.
  Replace `rule_use_counts` with `rule_use_stats(conn) ->
  HashMap<i64, RuleUse { count, total_cents }>`, one pass over unlocked,
  non-card-payment, **non-hidden** rows (today `rule_use_counts` and
  `count_rule_matches` count hidden rows while every other count excludes
  them, so Uses disagrees with category counts once a row is hidden;
  make `count_rule_matches` exclude them too). Callers switch to it.
  Tests: count and signed total match a direct SQL sum for the rows each
  rule decides; hidden rows are excluded from both functions.

- [ ] **3.66 Simplii card payments only on the card.** (Code review,
  2026-11-11.) `Simplii::is_card_payment` (`financi-parsers/src/simplii.rs`)
  only sees the `RawTransaction`, so the Visa rule ("money in whose
  description contains PAYMENT") also runs on **chequing** imports. A
  chequing deposit such as "GST PAYMENT CANADA", "PAYMENT REVERSAL" or an
  e-transfer memo saying payment becomes `Transfers › Card Payment`,
  locked against rules and left out of totals: income vanishes. On the
  Visa side, a refund from a merchant whose name contains PAYMENT
  ("PAYPAL *PAYMENTS") is locked as a transfer too.
  - Change the trait to `fn is_card_payment(&self, txn: &RawTransaction,
    kind: AccountKind) -> bool` and pass the account's kind at import
    (`import.rs`) and in `overrides.rs`. Simplii returns false unless
    `kind == CreditCard`, and matches `PAYMENT` only together with
    `RECEIVED` or `THANK YOU` (as Amex's `PAYMENT RECEIVED`). Amex
    ignores `kind`.
  - **Repair existing data:** add a numbered migration (or a one-time
    startup step recorded in `settings`) that, for chequing and savings
    accounts, finds rows in Card Payment with `amount_cents > 0` and no
    `transfer_pair_id`, clears `category_locked` and re-runs rules on
    them. Before writing it, run a read-only count on a copy of the
    user's DB (`FINANCI_DB=.cache/real-copy.sqlite`, counts only, never
    commit it) and put the count in the handoff.
  - The opt-in `real_samples` test prints the card-payment count per
    file.
  - Tests: a chequing fixture row ` GST PAYMENT CANADA,,100.00` is not a
    card payment and counts as income; "PAYPAL *PAYMENTS" money in on the
    Visa is not a card payment; "PAYMENT RECEIVED THANK YOU" on the Visa
    still is; the repair unlocks and recategorizes an affected row and
    leaves a paired or negative one alone.

- [ ] **3.67 The CSV reader survives real-world files.** (Code review,
  2026-11-11.) All in `financi-core/src/raw.rs`, `lib.rs` and
  `financi/src/flow.rs`.
  - **Quotes only open at a field's start.** `split_records` and
    `split_fields` flip the quote state on every `"`, so an Amex
    description like `BEST BUY 55" TV    TORONTO` swallows the following
    lines until the next `"` and the whole file fails to import. Open a
    quote only at the start of a field; inside an unquoted field `"` is a
    literal; a quoted field closes at `"` followed by `,`, end of line or
    end of input; `""` stays an escaped quote. Hashing must not change
    for files that parse today (they contain no such quotes): add a test
    that every existing fixture hashes the same before and after.
  - **Line numbers users can find.** `source_line` is a 0-based record
    index (it skips blank lines), but errors print it as "record 0". Keep
    `source_line` (it's stored and orders rows) and add
    `RawRecord.file_line`, the 1-based physical line where the record
    starts; errors say "line 2: bad Date …".
  - **Non-CSV files fail fast.** Reject files over 50 MB or containing a
    NUL byte with "This isn't a CSV export.". Cut the header shown in the
    unknown-format message to ~120 characters. An unclosed quote in a
    file with no recognized header reports the unknown-format message.
    Run `flow::load_file` on the background executor, not the UI thread.
  - **Fixtures end to end:** an Amex file with CRLF and a BOM, a Simplii
    file in Windows-1252 with an accented description, a quoted amount
    `"1,234.56"`, a blank line mid-file, and the stray-quote row; plus a
    test that the same rows hash identically across the LF / CRLF / BOM
    variants.
  - `parse::cents` accepts commas only as thousands groups (`1,234.56`);
    `"1,2,3"` and `"12,34"` are rejected.

- [ ] **3.68 Near-match review asks only about changed charges.** (Code
  review, 2026-11-11.) In `financi-db/src/import.rs`:
  - **Repeat purchases aren't "changed charges."** `find_near_matches`
    compares new rows with existing rows that the same file still
    contains unchanged. A re-export holding yesterday's $4.50 coffee
    plus today's $5.00 one offers today's as "Same charge?" against
    yesterday's. Exclude existing rows whose hash (or alias) the file
    already contains. Test: two coffees on consecutive days, the earlier
    still in the file → no near-match.
  - **Bounded scan.** It loads every visible row of the account and
    allocates the cleaned merchant twice per pair. Limit the SQL to
    `posted_date` between the file's min date − 3 days and max date + 3
    days, compute each existing row's lowercased merchant once, and look
    candidates up in a `HashMap<merchant, Vec<row>>`.
  - **A replacement keeps a valid master CSV.** "Same charge" puts the
    new export's bytes on a row whose `import_id` is the old import, so
    the master CSV writes them under the old import's header and
    encoding. Don't offer a near-match when the two imports' header /
    encoding variant differs (the row imports as new). Test with a
    changed header.
  - **FX columns follow the replacement.** `apply_replacement` doesn't
    update `original_currency` / `original_amount_cents`, and
    `replacements` can't restore them. Add both `old_…` columns to
    `replacements` in a new migration; update on replace, restore on
    undo; test it.

- [ ] **3.69 Paired transfers stay consistent.** (Code review,
  2026-11-11.) In `financi-db` (`overrides.rs`, `transfers.rs`,
  `import.rs`):
  - **Editing one side unpairs both.** Totals exclude transfers by
    category kind, so `set_category` on a paired row to "Rent" makes that
    side spend while its partner stays a transfer, and the link icon
    stays. `set_category` on a paired row clears `transfer_pair_id` on
    both sides in the same transaction; the edited row keeps its manual
    category; the partner is unlocked and rules re-run on it.
  - **Unpair keeps manual choices.** `unpair_transaction` calls
    `clear_override` on both sides, discarding a manual category. Only
    clear the override on a side whose category is still transfer-kind.
  - **Undo re-pairs.** `undo_last_import` doesn't re-run pairing, so
    partners released by the undo stay unpaired until the next import,
    and reviews can point at deleted rows. Run pairing at the end of the
    undo, and drop reviews left with fewer than two live candidates in
    `list_transfer_reviews`.
  - **Pairing settles.** `pair_mutuals` makes one pass, so a row whose
    only competitor was paired in the same run stays unpaired with no
    review. Repeat until nothing changes.
  - **Card-payment pairs need one card and one chequing row.**
    `pair_categories_for` returns Card Payment when either side is a card,
    so a stale review could pair two card rows. Require exactly one of
    each.
  - Tests for each bullet.

- [ ] **3.70 Database guard rails.** (Code review, 2026-11-11.) Small
  `financi-db` fixes, one commit each or together:
  - **Undo names its import.** `undo_last_import(account_id)` deletes
    whatever import is newest; the UI looks the account up from the row
    the user clicked, so stale history could delete a different import.
    Take `expected_import_id` and refuse if it isn't the newest.
  - **Deleting a category keeps its kind.** `delete_category` accepts a
    reassign target of another kind, so deleting an expense subcategory
    into an income one silently turns spend into income. Refuse with
    `CategoryKindMismatch`, as `move_child` does.
  - **The startup self-check compares content.** `year_is_current` only
    compares names and row counts, so a replacement (or a failed rewrite
    after a commit) that keeps the count is never repaired. Compare the
    file's bytes with the bytes `year_files` already builds.
  - **Worker panics say so.** A panicking job reaches the caller as
    `WorkerGone` ("database worker is gone") although the worker
    survives. Add `DbError::JobPanicked`.
  - Tests for each.

- [ ] **3.72 Text rules ignore case for accented letters.** (Code review,
  2026-11-11.) Substring rules lowercase with `to_ascii_lowercase`
  (`financi-core/src/rules.rs`), regex rules are Unicode
  case-insensitive. So the text rule `Montréal` never matches
  `CAFE    MONTRÉAL`, while the regex `montréal` does. Use
  `to_lowercase()` on pattern and description, keep tie-break lengths as
  `chars().count()` of the original pattern, and add tests with accented
  upper and lower case. 3.55's Aho-Corasick matcher must keep this
  folding.

- [ ] **3.71 Row hashes don't depend on the encoding guess.** (Code
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

- [ ] **3.90 Build and tooling.** (Code review, 2026-11-11.) A `chore:`
  filler for any free slot.
  - Drop `spikes/*` from the workspace `members` (keep the directory, as
    `CLAUDE.md` says): every build, clippy and clean-checkout verify
    compiles that extra GPUI binary today.
  - Add `rust-version` to `[workspace.package]` matching `mise.toml`.
  - `build.rs`: also `rerun-if-changed` on `git rev-parse --git-path
    packed-refs` and `--git-path index`, so the version follows commits
    and packed tags.
  - `.githooks/post-commit`: before 1.0 a breaking change bumps the minor
    version, not 1.0.0; for a merge commit, scan the merged commits'
    subjects and take the highest bump.
  - `flow::parser()` returns `Option` instead of panicking on an unknown
    id, and `new_account_defaults` doesn't index `account_kinds()[0]`;
    add a parser-registry test: unique ids, at least one account kind
    each, and every parser's institution exists in the `institutions`
    table.

### Lane A — dashboard (after both queues)

- [ ] **3.36 Chart legend under the chart.** The legend column beside the
  bars (`LEGEND_W`, 140 px) still takes width the plot needs. Move it
  below the chart and condense it (user request, 2026-11-11; changes
  decision D1 from "beside" to "below"):
  - **Review update (2026-11-11):** On `DashboardPage` (3.82), after 3.86: the plot width is measured, so drop the `LEGEND_W` arithmetic rather than adjusting it. Leave room in the chips for 3.46's selected / dimmed state.
  - The plot takes the panel's full inner width; `chart_label_plan` /
    `chart_plot_width` no longer subtract `LEGEND_W` (remove the const).
  - **Legend:** a wrapping row of chips under the plot (`flex_wrap`,
    `gap_x_3`, `gap_y_1`): each chip is the 10×10 swatch and the name in
    `text_xs`, name ellipsized at ~120 px. Clicking, the tooltip
    (amount · share), the selected chip's `list_active` and "← All
    categories" (first chip when drilled in) all work as in 3.31.
  - At most **two lines** of chips (~40 px). If more chips than fit, the
    last visible chip is "+N more" (muted), whose tooltip lists the hidden
    names; clicking it does nothing. The chart panel keeps its fixed
    height (~340 px); the plot takes what the title and legend leave.
  - Tests: the plot is wider than before at 1200 × 720 (about +150 px);
    the legend paints below the plot's bottom edge and inside the panel;
    with 15 categories it uses ≤ 2 lines and shows "+N more" with the
    right N; clicking a chip still selects / drills; the 3.31 legend and
    bar-label tests are updated, not deleted. Update the *Dashboard* row
    of `ui-conventions.md` and D1.

- [ ] **3.45 Chart and summary on their own rows.** The middle row puts
  the chart panel (`CHART_PANEL_SHARE`, 45 %) and the monthly summary side
  by side at 340 px tall; both are cramped (user request, 2026-11-11;
  changes D1's "chart beside the summary"). The user chose **a scrolling
  dashboard** over tabs or shorter panels:
  - **Review update (2026-11-11):** On `DashboardPage` (3.82), after 3.86 (measured layout): widths come from measured bounds, not window-width arithmetic.
  - **Review update (2026-11-11):** Avoid nested vertical scrollers: the summary panel takes its content height (it keeps only its horizontal month scroll) so the page scroll owns vertical movement, and the table's height is the viewport minus the pinned filter row, not a fixed 480 px. These replace the 340 px summary and ~480 px table bullets below.
  - **Layout, top to bottom:** the filter row (3.35) stays **pinned**
    above the scroll area. Below it, one vertical scroll container
    (`id` + `overflow_y_scroll` + a `ScrollHandle` kept on the shell)
    holds the **chart panel, full width, 340 px**; then the **summary
    panel, full width, 340 px**; then the **transactions table, full
    width, fixed ~480 px**, scrolling inside itself. `gap_3` between
    them, page padding as now. Nothing in the scroll area is `flex_1`
    in height; a scroll container gives children no height to fill (see
    *GPUI pitfalls*), so every panel has an explicit height.
  - **Chart:** `chart_plot_width` uses the full page width (drop
    `CHART_PANEL_SHARE`; after 3.36 there's no legend column either).
    The band-label plan (3.15) gets the wider budget for free.
  - **Summary:** the wider panel shows more month columns before its
    horizontal scroll kicks in; `NAME_W` / `MONTH_W` / `TOTAL_W` are
    unchanged. A vertical wheel over its month viewport must scroll the
    page (today it forwards vertical wheel to the rows; check it reaches
    the page scroll when the summary has no vertical overflow).
  - **Table:** wheel over the table scrolls the table. The no-matches
    state fills the same 480 px box.
  - **Scroll position** survives refreshes and filter changes (the handle
    lives on the shell); it resets to the top only on entering the
    dashboard from another page. The empty-window state (no data) is
    unchanged and doesn't scroll.
  - Do it right after 3.36 (both change the chart's width); 3.36's test
    "plot is ~150 px wider" gets updated to the full-width numbers here.
  - Tests: at 1200 × 720 the chart and summary panels both span the page
    width and the summary's top is below the chart's bottom; the table
    is ~480 px tall; scrolling the page moves the panels while the
    filter row stays put; the scroll offset survives a filter change;
    the summary shows more month columns than before with a 12-month
    range. The 3.25 title-alignment test (chart and summary titles on
    one line) is replaced, not deleted. Update the *Dashboard* row of
    `ui-conventions.md` and D1.

- [ ] **3.46 Global dashboard filters, and Clear all.** Every dashboard
  filter should apply to all three views at once: a category picked on
  the chart should show in the summary and the table too. Add a button
  that clears every filter (user request, 2026-11-11). Today the state
  is already shared (`legend_selection` + `summary_month`, set by chart,
  legend and summary-cell clicks; `sync_chart_to_selection`), but only
  the table *filters* by it. The summary highlights just the clicked
  cell, the chart ignores the month, and search / Uncategorized only /
  Show hidden are table-only (3.4) and never reach the chart or summary.
  - **Review update (2026-11-11):** Its backend half (the shared WHERE builder and one aggregate query) is now 3.88; build on it. The filter state is 3.56's `DashboardFilter`, so Clear all is `= default` and "shown" is `!= default`.
  - **Review update (2026-11-11):** Filter chips move under the pinned filter row, next to Clear all.
  - **Review update (2026-11-11):** Escape is the `DashboardPage`'s `Cancel` action (3.82), not a global handler.
  - **One filter model.** Fold the table-only fields into the shared
    filter: give `spend_by_category` and `monthly_summary` the same
    search, `uncategorized_only` and `show_hidden` narrowing as
    `list_transactions_filtered` (factor the `WHERE` builder in
    `views.rs` so all three queries share one function; no copies).
    Sort stays table-only.
  - **Category selection** (from a bar, a legend chip or a summary cell)
    shows in all three:
    - **Chart:** the selected bar keeps its color and the others dim to
      ~35 % opacity (bars aren't filtered away; the chart stays
      context). The drill-down (3.5) works as now.
    - **Summary:** the selected category's **whole row** is `list_active`;
      a selected subcategory auto-expands its parent. Rows aren't
      removed.
    - **Table:** filters to it (as now).
  - **Month selection** (summary cell) shows in all three:
    - **Chart:** the chart totals cover only that month, and the panel
      title reads "Spending by category · Oct 2026".
    - **Summary:** that month's column is tinted (`list_hover`), the
      selected cell `list_active`.
    - **Table:** filters to it (as now).
    - A month can be selected on its own, without a category: add a click
      on the summary's **month heading** that toggles the month alone.
      Today the month only narrows a category selection; drop that rule
      (keep Esc clearing both).
  - **Clear all:** a ghost button "Clear all" (`x` icon) at the right of
    the 3.35 filter row, just before the count, shown only when any
    filter differs from the default. It resets everything: date range
    to the default, all accounts checked, search empty, the three
    Options toggles off, and the category/month selection and drill-down
    cleared, then one refresh. The no-matches "Clear filters" link calls
    the same function (rename `clear_filters` → `clear_all_filters` and
    extend it). Esc keeps clearing just the selection.
  - The filter row must still fit 1200 × 720 with the custom range open
    and Clear all shown (3.35's fit test, extended).
  - Tests: DB tests that chart and summary totals honour search, hidden
    and uncategorized-only like the table does. UI tests:
    - clicking a bar dims the other bars, highlights the summary row and
      filters the table;
    - a summary cell's month narrows the chart totals and retitles it;
    - a month heading selects the month alone;
    - search narrows all three;
    - Clear all resets every filter (one assertion per filter) and
      hides itself;
    - the no-matches link does the same.

    Update the *Dashboard* row and *Interaction* section of
    `ui-conventions.md` (filters are global; Clear all).

### Lane B — Categories and Rules pages (after both queues)

- [ ] **3.58 A cleaner Rules page with totals.** The Rules page (3.28)
  "is very very ugly". The user also wants each rule's **total amount**
  next to its count, and the page must stay virtualized and responsive
  (user request, 2026-11-11). Today it has 24 px rows on the page
  background with no panel, a filter row of mixed controls, plain group
  header rows, and a Move bar that pushes the list down. Keep every
  behaviour of 3.28 / 3.29 (search, Test a description, Type, Unused,
  grouping and collapse, sort, checkboxes and Move, Show rule landing);
  change the look and add the totals:
  - **Review update (2026-11-11):** Split: the DB part (`rule_use_stats`, hidden rows excluded) is now 3.87; this task is the UI on `RulesPage` (3.80), built from 3.89's `PageHeader`, `FilterRow`, `DataList` (40 px rows, 36 px group headers on `muted`) and `row_action_button`. Ignore the bullets below that restate those components' geometry; follow the components.
  - **Review update (2026-11-11):** While its add form is open, the header's New rule shows outline (one primary per page).
  - **Review update (2026-11-11):** Cached `CompiledRules` and labels come from `Catalog` (3.85); group with a map, not a linear `find`.
  - **Header:** "Rules" and a muted summary "N rules · M unused"
    (`flow::count`), then **New rule** (`plus` icon, primary, no "…",
    3.43) at the right.
  - **Filter row:** one 32 px row like the dashboard's (3.35): Search
    `flex_1` with a leading search icon and the placeholder "Search rules
    or categories". Then a Type select (~140 px) and an **Options**
    popover holding **Test a description** and **Unused only**, showing
    "Options · N" when N are on. Each control without a visible label
    gets a tooltip.
  - **One bordered panel** (D7, `radius`, no fill) fills the rest of the
    page and holds the list:
    - a 32 px heading row inside it (`text_sm` muted, `px_3`, bottom
      `border`), with the columns: checkbox (24 px), **Pattern**
      (`flex_1`), **Type** (80 px), **Uses** (80 px, right), **Total**
      (110 px, right), then actions;
    - **group headers** are 36 px rows on `muted` background: chevron,
      "Food › Coffee" semibold, then the muted "4 rules", and the group's
      summed Uses and Total right-aligned under their columns;
    - **rule rows** are 40 px, `px_3`, with a bottom `border` divider and
      `list_hover`. The pattern is in `mono_font_family` (regexes are
      easier to read), ellipsized with a tooltip. Type is a small
      outline tag ("text" / "regex") instead of plain text. A 0 in Uses
      is muted.
    - Edit / Delete (icon-only ghost buttons, `pencil` / `trash-2`,
      with tooltips; see 3.43) show only on row hover or when the row
      is focused or has the editor open. Their column keeps its width so
      nothing shifts.
  - **Total column:** the signed sum of the transactions the rule
    *decides* (same set as Uses), formatted as the transactions table
    does (`flow::money`, money in shown in `success`). The header's sort
    buttons get **Total** too (by absolute amount, descending).
  - **Move bar:** with rows checked it **replaces the heading row**
    inside the panel (same 32 px), with no extra row pushing the list:
    "N selected", the subcategory picker (searchable), **Move**, and
    **Clear**.
  - **DB:** replace `rule_use_counts` with `rule_use_stats(conn) ->
    HashMap<i64, RuleUse { count, total_cents }>`, which also sums
    `amount_cents`, in the same single pass. Callers (Uses on this page,
    3.49 on the Categories page, 3.52) use it.  Group sums are
    computed in Rust from the per-rule stats.
  - **Virtualized and responsive:** the list stays one `v_virtual_list`
    (headers included). Build `rules_page_items` and the per-row display
    data (formatted totals, labels) **only when** rules, stats, filters,
    collapse state or sort change, cached on the shell (3.57's render
    audit), not every frame. Search filtering is debounced by 150 ms.
    Per-row widgets (checkbox, buttons) are built only for visible rows
    (3.24's test pattern). Test with 1 000 rules built inside the test; the timed
    16 ms-a-frame budget is checked at the very end (3.55).
  - Must fit 1200 × 720: no horizontal overflow, nothing clipped, with
    long patterns and category names.
  - Tests:
    - the panel and heading row paint inside the page;
    - Total shows each rule's signed sum, matching a direct SQL sum for
      the rows it decides;
    - group headers show the summed Uses and Total;
    - sorting by Total works;
    - Edit / Delete paint only on hover / focus;
    - the Move bar replaces the heading row, and the list's top doesn't
      move when rows are checked;
    - with 1 000 rules only the visible rows are built, and the item
      cache rebuilds only on input changes (counter hook);
    - every 3.28 / 3.29 behaviour test still passes (update their
      selectors, don't delete them).

    Update the *Rules* row of `ui-conventions.md`.

- [ ] **3.48 The rule editor gives the pattern room.** Clicking Edit on
  a rule swaps the row for an editor whose Pattern input shrinks so far
  that the pattern can't really be edited (user report, 2026-11-11).
  Cause: `rule_editor_row` is one `h_flex` with Pattern `flex_1` +
  `min_w_0` and, after it, the regex switch, a 160 px Category picker,
  Save and Cancel. In the detail panel (page minus the 210 px sidebar
  and the 300 px tree panel) the fixed controls take ~400 px of ~600, and
  3.53 removes the editor's picker, which frees most of that. The user also
  saw the editor's regex switch sit off the row's line (3.42's cause).
  - **Review update (2026-11-11):** Build the editor with 3.89's `InlineForm`. After 3.53 removed the picker, check whether the add row still needs two lines; if not, leave it on one.
  - **Two lines:** line 1 is the "Pattern" label over the input at the
    editor's **full width**. Line 2 is one `h_flex().items_center()` of
    32 px controls, with no label line: the regex switch, then `ml_auto`
    Save and Cancel (no Category picker, 3.53). Line 2 has no labels,
    so the switch lines up without 3.42's slot (3.42 still applies to
    the add-rule row). The preview count and error line stay under both
    lines.
  - Bump the editor's entry in `rule_sizes` (today 80 / 108 px) to the
    new height (~two control lines + label + preview, ~120 / ~148 px with
    an error) so the virtual list doesn't overlap the next row.
  - Enter saves, Escape cancels, focus lands in the pattern input with
    the caret at the end (check it does today; if not, add it).
  - The add-rule row: if its Pattern input is < 240 px wide at
    1200 × 720 after 3.53, give it the same two-line shape; otherwise
    leave it on one line.
  - Tests: at 1200 × 720 the editor's pattern input is at least 90 % of
    the detail panel's inner width; the switch, Save and Cancel
    share a vertical centre within 1 px; the row below the editor doesn't
    overlap it (bounds); Save/Cancel/Enter/Escape tests from 3.24 pass.
    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.50 No Organize heading or repeated labels.** In the category
  detail panel, the **Organize** section title and the "Move to
  category" field label just repeat what the picker's placeholder
  ("Move to category…") already says (user request, 2026-11-11). The
  parent side has the same problem: the "New subcategory name" label sits
  over an input whose placeholder is "New subcategory name".
  - **Review update (2026-11-11):** Implement with 3.89's `InlineForm`; the label rule it describes is now the general rule in `ui-conventions.md` → *Patterns*, so no exception note is needed.
  - Remove `region_title("Organize")` and both `field_label`s in that
    section. Each side becomes one `h_flex().items_center().gap_2()` row:
    - subcategory: `[Move to category… ▾] [Move]`;
    - parent: `[New subcategory name] [Add subcategory]`.
    Keep the `border_t_1` dividers above and below, so it still reads as
    its own section.
  - This overrides 3.16's visible-label rule for this section only; the
    placeholder is the label. Give the input and the select an
    `accessibility_label` with the same text.
  - Tests: no "Organize" or "Move to category" / "New subcategory name"
    label paints in the detail panel (the placeholders still do);
    the control and its button share a vertical centre; move and add-
    subcategory still work (existing tests). Update the *Categories* row
    of `ui-conventions.md` (drop **Organize** from the section list) and
    note the exception in the *Forms* labels rule.

- [ ] **3.51 Organize and Delete hidden until asked for.** The category
  detail panel spends vertical space on two sections that are rarely
  used, **Organize** (add a subcategory / move a subcategory) and
  **Delete** (move its transactions, then delete), so the rules list gets
  less of the panel (user request, 2026-11-11). Hide both by default:
  - **Review update (2026-11-11):** Forms are 3.89's `InlineForm`s; Escape closes them through `CategoriesPage`'s `Cancel` action (3.81).
  - **Title row actions:** next to Rename, at the right of the category
    name, add small buttons, each with an icon and no "…" (3.43):
    - **Add subcategory** (`folder-plus`), on a parent;
    - **Move** (`folder-input`), on a subcategory;
    - **Delete** (`trash-2`), not on system categories.
    Rename stays where it is.
  - Each opens an **inline bordered form** right under the title row,
    the same pattern as the header's New category form (3.23):
    - Add subcategory: `[New subcategory name] [Add subcategory]
      [Cancel]`;
    - Move: `[Move to category… ▾] [Move] [Cancel]`;
    - Delete: `[Move its transactions to… ▾] [Delete] [Cancel]`. Its
      Delete is still the two-click "Click again to delete" confirm in
      `danger`.
    The forms have no section titles or field labels (3.50: the
    placeholder is the label; `accessibility_label`s set). One form at a
    time: opening one closes another; Cancel, Escape, a successful
    action, selecting another category or leaving the page closes it.
    The title-row button of the open form shows `.selected(true)`.
  - Remove the Organize and Delete sections and their `border_t_1`
    dividers. With no form open the panel is the title row, a divider,
    then **Rules**, which gets all the remaining height.
  - Must fit the detail panel at 1200 × 720 with a long category name:
    the name ellipsizes before the buttons do. If the buttons can't all
    fit, make Add subcategory / Move / Delete icon-only with tooltips;
    don't let them wrap or clip.
  - 3.50's label removal applies inside the forms; do 3.50 and 3.51 in
    one branch.
  - Tests:
    - with no form open, the Rules section's top sits right under the
      title row's divider and the rule list is taller than before (bounds);
    - each button opens its form and the others stay closed;
    - opening a second form closes the first;
    - Cancel, Escape and selecting another category each close it;
    - add / move / delete still work through the forms (existing tests,
      updated to open the form first);
    - system categories have no Delete button;
    - at 1200 × 720 the buttons paint inside the panel with a 60-character
      name.

    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.49 A Uses column instead of Category.** Since 3.23 the
  Categories page's rule list only shows the selected category's rules, so
  on a subcategory every Category cell says the same thing. The user
  wants the count of transactions each rule matches there instead (user
  requests, 2026-11-11).
  - **Review update (2026-11-11):** Also moves the Categories rule list onto 3.89's `DataList` with 40 px rows, using the same row renderer as the Rules page (3.81), so a rule looks the same on both pages. Uses come from 3.87's `rule_use_stats`.
  - **Replace the Category column with Uses** (`RULE_USES_W`, 80 px,
    right-aligned, `flow::count`), the same number and wording as the
    Rules page's Uses column (`rule_use_stats` from 3.58, its `count`): the transactions
    the rule *decides*, so it agrees with stored categories and with
    3.52's "decided by this rule". A heading tooltip says "Transactions
    this rule categorizes". 0 shows in `muted_foreground`. The counts are
    loaded in the same refresh as the rules (one `rule_use_stats` call; no
    per-row queries) and update after every rule change.
  - **Parent selected:** its list mixes its subcategories' rules, so the
    subcategory also shows: a muted `text_xs` tag with the child's name
    (not "Parent › Child") after the pattern, inside the Pattern column,
    ellipsized first. A subcategory selection shows no tag.
  - Clicking the Uses heading sorts by it (descending), clicking Pattern
    goes back, as on the Rules page. Reuse its sort state if it fits;
    otherwise a separate `cat_rules_sort_by_uses`.
  - Tests:
    - no "Category" heading or cell paints;
    - "Uses" paints with each rule's count, and matches the Rules page
      for the same rule;
    - a rule overlapped by a longer one shows only the rows it wins;
    - the count updates after a rule edit or delete;
    - on a parent, the subcategory tags paint, and on a subcategory they
      don't;
    - Uses sort works;
    - 3.23's heading alignment and fit tests pass.

    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.43 No buttons end in "…".** The user doesn't want "…" on
  buttons; use an icon instead, or just drop it (user request,
  2026-11-11). This applies 3.32's Imports-header rule to every button.
  All of them are on the Categories page today:
  - **Review update (2026-11-11):** The *Copy* wording is already in `ui-conventions.md` (rewritten 2026-11-11). Rule-row Edit / Delete come from 3.89's `row_action_button`; what's left is the title-row and header buttons on the Categories page, and the guard test.
  - **New category…** (`NEW_CATEGORY`, header, primary): `plus` icon +
    "New category".
  - **Rename…** (detail panel title row): `pencil` icon + "Rename".
  - **Delete…** in the Delete form (3.51 moved it there; `confirm_label`):
    `trash-2` icon + "Delete"; armed, it still reads "Click again to
    delete" in `danger`.
  - Rule rows (on both the Categories and the Rules page; skip what
    3.58 already changed): **Edit** gets `pencil`, **Delete…** becomes `trash-2` +
    "Delete" (armed text unchanged); both stay `.ghost()` `text_xs`. The
    invisible heading spacers (`rule-heading-edit-spacer`,
    `rule-heading-delete-spacer`) change the same way so the headings stay
    aligned with the columns.
  - **Not buttons, unchanged:** app-menu items ("Import CSV…", "Export…",
    "Restore…"), the row context menu's "Change category…" submenu,
    select placeholders ("Move to category…", "Category…") and the
    "+ New account…" picker item. Menus keep the platform's "…" for items
    that open a dialog.
  - Check 3.23's fit rule still holds (Edit / Delete paint inside the
    detail panel at 1200 × 720 and 1600 × 900): an icon adds ~20 px per
    button.
  - Tests: each of the buttons above paints an icon and its text has no
    "…"; the armed labels are unchanged (existing two-click tests); a
    guard test that scans `ui.rs` for `.label("…")` / `const … = "……"`
    used as a button label and fails, so no new one creeps in; 3.23's
    heading-alignment and fit tests pass. In `ui-conventions.md`, replace
    the *Copy* bullet's "Use an ellipsis when the button opens a dialog or
    picker" with "Buttons never end in "…"; actions carry a Lucide icon
    left of the label (menus keep "…")", and update the *Categories* row.

- [ ] **3.60 "Category type" label.** From `plan.md` → *Label the
  category type dropdown* (never made a task; user confirmed
  2026-11-11). The New category form labels its kind picker "Kind". Make
  the field label "Category type" (`ui.rs`, the `cat:new-form`). The
  detail panel's muted "Expense · N transactions" line shows the value,
  not the word, and stays. Check no other user-visible "Kind" remains.
  Keep the form fitting the header row at 1200 × 720. Tests: the form
  paints "Category type" and no "Kind" label. Tick the `plan.md` box and
  update the *Categories* row of `ui-conventions.md`.

- [ ] **3.52 A rule's transactions under the rule list.** Nice-to-have
  (user request, 2026-11-11): clicking a rule shows, in its own scrollable
  area, the transactions that rule matches. Do it after 3.51 (which frees
  the vertical space) and after 3.28 (whose row checkboxes are the
  multi-select; this is a separate single **focus**).
  - **Review update (2026-11-11):** The matches panel is a 3.89 `DataList`; its loader is a keyed read job (`rule_matches:<id>`, 3.77) instead of a hand-kept `Task`; the decides set agrees with 3.87's `rule_use_stats`.
  - **Focus:** clicking a rule row (not its checkbox, Edit or Delete)
    makes it the `focused_rule` (`list_active` row background); clicking
    it again, Escape, selecting another category or leaving the page
    clears it. Deleting the focused rule clears it; editing keeps it.
  - **Layout:** with a rule focused, the Rules section splits vertically.
    The rule list stays on top (`flex_1`, `min_h(120)`), then a
    `border_t_1` divider. Below it is the **matches panel** (~45 % of the
    section, `min_h(160)`): a header "Transactions matching `PATTERN`"
    (pattern ellipsized, tooltip) with a muted "N · M decided by this
    rule" summary, then a virtual list (`v_virtual_list`, scrollbar per
    3.26) of rows. With nothing focused the list takes the whole section,
    as now.
  - **Rows:** Date (110 px), Description (`flex_1`, ellipsized with
    tooltip), Account (`text_sm` muted, ~140 px), Amount (right-aligned,
    `flow::money`). Rows the rule matches but doesn't decide carry a
    muted trailing tag and `muted_foreground` text:
    - "won by `OTHER`" when another rule wins (`winning_rule`);
    - "locked" when the row's category is locked.
    Rows the rule decides sort first, then by date descending.
  - **DB:** `rule_matches(conn, rule_id) -> Vec<RuleMatchRow>` in
    `financi-db/src/rules.rs`: every transaction (locked or not) whose
    description the rule's pattern matches (`CompiledPattern`, compiled
    once), with a `status` of `Decides`, `WonBy(rule_id)` or `Locked`.
    Use one `CompiledRules` per call, as `rule_use_stats` (3.58) does, so the
    "decides" set agrees with Uses (3.28). Loaded through `DbWorker` when
    focus changes and after every refresh while focused; a stale result
    for a previous focus is dropped (keep the `Task`, drop on refocus).
  - **While editing** the focused rule, the panel keeps showing the
    saved rule's matches (the live count under the editor is the
    preview); it reloads after Save.
  - Empty state inside the panel: "This rule matches no transactions".
  - Tests:
    - DB: statuses for decides / won-by / locked rows, consistent with
      `rule_use_stats`; an invalid regex returns empty.
    - UI: clicking a row focuses it and the panel paints below the list
      inside the section; the summary counts are right; a won-by row shows
      its tag; Escape and re-click clear focus; focus changes drop a late
      result; at 1200 × 720 both the list and the panel are at least their
      minimum heights with 3.51's forms closed.

    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.47 A deleted rule leaves the list at once.** After the second
  click on a rule's Delete, the row stays in the list for a noticeable
  moment before it disappears (user report, 2026-11-11, after 3.40
  landed). 3.40 made `reapply_rules` fast (34 ms for 5 000 rows × 50
  regex rules), so the wait is elsewhere. Today `arm_delete_rule` only
  changes the list after **both** the `delete_rule` task **and** the
  full `refresh` finish: `refresh` re-runs every page's queries
  (transactions, chart, summary, imports, category tree and counts,
  rules, queue, transfer reviews) in one `on_db` call before
  `self.rules` is replaced. On top of that, every `DbWorker` job queues
  behind any in-flight one (the live preview count, a previous refresh).
  - **Review update (2026-11-11):** Do it after 3.84 (feedback within a frame) and 3.64 (per-action tasks); `FINANCI_TIMINGS` comes from 3.77, and the scoped reloads from 3.78 / 3.57.
  - **After 3.57.** 3.56 / 3.57 scope the refresh and add the
    `FINANCI_TIMINGS=1` log; this task adds optimistic list updates and
    fixes whatever the measurements still show.
  - **Measure first.** With 3.56's `FINANCI_TIMINGS=1` log, record the time from the confirming click to (a)
    `delete_rule` returning, (b) the refresh result arriving, and each
    query inside `refresh`. Run it on the dev-seed DB and on a copy of the
    real DB (`FINANCI_DB=.cache/real-copy.sqlite`, timings only; never
    commit it). Put the numbers in the commit message, and fix whatever
    dominates if it isn't covered below (e.g. a missing index for
    `transaction_counts` or `uncategorized_by_merchant`).
  - **Optimistic removal:** on the confirming click, drop the rule from
    `self.rules` (and the visible list) right away and `cx.notify()`,
    keeping a copy. On error, put it back at its position and show the
    error in `rule_error`. The refresh still runs after and brings
    counts and categories up to date.
  - **Do the same for Save and Add**, so a rule edit shows its new
    pattern/category in the row at once, and a new rule appears at once,
    with the same rollback on error. Rollback on a failed Add means
    removing the provisional row; it has no id until the DB returns one,
    so give it a temporary negative id and swap it for the real one.
  - Tests: after the confirming click, the rule is gone from the list in
    the same frame, before the DB task completes (run the UI without
    advancing the executor); a failing delete (rule removed underneath)
    restores the row and shows the error; an edit's new pattern shows
    before the save completes; a new rule appears before its add
    completes and then carries its real id. The 3.24 delete and edit
    tests still pass.

- [ ] **3.54 Category changes don't lag the page.** Deleting a category
  and moving a subcategory (and likely add / rename) freeze or lag the
  page noticeably (user report, 2026-11-11). The DB side is small:
  `move_child` is one `UPDATE`, `delete_category` two `UPDATE`s and a
  `DELETE`, with no rule re-run. That leaves two suspects:
  - **Review update (2026-11-11):** Do it with 3.47, after 3.84. The `Names` list built once and shared, and the `transactions(category_id)` index, are already done by 3.78 and 3.73: drop those bullets and measure what's left.
  - (a) As in 3.47, the change waits for the full `refresh`, which re-runs
    every page's queries.
  - (b) **UI-thread work on the result.** A changed category list
    re-sets items on every category picker: `sync_queue` re-sets every
    surviving queue picker, plus the rule / move / delete targets, the
    Rules page's Move picker and the table delegate's `child_choices`.
    The tree virtual list is also rebuilt.

  There's also no index on `transactions(category_id)`, so the
  reassign `UPDATE` and `transaction_counts` scan the table.
  - **Measure first,** with 3.56's `FINANCI_TIMINGS=1` logging, after
    3.57's scoped refreshes (do 3.47 and 3.54 in one branch), on the dev-seed DB and on a copy of the real
    DB. Log for each of delete / move / add / rename: click to DB
    returning, to refresh result, the refresh's per-query times, and the
    **main-thread time spent applying the result** (from entering the
    `update_in` closure to its return, broken down by picker re-sets and
    table refresh). Numbers go in the commit message; fix what dominates.
  - **Optimistic tree update:** delete removes the node (and its row
    count moves to the reassign target) and move re-parents the node in
    `self.categories` right away, with `cx.notify()`; roll back and show
    the error on failure. The refresh then reconciles.
  - **Cheaper picker refresh:** build the `Names` list once per refresh
    and share it (`Rc`/`SharedString`s), and re-set items only on pickers
    that are **built and visible** (queue pickers are already lazy per
    3.19; do the same for the rest). Other pickers re-set lazily when
    next opened or shown (a `categories_version` counter compared on
    render).
  - **Index:** if the measurements show the scans matter, add a numbered
    migration `CREATE INDEX tx_category ON transactions(category_id)`
    (0001 stays frozen).
  - Tests:
    - after a confirmed delete or a move, the tree shows the change in
      the same frame, before the DB task completes;
    - a failing move (e.g. target removed underneath) rolls back and
      shows the error;
    - after a category change, the hidden pickers are re-set on their
      next render, not during the refresh (count re-sets with a test
      hook);
    - if the index is added, a migration test checks it exists;
    - the existing category CRUD tests pass.

### Lane C — small tasks (after both queues)

- [ ] **3.63 An app icon.** The window and Dock / taskbar show the
  default icon (user request, 2026-11-11). The source artwork is
  committed: `assets/icon/financi.svg` (1024 × 1024, macOS icon grid: a
  dark rounded square with three rising bars, the tallest green). Don't
  redesign it; ask the user if it needs changes.
  - **Rendered PNGs, committed:** `assets/icon/png/financi-<n>.png` for n
    = 16, 32, 64, 128, 256, 512, 1024, made by `scripts/icons.sh` with
    `rsvg-convert` (installed here) and exposed as `mise run icons`. The
    build reads the PNGs, so it needs no SVG renderer.
  - **Linux, X11:** set `WindowOptions.icon` (`gpui-pre`
    `platform.rs`, X11 only) from the 256 px PNG, embedded with
    `include_bytes!` and decoded with the `image` crate. `image` 0.25 is
    already in `Cargo.lock` through GPUI; add it as a direct dependency at
    that version with `default-features = false, features = ["png"]`.
  - **Linux, Wayland:** compositors ignore per-window icons and match the
    window's `app_id` to a desktop entry. Set `WindowOptions.app_id =
    Some(APP_ID)` (`const APP_ID: &str = "financi"`) and add
    `packaging/linux/financi.desktop` (`Name=financi`, `Exec=financi`,
    `Icon=financi`, `StartupWMClass=financi`, `Categories=Office;Finance;`).
    `mise run install-desktop` copies it to `~/.local/share/applications/`
    and the PNGs to `~/.local/share/icons/hicolor/<n>x<n>/apps/financi.png`.
  - **macOS:** the Dock icon comes from the app bundle. Add `mise run
    bundle` (`scripts/bundle-macos.sh`): build release, make
    `target/release/financi.app` with an `Info.plist`
    (`CFBundleIconFile`, `CFBundleIdentifier` `dev.financi.app`,
    `CFBundleShortVersionString` from the 3.39 version) and an `.icns`
    made with `iconutil` from an `.iconset` of the PNGs. For `cargo run`,
    also set the Dock icon at startup with
    `NSApplication::setApplicationIconImage` through `objc2-app-kit`
    (0.3.2 is already in `Cargo.lock`; add it as a macOS-only dependency
    at that version). If either dependency can't be added at the locked
    version, stop and ask.
  - Tests: the embedded PNG decodes to 256 × 256 RGBA; the desktop file's
    `Icon` and `StartupWMClass` equal `APP_ID`; `scripts/icons.sh` is
    idempotent (re-running changes no committed PNG). Check by hand on
    this Linux machine that the taskbar shows the icon after `mise run
    install-desktop`, and say which session type (X11 / Wayland) you
    checked. Add the new `mise` tasks to `CLAUDE.md` → *Commands*.

- [ ] **3.37 In-row Undo says "Undo".** 3.32 made the newest history
  row's Undo an icon-only button, which the user finds unclear. Make it
  the `undo-2` icon **plus the label "Undo"** (still `.xsmall().ghost()`,
  vertically centered with ≥ 4 px above and below); armed it still reads
  "Click again to undo" in `danger`. Keep the "Undo this import" tooltip.
  Recompute `history_undo_col_w` so both labels show in full and the
  column is no wider than needed. Tests: the idle button's text is
  "Undo" and it paints an icon; 3.32's bounds and fit tests still pass
  in both states. Update the *Imports* row of `ui-conventions.md`.
  - **Review update (2026-11-11):** The history row's Undo keeps its text label: an explicit exception to the icon-only `row_action_button` (user request). Note it in `ui-conventions.md` → *Pages* (Imports).

- [ ] **3.61 The import preview names the institution.** From `plan.md`
  → *Make account selection on import clearer* (never made a task; user
  confirmed 2026-11-11). The account picker already lists "Institution ·
  Account" (`Account::display_name`), but the preview's only sign of the
  institution is a muted `text_xs` "Detected format: …" after the file
  name, which is easy to miss when choosing which card a file belongs to.
  - Line one of the preview: file name, then the **institution name**
    (e.g. "American Express", from the parser's institution, not the
    parser id) as an outline tag in `text_sm`, replacing "Detected format:
    …".
  - The "Into" picker lists only that institution's eligible accounts, as
    now; check each item reads "Institution · Account" and that "+ New
    account…" creates the account under the detected institution.
  - Tests: the preview paints the institution name and no "Detected
    format"; the picker items carry the institution. Tick the `plan.md`
    box and update the *Import preview* bullet of `ui-conventions.md`.

- [ ] **3.59 Searchable category pickers.** From `plan.md` → *Make
  category dropdowns usable with thousands of categories* (never made a
  task; user confirmed 2026-11-11). A flat scrolling list doesn't work at
  that size, and same-named subcategories under different categories
  can't be told apart.
  - **Review update (2026-11-11):** `Catalog` (3.78) builds the grouped `SelectGroup` items once per category change; every picker shares them. 3.83 already prunes the queue's pickers.
  - gpui-component's `SelectState` has `.searchable(true)`
    (`gpui-component-0.9.0/src/select.rs`, ~line 305: a search input at
    the top of the menu), `SearchableVec` and `SelectGroup` for grouped
    items, and `Select::menu_width`. Use them; don't hand-roll a picker.
  - **Which pickers:** every `Select` that picks a category or
    subcategory: the To categorize row pickers, the Rules page Move bar,
    the Categories page's Move form (categories) and Delete form
    (subcategories), the parent-scoped add-rule picker (3.53; searchable
    once it has more than 10 items), and any others a grep for
    `child_choices` / `SelectState::new` finds. The table's right-click
    *Change category…* submenu is a menu, not a `Select`; leave it.
  - **Items:** subcategories grouped under their category (`SelectGroup`,
    the category as the group title), so each result is seen with its
    category. Typing matches category **and** subcategory names, case-
    insensitive: "food" shows every subcategory of Food. If grouped search
    can't match group titles, use flat items labelled `Category ›
    Subcategory` with the category part muted, and say which you chose in
    the handoff.
  - Keyboard: the search input has focus when the menu opens; arrows move,
    Enter picks, Escape closes. The menu is wide enough for the longest
    label up to 320 px; longer labels ellipsize.
  - Renders only the visible options (check the gpui-component list is
    virtual; if it isn't, stop and report).
  - Tests: with 2 000 subcategories across 200 categories, typing a
    subcategory name shows it with its category and Enter picks it;
    typing a category name lists its subcategories; two same-named
    subcategories under different categories are both shown, each under
    its category; opening the menu builds only the visible items; the
    existing categorize / move / delete tests pass. Add a rule to
    `docs/ui-conventions.md` (*Interaction*): every category picker is
    searchable and shows the category with each subcategory. Tick the
    `plan.md` box.

### Last in M3

- [ ] **3.26 A scrollbar on everything that scrolls.** Most scrolling
  areas give no sign that there is more content or where you are in it.
  Every scrollable region shows a vertical scrollbar whenever its content
  is taller than its viewport, and none when it fits. Do this last in
  M3: 3.45, 3.52, 3.58 and 3.59 add or re-lay out scrolling regions.
  - **Review update (2026-11-11):** After 3.86 (measured layout), since the 16 px scrollbar gutter would break window-width arithmetic. Add the dashboard page scroll (3.45) and `DataList` (3.89, give the component a scrollbar so every list gets one) to the regions.
  - **Find every region at the time:** grep the UI code for
    `overflow_y_scroll`, `overflow_x_scroll` and `v_virtual_list`. Besides
    the lists below, that includes the Rules page list (`rules_scroll`),
    the dashboard page scroll (3.45), the rule matches panel (3.52) and
    the summary's month strip (3.30 already has its horizontal bar). 3.36
    removes the scrolling legend list.
  - Use gpui-base's `Scrollbar` (`gpui-base-0.9.0/src/scrollbar.rs`:
    `Scrollbar::vertical(&handle)`); it takes a `ScrollHandle`, and
    `VirtualListScrollHandle` implements `ScrollbarHandle` too. Don't
    hand-roll one.
  - **Always visible while the content overflows:** set
    `ScrollbarMode::Always` once, app-wide, on the theme's
    `ScrollbarTheme` at startup. The default (`Scrolling`) only shows the
    bar while scrolling, which is the behaviour being replaced.
  - **Virtualized lists** (category tree, rule list, To categorize queue):
    reuse the handle each list already keeps on `Shell` (`tree_scroll`,
    `rule_scroll`, `queue_scroll`, from 3.21).
  - **Plain scrolling divs** (`overflow_y_scroll` in `ui.rs`: the chart
    legend `legend-list`, the monthly summary `summary-rows`, the Transfers
    list `transfer-reviews`): give each a `ScrollHandle` kept as a `Shell`
    field (one per region, never created in `render`; same trap as 3.21)
    and `track_scroll` it.
  - **Popovers and dropdowns** that can outgrow their height: the Accounts
    filter popover gets a max height, scrolls, and shows a scrollbar; check
    that `Select` menus already show one for long lists (category pickers)
    and fix them if not.
  - **`DataTable`s** (transactions, import history) draw their own
    scrollbars (`scrollbar_visible`); confirm they follow the app-wide
    mode and show both when the content overflows.
  - **Placement:** the bar overlays the region's right edge inside its
    border (wrap the region in a `relative()` container and add the
    `Scrollbar` as an absolutely positioned sibling). Content reserves the
    bar's width (16 px) on the right so the thumb never covers amounts,
    counts or buttons.
  - Add a rule to `docs/ui-conventions.md` (*Layout*): every scrollable
    region has a visible scrollbar while it overflows, and new scrolling
    regions must add one.
  - Tests: for each region above, with enough rows to overflow, the
    scrollbar paints inside the region's bounds and the right-most content
    (amount, count or button) doesn't overlap it; with content that fits,
    no scrollbar paints; dragging the thumb (or `set_offset` on the handle)
    moves the content, and the offset survives an unrelated `cx.notify()`.

**Done when:** on the demo data the dashboard totals match
`expected.json`, filters compose, and every M3 task above is merged.

---

## M5 — Export / restore

Design: `plan.md` → *Export and restore*. The per-account, per-year master
CSVs already exist from 2.2; export copies them instead of rebuilding
anything. 5.1 and 5.2 are backend only and are the end of the backend queue
(see *Picking the next task*); 5.3 and 5.4 wait for M3.

- [ ] **5.1 Export, backend** (`financi-db/src/export.rs`, crates `zip`,
  `serde_json`).
  - **Review update (2026-11-11):** Snapshot on the reader connection or its own connection (3.77), so `VACUUM INTO` doesn't block the writer. The manifest gets an integer `format_version`; the app `VERSION` string is informational and never parsed.
  - `export(conn, root, dest, app_version)`: run the 2.2 self-check, then
    zip `financi.sqlite` (snapshot via `VACUUM INTO` or the backup API),
    `manifest.json`, and the `csv/` tree copied as-is (no rebuilding).
  - `manifest.json`: financi version (passed in by the app, from 3.39's
    `version.rs`), schema version, exported_at, and per file:
    account_key, institution, account name, kind, year, row count,
    SHA-256; plus the database file's SHA-256.
  - Tests: the zip holds the three parts; every manifest hash and row
    count matches the file; a missing year file is rebuilt by the
    self-check before zipping; hidden rows are in the CSVs.

- [ ] **5.2 Restore, backend.**
  - **Review update (2026-11-11):** Restore runs through the worker (close or reuse its connections; both have `busy_timeout`, 3.77).
  - `verify_export(zip)` checks every file against the manifest SHA-256s
    and refuses a schema version newer than this build. `restore(...)`
    backs up the current database to `financi.sqlite.bak` (backup API)
    and `csv/` to `csv.bak/`, restores into the live connection with
    rusqlite's backup API on the worker thread (no file swapping),
    migrates an older schema, and replaces `csv/` with the zip's tree.
    Expose it as a `DbWorker` job.
  - Tests: a tampered file is refused before anything changes; a newer
    schema is refused; an older one migrates; the `.bak` copies exist
    after a restore; a failure part-way leaves the live database usable.

- [ ] **5.3 Export and restore in the app.**
  - **Review update (2026-11-11):** After a restore, `Catalog` emits `DataChanged(all)`, every page resets its transient state (selection, open editors, armed confirms, queue pickers), and pending keyed jobs against the old data are superseded. Test: start a dashboard load, restore, and the late result is dropped.
  - File → Export… (⌘E) and the Imports page's **Export** button open a
    save dialog defaulting to `~/Desktop/financi-export-YYYY-MM-DD.zip`.
  - File → Restore…, the Imports page's **Restore** button and a
    "Restore from export…" link on the empty-state screen open a file
    dialog, then restore and refresh every view. Both say what happened in
    a toast; errors stay until closed.
  - Remove the "… is not available yet." toasts (`Shell::export_data`
    and its Restore twin). UI tests for both flows
    (stub the dialogs), and update the *Imports* row of
    `ui-conventions.md`.

- [ ] **5.4 Acceptance tests** (all must pass):
  - Overlapping re-import adds 0 rows (exists; keep it) and leaves the
    master CSVs byte-identical.
  - Totals match `fixtures/demo/expected.json` (from 3.1).
  - **Export round-trip, byte-exact:** import every demo file, hide a row,
    export, then import the zip's CSVs into a fresh database whose accounts
    use the manifest's account keys → same multiset of
    `(account_key, row_hash)` and identical row bytes, and the fresh
    database's master CSVs are byte-identical to the zip's; re-importing into
    the source database adds 0 rows; restore from the zip gives an identical
    database (same rows, categories, rules, locks, hidden flags) and `csv/`.

**Done when:** 5.1–5.4 pass. Then do the final task below.

---

## Final — performance at scale (last of all)

Runs after M5, when every other task is done (user decision, 2026-11-11).

- [ ] **3.55 Fast at real scale: hundreds of rules, dozens of
  categories.** The user has hundreds of rules and dozens of categories
  and is worried about performance (2026-11-11). The dev seed (18 rules,
  458 rows) hides scale problems. `CompiledRules::pick` loops over **every
  rule for every row**, and a rule change runs that loop twice: once in
  `reapply_rules`, and again in `rule_use_stats` (3.58) inside `refresh`. At
  1 000 rules × 100 000 rows (10 years at the user's ~10 000 a year)
  that's ~100 M checks per pass, likely around a
  second or more. The live preview (`count_rule_matches`) scans every row
  on each keystroke. **Runs last** (user decision, 2026-11-11): after M5 and every other
  task. Earlier tasks measure on the dev seed and a throwaway local seed;
  the committed scale seed and the timed budgets land here.
  - **Review update (2026-11-11):** Added 2026-11-11 (review): **table paging** — `list_transactions_filtered` has no `LIMIT`, so "All time" loads every row on each change; page the table with `TableDelegate::has_more` / `load_more` plus a `COUNT(*)`. Timed tests for pairing at 100 000 rows (3.73) and the reader connection (3.77). `RegexSet::new` fails as a whole when one pattern or the set exceeds the size limit: fall back to per-regex matching for the patterns that don't fit, test with 200 `\w+`-heavy rules, and keep 3.72's Unicode case folding and `chars().count()` lengths.
  - **Scale fixture:** a `seed_large` (dev-seed feature) and a
    `FINANCI_DEV_SEED=large` switch. It builds **1 000 rules** (80 %
    substring, 20 % regex, overlapping on purpose), **80 categories**
    (15 parents), and **100 000 transactions** over 10 years (the user has ~1 800 today
    and expects ~10 000 a year) with realistic
    merchant strings, all deterministic. It uses only synthetic data.
  - **Budgets** (release build, on the large seed), as `#[ignore]` perf
    tests runnable with `cargo test --release -- --ignored perf_`, which
    print their times:
    - `reapply_rules` < 150 ms;
    - `rule_use_stats` < 150 ms;
    - `count_rule_matches` < 30 ms;
    - the full `refresh` query set < 250 ms;
    - main-thread apply of a refresh result < 16 ms;
    - opening the Categories and Rules pages < 16 ms per frame;
    - a dashboard filter change shows its result within 100 ms (from
      3.56);
    - no frame over 16 ms while hovering, scrolling or typing, on the
      one-year seed (from 3.57);
    - the Rules page with 1 000 rules scrolls and filters under 16 ms a
      frame (from 3.58).

    Debug-build versions of the DB budgets (×10) run in the normal test
    suite on a 10 000-row / 300-rule seed (one year) so regressions fail CI.
  - **Matching in one pass per row:** in `CompiledRules`, replace the
    per-rule loop with an **Aho-Corasick** automaton over the lowercased
    substring patterns (`aho-corasick` crate, already in the tree via
    `regex`; `MatchKind::Standard` with overlapping search to collect
    every match) and one **`RegexSet`** for the regex rules (case-
    insensitive). Each row is scanned once per kind; the winner is the
    longest match, ties to the lowest id, substring before regex, exactly
    as today. Invalid regexes stay excluded. The existing core tests are
    the contract; add a property test that the new `pick` equals the old
    per-rule loop on random rules and descriptions.
  - **One pass per rule change:** have `reapply_rules` return the per-rule
    use counts it computed, and cache them on the shell until the next
    rule or import change, so `refresh` doesn't recompute them. If that
    isn't enough, store the deciding rule: add a nullable
    `transactions.category_rule_id` column in a new migration, set by
    `reapply_rules` and at import. Uses then becomes a `GROUP BY`, and
    3.52's "decides" set becomes a query. Only add the column if the
    budgets need it.
  - **Live preview:** debounce `preview_rule_match` by 150 ms, and keep
    the in-flight task cancellable, as now.
  - **UI at scale:** with 1 000 rules and 80 categories, check that the
    rule lists, tree and pickers stay virtual or lazy (3.12, 3.19, 3.21).
    Category pickers with 65 children stay usable (3.59 made
    them searchable; check it holds at this size).
  - Commit message: the before / after numbers for every budget.

---

## After v1 (not now)

v1.1 spend-over-time line + top merchants · v1.2 flags (large, new merchant,
FX/fees) · v1.3 recurring/subscriptions · default categories and merchant
rules on first run (`plan.md` → *Categorization*; moved out of v1
2026-11-11) · more institutions (first widen the parser trait: `detect`
sees the file, not just the header row; the parser controls header /
preamble rows; it supplies its institution's display name, replacing the
match in `flow::format_label`) · CSV export of filtered rows. Real Simplii
Visa sample → revisit 4.1.

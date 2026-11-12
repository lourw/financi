# GPUI: target architecture, coding practices, pitfalls

Read this before any task that changes the GPUI app (`crates/financi/src/ui*`). Backend tasks skip it.

## GPUI best practices: the target, step by step

The UI grew as one `Shell` entity (~200 fields, every page's state) that
re-queries every page after every action. Zed-style GPUI is the target
(user decision, 2026-11-11). Each step below is the GPUI rule, what
changes, and the task that does it, in implementation order. When a step's
task merges, the rule becomes binding (see *Coding practices*).

1. **Small files, one per page** (3.62, done). `ui.rs` / `ui_tests.rs` became
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
  explicitly (see `ui/tests/mod.rs`).
- UI tests talk to a real `DbWorker` thread: call
  `cx.executor().allow_parking()` and poll with the `wait_until` helper.
- `BarChart` has no click handler. `SelectState::set_items` and
  `set_selected_index` need a `&mut Window`, so async updates use
  `cx.spawn_in(window, …)` + `this.update_in(cx, |this, window, cx| …)`.
- All DB work goes through `DbWorker` via the `on_db` helper in
  `ui/widgets.rs`.
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

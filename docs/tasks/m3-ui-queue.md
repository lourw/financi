# M3 — UI queue: architecture and GPUI best practices

Do these in the order listed (`docs/tasks.md` → *Picking the next task*). Each task is one step of `docs/tasks/gpui.md` → *GPUI best practices*; read that file first.

## UI queue: architecture and GPUI best practices (in this order)

Steps 1–15 of `docs/tasks/gpui.md` → *GPUI best practices*, task by task.

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


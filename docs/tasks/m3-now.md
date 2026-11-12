# M3 — in-flight features

Features the paused run left half done. Finish each in its existing worktree before starting the queues. Index and order: `docs/tasks.md`.

## Now: finish the in-flight features (3.34, 3.53 + 3.42)

- [x] **3.34 To categorize looks cleaner.** The queue is loose rows on the
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

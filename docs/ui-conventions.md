# UI conventions

financi's design system: how every page looks and behaves. `plan.md` says
what to build and `docs/tasks.md` gives the order (GPUI pitfalls are in
`docs/tasks/gpui.md`). A
task that names a specific layout wins for that task; update this file in
the same commit. Where the code differs from a rule here, the rule is the
target and a task brings the code in line.

## 1. Principles

1. **Data first, chrome last.** A desktop tool for one person who knows
   their own money. Numbers and rows get the space. No hero headers, cards
   in cards, or icons that do nothing.
2. **One window, one filter state.** Every dashboard view reads the same
   `ViewFilter` plus the selected category and month. Clicking a number
   narrows the filter. Nothing opens a second window, side sheet or modal.
3. **Use the gpui-kit theme as it is.** Colors, radius and fonts come from
   `cx.theme()` (default shadcn "neutral", light and dark). Never hardcode
   `rgb(...)`/`hsla(...)` in views. If a color you need isn't in the token
   table, ask.
4. **Every state is designed.** Each region has an empty, a loading (keep
   the old content, dimmed; never flash blank) and an error state.

## 2. Tokens

### Spacing

gpui spacing helpers only, in these steps:

| Step | Helper | Use |
|---|---|---|
| 4 px | `gap_1` | icon next to its label, chip contents |
| 8 px | `gap_2` | controls in one group (a filter row, buttons in a row) |
| 12 px | `gap_3`, `px_3` | between page regions; list row and heading padding |
| 16 px | `p_4` | page padding; padding inside a content panel |

### Heights

| Element | Height |
|---|---|
| Control (input, select, button, date picker) | 32 px (gpui-kit default size) |
| `DataList` heading row; filter row | 32 px |
| `DataList` group header | 36 px |
| `DataList` row | 40 px |
| `DataTable` row | 32 px (gpui-kit default) |

Button sizes: page header and form actions at default size; panel
title-row actions `small`; buttons inside a list or table row `xsmall`
`ghost`, vertically centered with at least 4 px clear above and below.

### App-wide widths

- Sidebar ~210 px. Minimum window 1200 × 720: every page fits it with
  nothing clipped, no wrapping and no horizontal page overflow.
- Page-specific widths (column widths, picker widths) are constants next
  to the page's code, not tokens.

### Typography

- The theme's system UI font. Base 16 px, but **data regions use
  `text_sm`** (14 px): lists, tables, summary, legend.
- Page title `text_lg` `font_semibold`. Region and panel titles
  `text_sm` `font_semibold` `foreground`; no larger headings inside a
  page. The sidebar's app name is `font_semibold`.
- Secondary text (counts, summaries, captions, list headings):
  `text_xs` or `text_sm` in `muted_foreground`. Field labels, where
  allowed (*Labels*), are `text_xs` muted.
- Amount columns are right-aligned. If system-font digits don't line up,
  switch amount cells (only those) to `mono_font_family`. Rule patterns
  are shown in `mono_font_family`.
- Sentence case ("Import CSV"). Buttons are verbs.

### Color tokens

| Meaning | Token | Where |
|---|---|---|
| Text | `foreground` | everything by default, including money out |
| Secondary text | `muted_foreground` | captions, summaries, Uncategorized, hidden rows, 0 counts |
| Lines | `border` | panel borders, row dividers, section dividers |
| Main action | `primary` button | at most one visible per page or panel (*Buttons*) |
| Money in | `success` | positive amounts in tables |
| Problems | `danger` | errors, armed confirms, negative Net |
| Hover / selection | `list_hover` / `list_active`, `table_active` | hovered row; selected row, legend item, summary cell |
| Group header fill | `muted` | `DataList` group headers, the one allowed fill |
| Toast surface | `popover` | toast cards |
| Needs action | sidebar count pill | never a banner or filled box |
| Category colors | `blue`, `cyan`, `magenta`, `yellow`, `green`, `red`, then their `_light` variants | see below |

Money out is plain `foreground`: spending is the normal case and a table
of red reads as an alarm. Expected behavior isn't a warning ("Rows you
already have are skipped" is plain muted text).

### Category colors

Each category has one color, the same everywhere: its bar
(`BarChart::fill`), its legend swatch, its swatch in the summary, the
category tree and lists.

- Palette order: `blue`, `cyan`, `magenta`, `yellow`, `green`, `red`, then
  `blue_light` … `red_light` (12). Red and green come last because they
  also mean danger and money in.
- **Stable, not by rank:** `index % 12` by position among siblings sorted
  by category id, so a color survives range changes. Drilling in gives the
  subcategories colors from the start of the palette.
- Uncategorized is always `muted_foreground`, never a palette color.
- Shown only as a `Swatch`. Never color text with category colors.

## 3. Components

*(Built in task 3.89 in `crates/financi/src/ui/widgets.rs`; until then,
match this by hand.)* Pages compose these; don't restyle them per page.

- **`Panel`** — `border_1` `border`, `rounded(theme.radius)`, no fill
  (background `background`). Optional title row: `text_sm`
  `font_semibold` title on the left, `small` actions on the right. A
  content panel pads `p_4`; a panel holding a `DataList` has no padding
  and the list runs edge to edge.
- **`PageHeader { title, summary, actions }`** — the first row of every
  page except Dashboard: `text_lg` semibold title, a muted summary
  ("N rules · M unused", counts via `flow::count`), actions right-aligned.
- **`DataList`** — a heading row plus a virtual list (`v_virtual_list`;
  per-row widgets built only for visible rows). Columns are typed
  `Fixed(px) | Flex` with an alignment, shared by heading and rows so they
  always line up; a `Flex` text column gets `min_w_0` and ellipsizes with a
  tooltip holding the full text.
  - Heading row 32 px, `text_sm` muted, `px_3`, bottom `border`. Sortable
    headings toggle the sort on click.
  - Rows 40 px, `px_3`, bottom `border` divider (none after the last),
    `list_hover` on hover, `list_active` when selected.
  - Optional group headers: 36 px on `muted` fill, chevron to collapse,
    the group name semibold, a muted count, and group sums under their
    columns.
  - Optional actions column at the right: `row_action_button`s revealed
    on row hover or focus (or while the row's editor is open). The column's
    width is always reserved so nothing shifts.
- **`FilterRow`** — one 32 px row, `gap_2`, controls vertically centered.
  Controls have no visible labels; each names itself by placeholder or
  tooltip. Search is `flex_1` with a `min_w`, a leading search icon and a
  placeholder ("Search transactions"). Rarely used toggles go in an
  **Options** popover (`list-filter` icon + "Options", reading
  "Options · N" when N are on). A muted count or summary sits last,
  `ml_auto`, at the page's right edge.
- **`InlineForm`** — a bordered row opened in place, right under the
  header or panel title row whose action opened it. Placeholder-labelled
  controls, then Cancel and the primary action at the right end. Escape,
  Cancel, a successful submit and leaving the page close it; opening
  another form closes the open one (one form open at a time).
- **`Swatch`** — 10 × 10 square, 2 px radius, the category color.
- **`TypeTag`** — small outline tag (`text_xs`, `border`, radius), e.g.
  "text" / "regex", "system", the import preview's institution.
- **`control_slot(el)`** — a 32 px tall, `items_center`, `flex_shrink_0`
  slot that lines an unlabelled control (a switch, a checkbox) up with
  labelled 32 px controls in the same `items_end` row.
- **`row_action_button(icon, tooltip)`** — `xsmall` `ghost`, icon only,
  with a hover tooltip naming the action. Two-click destructive buttons
  widen to show "Click again to …" when armed.

## 4. Patterns

### Labels

- **Filter rows and inline forms carry no visible labels.** The
  placeholder or a hover tooltip names the control, and every unlabelled
  control sets an `accessibility_label`.
- **Visible labels** (`text_xs` muted, above the control) only in
  multi-field forms where a filled value hides the placeholder: the rule
  editor's Pattern, the New category form's name and "Category type".
- Region titles name data panels (*Spending by category*, *Monthly
  summary*). Don't add a title that repeats a placeholder.

### Buttons

- Never end a button label in "…". Actions carry a Lucide icon left of the
  label (`plus` New, `pencil` Rename/Edit, `trash-2` Delete, `file-up`
  Import CSV, `undo-2` Undo, `download` Export, `archive-restore` Restore).
  Menu items keep "…" when they open a dialog.
- **At most one primary visible per page or panel.** A header action
  shows outline while its inline form is open (the form's submit is the
  primary). Row-level "ready" buttons are outline, never primary.
- Order in a row of actions: Cancel, then the primary last at the right
  end (macOS order).

### Tables and lists

- `DataTable` (dashboard transactions, import history) and `DataList` both
  use row dividers and `list_hover`; no stripes.
- `DataTable` and `DataList` take `flex_1` (or a fixed height inside a
  scrolling page) and scroll inside themselves. Flexible columns absorb
  the window width so a table never overhangs the page.
- Row actions in the transactions table are a right-click context menu
  (Change category, Hide, Unhide, **Show rule**), no buttons in rows.
  Show rule opens the Rules page on the deciding rule; with none it is
  disabled with the reason ("Set by hand", "Card payment (built in)", "No
  rule matches"). Lists that manage their rows use `row_action_button`s.

### Destructive actions

Two-click confirm inside the control: the first click arms it and it reads
"Click again to …" in `danger`; the second acts. Escape, leaving the page
or changing the selection disarms it. No modals, no native alerts.

### Feedback

- Every click shows its effect, or a loading/disabled state on the
  control, within one frame (`plan.md` → *Responsiveness*).
- **Toasts, never inline status lines.** Messages ("Categorized
  MERCHANT", "Export is not available yet.", errors) are gpui-base toasts
  over the window's bottom-right corner. No page renders a status line in
  its content column.
- Success and info toasts dismiss after ~4 s; errors stay until closed. A
  burst stacks (newest in front), never replaces.
- Card: one line of `text_sm` on `popover`, `border`, `radius`; error text
  `danger`, else `foreground`. Clicking the card dismisses it.
- Errors say what happened and what to do next ("Pick an account before
  importing").

### States

| Region | Empty | Loading | Error |
|---|---|---|---|
| Whole window (no data) | Centered: "Drop an Amex or Simplii CSV export here, or press ⌘O" plus a Restore link (5.2) | none | none |
| Chart / summary | "No spending in this range" (muted, centered) | previous content, dimmed | `danger` line inside the panel |
| Table / list | "No transactions match these filters" + "Clear filters" link (or the list's own message), centered in its panel | previous rows | `danger` line above it |

### Keyboard

- Escape cancels the innermost transient UI of the focused page: editor,
  inline form, armed confirm, then selection (chip, then drill-down on the
  dashboard). Bind it through GPUI key contexts and actions on the page,
  never a global key handler.
- Enter submits the focused form.
- ⌘O import, ⌘E export.

### Filtering and hints

- **Click to filter.** Legend items, summary cells and summary row
  headers set the shared filter; the selected item is `list_active`.
  Clicking it again, or Escape, clears it.
- **Hints go in hover tooltips, never captions** (`.tooltip(...)`, needs an
  `id`). Summary cell: `Shopping · Oct 2026 · 14 transactions` over "Click
  to show them in the table". Chart bar and legend item: the full name over
  `amount · share` (`Restaurants · $642.15 · 31.4% of Food`).
- **Filter chips:** each active narrowing filter (category, month,
  search, Uncategorized only) is a chip with a ×, above the table. Filters
  the filter row already shows (dates, accounts, transfers) aren't chips.
  Once 3.46 lands, chips sit under the pinned filter row next to Clear all.

### Scrolling and overflow

- Every scrollable region shows a scrollbar while its content overflows,
  visible without scrolling first, and none when it fits (3.26).
- **Text never pushes its container wider.** Any flex child holding text
  gets `min_w_0` plus `overflow_hidden`/`text_ellipsis`; ellipsized text
  carries a tooltip with the full text. Amount and percent columns have
  fixed widths; selects are wide enough for their longest label.

### Formats

- **Money:** `$1,234.56`. Tables are signed: `-$86.42` out, `$2,105.00` in
  (`success`). Chart, tooltips and summary show positive spend; Net is the
  one signed summary row.
- **Shares:** one decimal, `22.4%`. Over chart bars: whole percent
  (`18%`), one decimal only below 1 % (`0.4%`).
- **Dates:** `2026-11-06` in tables; summary months `Sep 2026`; ranges
  `Sep 1 – Nov 30, 2026`.
- **Accounts:** "Institution · Account" everywhere (`Amex · Cobalt`).
- **Categories:** `Food › Groceries`, the category muted.
  Uncategorized is muted text; in the legend and summary it is always the
  last category row, whatever its amount.

### Copy

Say **category** and **subcategory**, never parent, child or children, in
every user-visible string (labels, placeholders, buttons, tooltips, errors,
including `financi-db` errors the UI shows). Code identifiers and DB
columns keep their names (`parent_id`).

## 5. Pages

Every page has `p_4` padding and `gap_3` between regions. Opening a file
(drop anywhere, ⌘O, Import CSV, `FINANCI_DEV_OPEN`) switches to Imports
with the preview open. The selected page isn't persisted; launch opens
Dashboard.

**Sidebar.** gpui-kit `Sidebar`, ~210 px, `sidebar` background, `border`
right, not collapsible. Header: the app name only; no buttons. Items
(`SidebarMenuItem`, Lucide icon, `.active` on the current page): Dashboard;
*Organize*: To categorize, Categories, Rules, Transfers; *Data*: Imports.
Items with work waiting (uncategorized transactions, open transfer reviews)
show a count pill in their `suffix`, hidden at 0; there are no banners.
Footer, `text_xs` muted, ellipsized with full-text tooltips: transaction
count, database path (tooltip "Database: …"), build version (`git
describe`).

**Dashboard.** No `PageHeader`; the first row is a `FilterRow`: Date range
select, From/To pickers when Custom (shrinking first, to a floor), Accounts
popover ("All accounts" / "N of M accounts"), Search, Options (Include
transfers, Uncategorized only, Show hidden), then the transaction count.
It must fit 1200 × 720 with Custom open. Below: the chart `Panel`
(*Spending by category*: `BarChart`, share labels over bars, band labels
shortened and thinned to fit the live width, full names in legend and
tooltips) beside the *Monthly summary* `Panel` (~45/55, both 340 px; the
summary's name column and Total are pinned while only month columns
scroll sideways on one shared handle, landing on the latest month). Then
filter chips and the transactions `DataTable`, where every column sorts
(text A→Z first, Date and Amount newest/biggest first; the sort survives
filter changes). The legend moves under the chart (3.36); chart, summary
and table stack in a scrolling page under a pinned filter row (3.45);
every filter applies to all three views, with Clear all (3.46); filter
changes apply at once (3.56).

**To categorize.** `PageHeader` ("N transactions · M merchants") and one
bordered `Panel` filling the rest of the page, holding the Uncategorized
queue as a `DataList` (32 px heading row, 40 px rows with dividers and
`list_hover`): merchant (`Flex`, normal weight), transactions and total
(right), dates (`text_sm` muted), a category picker and a `small` outline
Categorize button, disabled until that row's picker has a category.
Choosing a category adds a rule. Pickers become searchable (3.59).

**Categories.** `PageHeader` ("N categories · M subcategories", **New
category** opening an `InlineForm`: name, "Category type", Cancel, Add
category). Two `Panel`s side by side: the category tree (~300 px; rows
with `Swatch`, name — categories semibold, subcategories indented — the
`system` `TypeTag` and a muted count; `list_active` / `list_hover`) and the
selected category's detail (`flex_1`; with none selected it prompts
"Select a category to see its rules and settings."). The detail's title
row holds the name and `small` actions (Rename; Add subcategory, Move,
Delete each opening an `InlineForm` under it, 3.51); sections are split by
`border_t_1` dividers. Its Rules section is a `DataList` of the category's
rules (a category's include its subcategories'; "No rules for this
category yet" when empty) with an add-rule row (Pattern, regex switch in a
`control_slot`, Add rule) showing a live "matches N transactions" count.
Where a rule goes follows the selection (3.53): a selected subcategory is
the target and the row carries no picker; a selected parent picks among
its own subcategories in a "Subcategory…" `Select` — their own names, no
default pick; the trigger fits the longest name up to 200 px, ellipsizing
with a hover tooltip, and the open menu grows to 320 px so names are
never cut off; a parent without subcategories shows a muted "Add a
subcategory first" in place of the row. Edit swaps a row for a two-line
editor (Pattern labelled at full width, then the switch and Cancel /
Save; 3.48), one at a time; the editor has no category picker — Save
keeps the rule's category, and a muted "To move rules to another
category, use the Rules page." sits under a non-empty rule list. Delete
is a two-click confirm. Selecting a category or leaving the page closes
the editor and disarms. Planned: a Uses column (3.49), a focused rule's
transactions under the list (3.52).

**Rules.** `PageHeader` ("N rules · M unused", **New rule** opening the
add-rule row in place). `FilterRow`: Search ("Search rules or
categories", focused on open; matches pattern and category names), a Type
select, Options (Test a description, Unused only). A `Panel` with a
grouped `DataList` (group per category › subcategory, collapsible, with
summed Uses and Total): checkbox, Pattern (mono), Type (`TypeTag`), Uses,
Total, actions (Edit, Delete); sortable by Pattern, Uses, Total. Test a
description turns Search into a pasted bank description and lists only the
matching rules in matcher order, the winner first and marked "wins".
Checked rows replace the heading row with a Move bar ("N selected", a
subcategory picker, Move, Clear). Show rule lands here: filters cleared,
Search set to the pattern, the row scrolled into view in `list_active`
until the user searches or leaves. The look, Total and the Move bar
arrive with 3.58.

**Transfers.** Open transfer reviews, each row with its candidates and
"Not a transfer".

**Imports.** `PageHeader` actions: **Import CSV** (primary), **Undo**
(two-click, newest import, disabled with no history), **Export**,
**Restore** (live in M5; until then a toast says so). The import preview
(while a file is open) on top, then the history `DataTable` (time, file,
account, rows added, dates). The newest history row carries an in-row Undo
sharing the header Undo's confirm state, tooltip "Undo this import"; it
gains the label "Undo" (3.37).

**Import preview.** A two-line `Panel`. Line one: file name, the
institution, then Cancel and the primary import action ("Import 32
transactions") at the right end. Line two, muted with the key number in
`foreground`: `Into [account ▾] | 32 new of 84 rows · 52 already
imported, skipped · Oct 1 – Dec 4, 2026`. No stat blocks. The institution
becomes a `TypeTag` (3.61).

## 6. Decisions

Confirmed with the user. Don't reopen without asking.

| # | Date | Question | Decision |
|---|---|---|---|
| D1 | 2026-11-07 | Category chart | gpui-kit `BarChart` with a clickable legend, beside the summary; bars aren't drawn into the legend. 3.36 moves the legend under the chart and 3.45 puts chart and summary on their own rows. |
| D2 | 2026-11-07 | Category label in tables | `Category › Subcategory`, the category muted. |
| D3 | 2026-11-07 | Money going out | Plain `foreground`; only money in is colored. |
| D4 | 2026-11-07 | Undo last import | Imports page (header and newest row) and File → Undo last import. |
| D5 | 2026-11-07 | Category colors | One stable color per category (*Category colors*). |
| D6 | 2026-11-07 | Hints | Hover tooltips, never captions. |
| D7 | 2026-11-07 | Panels | Bordered, never filled; compact two-line import preview. |
| D8 | 2026-11-07 | Navigation | Sidebar of pages: Dashboard, To categorize, Categories, Rules, Transfers, Imports. No side sheets, modals or banners; counts in the sidebar. |
| D9 | 2026-11-07 | Import button | Not in the sidebar: Imports header, ⌘O, drop anywhere. |
| D10 | 2026-11-11 | Labels | Filter rows and inline forms name controls by placeholder or tooltip; visible labels only where a filled value hides the placeholder. |
| D11 | 2026-11-11 | "…" on buttons | Never; an icon left of the label instead. Menus keep "…" for dialogs. |
| D12 | 2026-11-11 | Primary buttons | At most one visible per page or panel; row-level buttons are never primary. |
| D13 | 2026-11-11 | Table rows | Dividers and hover, no stripes. |
| D14 | 2026-11-11 | Account names | "Institution · Account" everywhere. |

## 7. Testing UI conventions

- UI tests (`ui_tests.rs` style) check behavior, not pixels: which rows
  show, which filter is active, what text a state shows, that a region
  paints inside its page and fits 1200 × 720.
- Click through rendered controls with a click helper (a simulated mouse
  click at the control's painted bounds) where possible, rather than
  calling the handler directly.
- To assert that nothing happened, first wait until the DB worker has no
  pending requests (`wait_until` on its idle state), then assert.
- When a convention here changes, update this file in the same commit.

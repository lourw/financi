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
  `text_sm`** (14 px): lists, tables, summary. The chart's legend chips
  are smaller: `text_xs` (12 px, 3.36).
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

Each category has one color, the same everywhere: its donut slice
(`PieChart`, 3.99) or stacked bar (`BarChart::fill`), its legend swatch,
its swatch in the summary, the category tree and lists.

- Palette order: `blue`, `cyan`, `magenta`, `yellow`, `green`, `red`, then
  `blue_light` … `red_light` (12). Red and green come last because they
  also mean danger and money in.
- **Stable, not by rank:** `index % 12` by position among siblings sorted
  by category id, so a color survives range changes. Drilling in gives the
  subcategories colors from the start of the palette.
- Uncategorized is always `muted_foreground`, never a palette color.
- Shown only as a `Swatch`. Never color text with category colors.

## 3. Components

Pages compose these; don't restyle them per page. They live in
`crates/financi/src/ui/widgets.rs`.

- **`Panel`** — `border_1` `border`, `rounded(theme.radius)`, no fill
  (background `background`). Optional title row: `text_sm`
  `font_semibold` title on the left, `small` actions on the right. A
  content panel pads `p_4`; a panel holding a `DataList` has no padding
  and the list runs edge to edge. On the Dashboard the panels are dock
  frames (3.100): a hand-written `DockAreaRenderer`/`TabGroupRenderer`
  skin (`ui/dashboard/dock.rs`) draws this same look, with the title row
  doubling as the drag handle (`cursor_grab`).
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
- **Category pickers are searchable** (3.59) — every `Select` that picks
  a category or subcategory (To categorize rows, the Rules Move bar and
  add-rule row, the Categories Move and Delete forms) opens with its
  search box focused; arrows move, Enter picks, Escape closes. The
  sub-category pickers list sub-categories grouped under their category,
  and the query matches category **and** sub-category names case-
  insensitively, so every result shows with its category and same-named
  sub-categories are told apart. The menu is wide enough for the longest
  label up to 320 px; longer labels ellipsize. Items come from
  `Catalog::cat_choices` (`ui/category_picker.rs`); pickers resolve picks
  by the item's id value.
- **`Swatch`** — 10 × 10 square, 2 px radius, the category color.
- **`TypeTag`** — small outline tag (`text_xs`, `border`, radius), e.g.
  "text" / "regex", "system", the import preview's institution.
- **`control_slot(el)`** — a 32 px tall, `items_center`, `flex_shrink_0`
  slot that lines an unlabelled control (a switch, a checkbox) up with
  labelled 32 px controls in the same `items_end` row.
- **`row_action_button(icon, tooltip)`** — `xsmall` `ghost`, icon only,
  with a hover tooltip naming the action. Two-click destructive buttons
  widen to show "Click again to …" when armed.
- **`StackedBarChart`** (3.93) — monthly stacked bars from plain divs
  (gpui-kit's `BarChart` can't stack). One column per month; the column's
  bar is a `flex_grow` share of the plot (`total / max_total`, no
  measurement), each segment a div grown by its cents and filled with its
  category color (Uncategorized `muted_foreground`). Segment order is the
  summary's row order in every bar, so colors line up month to month. A
  per-segment hover tooltip reads `Category · Oct 2026` over
  `$412.30 · 22.4%`; segment and month-label clicks are callbacks. Below a
  12 px bar floor the strip scrolls sideways from the latest month. Named
  after its gpui-kit siblings so it can be upstreamed later.

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
  MERCHANT", "Exported to …", errors) are gpui-base toasts
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
  headers set the shared category filter, summary month cells and month
  headings set the month; every dashboard filter (dates, accounts,
  search, options, category, month) narrows chart, summary and table
  together. The selected item is `list_active`, slices and bars of other
  categories dim to 35% (a drilled-into selection's own slices and bars
  never dim), and clicking again, Escape or Clear all clears it.
- **Hints go in hover tooltips, never captions** (`.tooltip(...)`, needs an
  `id`). Summary cell: `Shopping · Oct 2026 · 14 transactions` over "Click
  to show them in the table". Chart slice and legend item: the full name over
  `amount · share` (`Restaurants · $642.15 · 31.4% of Food`).
- **Filter chips:** each active narrowing filter (category, month,
  search, Uncategorized only) is a chip with a × in a row under the
  pinned filter row, beside **Clear all** — a ghost button with an X icon
  before the transaction count. Both appear only while some filter is
  set; Clear all resets every dashboard filter, the drill-down included.
  Filters the filter row already shows (dates, accounts, transfers,
  Show hidden) aren't chips.

### Scrolling and overflow

- Every scrollable region shows a scrollbar while its content overflows,
  visible without scrolling first, and none when it fits (3.26).
  `ui::init` sets the theme's scrollbar mode to `Always` app-wide, so new
  regions add no local `.mode(…)`. The bar overlays the region's right
  edge inside its border, and content reserves the bar's 16 px
  (`SCROLLBAR_GUTTER`) on the right so the thumb never covers amounts,
  counts or buttons. A `DataList` (3.89) gets the bar and the gutter by
  itself; any other scrolling region reuses gpui-base's `Scrollbar` with
  a handle kept as state, never created in render.
- **Text never pushes its container wider.** Any flex child holding text
  gets `min_w_0` plus `overflow_hidden`/`text_ellipsis`; ellipsized text
  carries a tooltip with the full text. Amount and percent columns have
  fixed widths; selects are wide enough for their longest label.

### Formats

- **Money:** `$1,234.56`. Tables are signed: `-$86.42` out, `$2,105.00` in
  (`success`). Chart, tooltips and summary show positive spend; Net is the
  one signed summary row.
- **Shares:** one decimal, `22.4%`.
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

Every page has `p_4` padding and `gap_3` between regions (on the
Dashboard the dock draws that `gap_3` as 12 px gutters between panel
frames, 3.100). Opening a file
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
transfers, Uncategorized only, Show hidden), then the transaction count,
then — right-aligned, after the count — the layout **Reset** icon button
(Lucide `layout-dashboard`, tooltip "Reset layout", 3.101).
It must fit 1200 × 720 with Custom open. Below sits the **dock**
(3.100): a vertical split of two rows — top row Chart | Monthly bars,
bottom row Summary | Transactions, all halves 50 % — with the filter row
pinned outside it. Panels keep the `Panel` look; the title row is the
drag handle (`cursor_grab`): dragging a panel onto another panel's title
tabs them (a `text_xs` tab strip appears in the frame — tabs only when a
group holds two or more panels), and onto its content half splits it.
Dividers resize both neighbours; gutters between panels are 12 px
(`gap_3`, "between page regions") with the divider's hit area in the
gutter and a `border`-colored line on hover or drag. Minimum panel size
240 × 160, the table 320 × 200; everything still fits 1200 × 720. There
is no page scroll: each panel scrolls inside its own frame when its
content overflows (3.100). The arrangement is **saved**: 500 ms after
the last move or resize it is written to the `dashboard.layout` settings
key and restored at launch; a stored layout that is corrupt, of another
version, or missing a panel falls back to the default 2×2. The Reset
button (and *View ▸ Reset dashboard layout*) puts the default back and
saves that (3.101). Top-left, the chart `Panel`
(*Spending by category*: donut `PieChart` — 3.99, which replaced the
`BarChart`, its share labels over bars and thinned band labels; the range
total sits in the hole, `text_lg` semibold amount over `text_xs` muted
"Expense", the drilled parent's name and total while drilled; inner
radius 60 % of the outer, the largest 8 categories plus a muted "Other"
slice whose tooltip lists its members and whose click does nothing,
categories netting ≤ 0 get no slice; full names in legend and tooltips).
The legend is a wrapping chip row (10 px swatch + name `text_xs`
ellipsized at 120 px, click drills, tooltip `amount · share`,
`list_active` selected, "← All categories" first while drilled, muted
"+N more" chip whose tooltip lists the hidden names) laid out from the
panel's measured content area (3.86): wider than tall, the donut sits
left as a square of the area's height with the legend wrapping to its
right as far down as the height fits; otherwise the legend wraps under
the donut, at most two lines. The donut never drops below 120 px across;
under that the panel shows the legend alone). Top-right, the
*Monthly spending by category* `Panel` (3.93: `StackedBarChart`,
one stacked bar per month from the summary — parents, `Uncategorized` and
`Transfers` when included, the drilled-in parent's children when drilled;
compact-amount axis and gridlines, month labels thinned to fit, a 3.36
chip legend (≤ 2 lines, "+N more") whose chips select the category alone,
segment clicks selecting category + month like a summary cell, month
labels the month alone, 3.46 dimming, sideways scroll from the latest
month at a 12 px bar floor, "No spending in this range" when empty).
Bottom-left, the *Monthly summary* `Panel` (3.45: the summary's name
column and Total are pinned while only month columns scroll sideways on
one shared handle, landing on the latest month; 3.100: its rows scroll
vertically inside the frame). Bottom-right, the transactions
`DataTable`, where every column sorts (text A→Z first, Date
and Amount newest/biggest first; the sort survives filter changes).
The filter chip row and Clear all sit directly under the filter row
(3.46); every filter applies to all four panels, and filter
changes apply at once (3.56).

**To categorize.** `PageHeader` ("N transactions · M merchants") and one
bordered `Panel` filling the rest of the page, holding the Uncategorized
queue as a `DataList` (32 px heading row, 40 px rows with dividers and
`list_hover`): merchant (`Flex`, normal weight), transactions and total
(right), dates (`text_sm` muted), a Rule cell — a pattern `Input`
(`flex_1 min_w_0`, placeholder the merchant; blank means "use the
merchant") and an icon-only regex `Switch` in a `control_slot` (tooltip
"Regex"), off by default (3.95) — a category picker and a `small` outline
Categorize button, disabled until that row's picker has a category.
Choosing a category adds a rule: the pattern is what the row's input
holds, regex when its switch is on; Enter in the input is the row's
Categorize, and a pattern matching none of the group's rows is refused
with an error toast, the typed text kept for another try. The success
toast names a rule that differs from the merchant ("Categorized
<merchant> · rule: <pattern>"). Pickers become searchable (3.59).

**Categories.** `PageHeader` ("N categories · M subcategories", **New
category** opening an `InlineForm`: name, "Category type", Cancel, Add
category). Two `Panel`s side by side: the category tree (~300 px; rows
with `Swatch`, name — categories semibold, subcategories indented — the
`system` `TypeTag` and a muted count; `list_active` / `list_hover`) and the
selected category's detail (`flex_1`; with none selected it prompts
"Select a category to see its rules and settings."). The detail's title
row holds the name and `small` actions (Rename; Add subcategory, Move,
Delete each opening an `InlineForm` under it, 3.51); sections are split by
`border_t_1` dividers. A successful **Add subcategory** keeps its form open
on the still-selected parent — the name field clears, stays focused, and
Enter submits the next name, so subcategories go in a row (3.51, 3.96).
Its Rules section is a `DataList` of the category's
rules (a category's include its subcategories'; "No rules for this
category yet" when empty). The add-rule form (3.103) is the 3.48
editor's two-line `InlineForm` shape, so adding and editing look
alike: "Pattern" labelled at full width over the input, then a
label-less controls line — the regex `Switch` at the left, the
"Subcategory…" `Select` when a parent is selected, and the live match
summary and **Add rule** right-aligned (no `control_slot`;
`items_center` lines the controls up). While the pattern is non-empty
a compact heading-less `DataList` previews up to 8 matching
transactions under the form — Date, Merchant (ellipsized, full
description in a tooltip), Amount, with a muted "kept by `PATTERN`"
or "locked" tag after the amount — under the summary "Matches N
transactions · M would move here"; "No transactions match" when
empty, nothing at all for an empty pattern, the query reruns 150 ms
after the last keystroke (latest wins), and an invalid regex shows
the error text in place of the rows.
Where a rule goes follows the selection (3.53): a selected subcategory is
the target and the row carries no picker; a selected parent picks among
its own subcategories in a "Subcategory…" `Select` — their own names, no
default pick; the trigger fits the longest name up to 200 px, ellipsizing
with a hover tooltip, and the open menu grows to 320 px so names are
never cut off; a parent without subcategories shows a muted "Add a
subcategory first" in place of the row. Edit swaps a row for a two-line
`InlineForm` (Pattern labelled at full width over the input, then a
label-less controls line: the regex switch at the left, Save and Cancel
right-aligned; 3.48) with the same live match preview underneath, the
edited rule excluded (3.103), one at a time; the editor has no category picker —
Save keeps the rule's category, and a muted "To move rules to another
category, use the Rules page." sits under a non-empty rule list. Delete
is a two-click confirm. Selecting a category or leaving the page closes
the editor and disarms. Columns (3.49): Pattern (flex, ellipsized with a
tooltip; with a parent selected each rule also carries a muted `text_xs`
tag of its subcategory's name after the pattern — a subcategory view
paints no tags), Type, and Uses — the transactions this rule categorizes,
the same `rule_use_stats` mirror the Rules page shows, right-aligned
`text_xs` through `flow::count` with 0 muted. The Pattern and Uses
headings toggle the sort (Uses descending with pattern as tie-break, or
pattern ascending; the active one carries a ↓ marker) and the Uses
heading's tooltip reads "Transactions this rule categorizes". A focused
rule's transactions show under the list (see **Rules**, 3.52).

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
until the user searches or leaves. Clicking a rule row focuses it (3.52;
the checkbox and the row buttons don't): the list shrinks to its area
(~55%, `min_h` 120 px) and a matches panel takes the rest below a
`border_t_1` (`min_h` 160 px) — header "Transactions matching `PATTERN`"
(the pattern mono, ellipsized with a tooltip) plus a muted "N · M decided
by this rule", then rows with Date (110 px, `YYYY-MM-DD`), Description
(ellipsized with a tooltip), Account (140 px, muted, "Institution ·
Account") and Amount (right, 110 px): deciding rows first, newest first,
non-deciding rows muted with a trailing tag after the amount — "won by
`PATTERN`" or "locked" (hidden and card-payment rows never appear). The
empty state reads "This rule matches no transactions" ("…" while
loading). A second click, Escape, picking a category or leaving the page
clears the focus; the editor keeps it, and saving or any refresh reloads
the panel while focused; deleting a focused rule closes it. Escape
unwinds one level at a time: editor → armed Delete → focus → ticked rows.

**Transfers.** Open transfer reviews, each row with its candidates and
"Not a transfer".

**Imports.** `PageHeader` actions: **Import CSV** (primary), **Undo**
(two-click, newest import, disabled with no history), **Export**,
**Restore** (5.3: a save dialog defaulting to
`~/Desktop/financi-export-YYYY-MM-DD.zip` and a file dialog; busy while
one runs; the result is a toast, errors until closed). The import preview
(while a file is open) on top, then the history `DataTable` (time, file,
account, rows added, dates). The newest history row carries an in-row Undo
sharing the header Undo's confirm state, tooltip "Undo this import"; at
rest it reads "Undo" beside the `undo-2` icon — an explicit exception to
the icon-only `row_action_button` (3.37, user request).

**Import preview.** A two-line `Panel`. Line one: file name, then the
institution as a `text_sm` `TypeTag` (3.61), then Cancel and the primary
import action ("Import 32 transactions") at the right end. Line two, muted
with the key number in `foreground`: `Into [account ▾] | 32 new of 84 rows
· 52 already imported, skipped · Oct 1 – Dec 4, 2026`. No stat blocks.

## 6. Decisions

Confirmed with the user. Don't reopen without asking.

| # | Date | Question | Decision |
|---|---|---|---|
| D1 | 2026-11-07 | Category chart | gpui-kit `BarChart` with a clickable legend, beside the summary; bars aren't drawn into the legend. 3.36 moved the legend under the chart as a wrapping chip row (≤ 2 lines, "+N more" overflow), and 3.45 puts chart and summary on their own rows. 3.99 redrew it as a donut (`PieChart`) with the range total in the hole and the top 8 plus "Other"; the legend stays the click target and the panel picks side-by-side or stacked from its measured size. |
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

- UI tests (`ui/tests/` style) check behavior, not pixels: which rows
  show, which filter is active, what text a state shows, that a region
  paints inside its page and fits 1200 × 720.
- Click through rendered controls with a click helper (a simulated mouse
  click at the control's painted bounds) where possible, rather than
  calling the handler directly.
- To assert that nothing happened, first wait until the DB worker has no
  pending requests (`wait_until` on its idle state), then assert.
- When a convention here changes, update this file in the same commit.

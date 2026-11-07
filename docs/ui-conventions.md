# UI conventions

This file says how financi's window should look and behave, so every view
added in M2–M5 matches. It sits beside `plan.md` (what to build) and
`docs/tasks.md` (task order, GPUI pitfalls). If one of those files names a
specific layout or control, that file wins. This one covers everything else.

## Principles

1. **Data first, chrome last.** This is a desktop tool for one person who
   knows their own money. Numbers and rows get the space. Decoration doesn't.
   No hero headers, cards around cards, or icons that don't do anything.
2. **One window, one filter state.** Every view reads the same `ViewFilter`
   plus the selected category and month. Clicking a number narrows the
   filter. Nothing opens a second window.
3. **Use the gpui-kit theme as it is.** Colors, radius and fonts come from
   `cx.theme()`, using the default shadcn "neutral" theme in both light and
   dark mode. Don't hardcode `rgb(...)`/`hsla(...)` in views. If a color you
   need isn't in the table below, ask.
4. **Every state is designed.** Each region has an empty state, a loading
   state (keep the old content and dim it; don't flash a blank region) and an
   error state.

## Window layout

The window is a single vertical stack inside `p_4` padding, with `gap_3`
between regions:

```
┌──────────────────────────────────────────────────────────────────────┐
│ Toolbar: financi · [date range ▾] [accounts ▾] [☐ transfers]  … [Categories] [Import CSV…] │
│ Banner (only when something needs action, e.g. "2 transfers need review") │
│ Import preview panel (only while importing)                          │
├───────────────────────────────┬──────────────────────────────────────┤
│ Spending by category          │ Monthly summary                      │
│ chart + clickable legend      │ parents × months + Total             │
│ (~40% width)                  │ (~60% width)                         │
├───────────────────────────────┴──────────────────────────────────────┤
│ Filter chips · search · toggles                                      │
│ Transaction table (fills remaining height, scrolls)                  │
├──────────────────────────────────────────────────────────────────────┤
│ Status bar: 1,284 transactions · ~/…/financi.sqlite                  │
└──────────────────────────────────────────────────────────────────────┘
```

- **Toolbar.** Filters on the left (date range, accounts, transfers toggle).
  Actions on the right (Categories, Import CSV…). Import CSV… is the only
  `primary()` button in the window. Undo last import moves into the File menu
  and the import success message (M3, 3.7).
- **Middle row.** Chart and summary sit side by side in an `h_flex` with
  `gap_3`, and both have the same height (fixed, ~280px). The summary scrolls
  sideways if there are many months. The chart region never scrolls.
- **Table.** Takes `flex_1`, so it gets whatever height is left. Minimum
  window size: 1100 × 720.
- **Side sheets.** Categories (2.8) and transfer review (4.2) open as a
  gpui-kit `sheet` from the right, ~420px wide, over the dashboard. Only one
  sheet is open at a time.
- **Inline panels.** The import preview stays inline above the dashboard, as
  in M1. It's part of the main flow, so it isn't put in a modal. Keep it to
  two lines: the title (file name, then the institution in muted text) with
  Cancel and the primary action at the right end of the same row; then
  `Into [account ▾] | 32 new of 84 rows · 52 already imported, skipped ·
  Oct 1 – Dec 4, 2026` in muted text with the key number in `foreground`.
  No big stat blocks.
- **Banners.** One line in a bordered panel (no fill): a small `warning`
  dot, a bold sentence saying what needs doing, a muted reason, and one
  button at the right. Only for things that need a decision.

## Spacing and sizing

Use gpui's spacing helpers only, in these steps:

| Step | Helper | Use |
|---|---|---|
| 4px | `gap_1` | icon next to its label, chip contents |
| 8px | `gap_2` | controls inside one group (buttons in a row, label + input) |
| 12px | `gap_3` | between window regions; between toolbar groups |
| 16px | `p_4` | window padding; padding inside a panel or sheet |

- Panels (import preview, banners, chart, summary): `border_1`,
  `theme.border`, `rounded(theme.radius)`, background `theme.background`.
  **No filled panels.** A grey `secondary` fill or a tinted warning box
  looked heavy in the mockups and was rejected.
- **Button order in a row of actions:** Cancel first, then the primary
  action last, at the right end (the macOS order).
- Table rows: default gpui-kit row height, striped (`.stripe(true)`).
- **Text never pushes its container wider.** Any flex child that holds text
  (legend rows, summary cells, select labels) gets `min_w_0()` plus
  `overflow_hidden()`/`text_ellipsis()`, and amount and percent columns get
  fixed widths. Selects in the toolbar get a width that fits their longest
  label on one line. The first mockups broke exactly here: legend amounts
  spilled into the summary, and the date range label wrapped.

## Typography

- Font: the theme's system UI font. Base size 16px (gpui-kit default), but
  **data regions use `text_sm` (14px)**: table, summary, legend.
- Region titles: `text_sm`, `font_semibold`, `theme.foreground`. No larger
  headings inside the dashboard. The app name in the toolbar is `text_lg`.
- Secondary text (counts, captions, the status bar, hints): `text_xs` or
  `text_sm` in `theme.muted_foreground`.
- Numbers line up: right-align every amount column. If the system font's
  digits don't line up in practice, switch amount cells (only those) to
  `theme.mono_font_family`.
- Copy: sentence case ("Import CSV…", not "Import Csv"). Buttons are verbs.
  Use an ellipsis when the button opens a dialog or picker. Errors say what
  happened and what to do next ("Pick an account before importing").

## Color (theme tokens)

| Meaning | Token | Where |
|---|---|---|
| Text | `foreground` | everything by default |
| Secondary text | `muted_foreground` | captions, Uncategorized label, hints |
| Lines | `border` | panel borders, dividers |
| Main action | `primary` (button) | Import CSV…, the confirm button in a sheet |
| Money in | `success` | positive amounts in the table |
| Problems | `danger` | errors, negative Net in the summary |
| Needs action | `warning` | the small dot in a banner; never a filled box |
| Selection | `list_active` / `table_active` | selected legend row, selected summary cell |
| Category colors | `blue`, `cyan`, `magenta`, `yellow`, `green`, `red`, then their `_light` variants | one color per category; see *Category colors* below |
| Hidden rows | `muted_foreground` text | "Show hidden" mode |

### Category colors

Every category gets its own color, the same everywhere it appears: its bar
(`BarChart::fill` takes a closure per bar), its legend swatch, and a swatch
before its name in the monthly summary.

- Palette, in order: the theme's `blue`, `cyan`, `magenta`, `yellow`,
  `green`, `red`, then `blue_light` … `red_light` (12 colors, light and dark
  handled by the theme). Red and green come last because they also mean
  danger and money in.
- **Stable, not by rank.** Assign by position among siblings sorted by
  category id (`index % 12`), so a category keeps its color when the date
  range changes the order. Drilling into a parent gives its children their
  own colors from the start of the palette.
- Uncategorized is always `muted_foreground` grey, never a palette color.
- The swatch is a 10×10 square with a 2px radius. Don't color text with
  category colors.

Expected behavior isn't a warning. "Rows you already have are skipped" is
plain muted text. Use `warning` only for something the user must decide.

Money out is drawn in plain `foreground`, not red. Spending is the normal
case, and a table full of red reads as an alarm.

## Numbers, dates and labels

- **Money:** `$1,234.56`. In the table, amounts are signed: `-$86.42` out,
  `$2,105.00` in (green). In the chart, legend and summary, amounts are
  positive spend. Net is the one signed summary row.
- **Shares:** one decimal, `22.4%`.
- **Dates:** `2026-11-06` in the table (sorts by eye, no locale guessing).
  Month headers in the summary: `Sep 2026`. Ranges: `Sep 1 – Nov 30, 2026`.
- **Accounts:** the account's own name ("Amex Gold", "Simplii Chequing").
  Don't add the institution unless two accounts would otherwise show the
  same name.
- **Categories in the table:** the child name, with the parent in
  `muted_foreground` before it: `Food › Groceries`. Uncategorized rows show
  `Uncategorized` in `muted_foreground`.
- **Uncategorized in the legend and summary:** always the last category row
  (whatever its amount), with its name in `muted_foreground`, so it reads as
  work still to do rather than as a category.

## Interaction

- **Click to filter.** Legend rows, summary cells and summary row headers
  all set the shared filter. The selected item uses the active color. Click
  it again, or press Esc, to clear it.
- **Hints go in hover tooltips, not captions.** Don't put instructions like
  "click a cell to filter" in panel titles. Use `.tooltip(...)` on the
  element (it needs an `id`). Summary cell: `Shopping · Oct 2026 · 14
  transactions`, with "Click to show them in the table" under it. Chart bar
  (BarChart's own `tooltip_title`/`tooltip_value`): name, exact amount and
  share (`Restaurants · $642.15 · 31.4% of Food`).
- **Filter chips.** Each active narrowing filter (category, month, search,
  "Uncategorized only") shows as a chip with a × above the table. The
  toolbar filters (date, accounts, transfers) aren't chips, because the
  toolbar already shows them.
- **Row actions:** right-click context menu (2.9). No action buttons inside
  rows.
- **Destructive actions:** ask for confirmation inside the control (the
  current "Click again to undo" pattern) or in the sheet itself. Never use a
  native alert.
- **Keyboard:** Esc clears the innermost selection first (chip, then
  drill-down). ⌘O import, ⌘E export (3.7).

## States

| Region | Empty | Loading | Error |
|---|---|---|---|
| Whole window (no data) | Centered: "Drop an Amex or Simplii CSV export here, or use Import CSV…" plus "Restore from export…" link (5.2) | none | none |
| Chart / summary | "No spending in this range" (muted, centered) | previous content, dimmed | `danger` line inside the panel |
| Table | "No transactions match these filters" + "Clear filters" link | previous rows | `danger` line above the table |

## Decisions (confirmed with the user, 2026-11-07)

Decided from mockups. Don't reopen these without asking.

| # | Question | Decision |
|---|---|---|
| D1 | Category chart | gpui-kit `BarChart` beside a clickable legend, as task 3.5 says. Bars aren't drawn into the legend rows. |
| D2 | Category label in the table | `Parent › Child`, with the parent muted. |
| D3 | Color of money going out | Normal `foreground`. Only money in is colored (green). |
| D4 | Undo last import | Not in the toolbar. It's offered in the message shown after an import, and in File → Undo last import. |
| D5 | Category colors | Each category has its own stable color (see *Category colors*). |
| D6 | Hints | Hover tooltips instead of caption text in panel titles. |

## Testing UI conventions

UI tests (`ui_tests.rs` style) check behavior, not pixels: which rows show,
which filter is active, what text a state shows. When a convention here
changes, update this file in the same commit.

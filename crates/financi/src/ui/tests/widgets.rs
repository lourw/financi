//! Tests for `ui/widgets.rs` (3.62 split from `ui_tests.rs`).

use super::*;

use crate::ui::widgets::{
    Col, ColAlign, DataCol, DataItem, DataList, LIST_GROUP_HEIGHT, LIST_HEADING_HEIGHT,
    LIST_ROW_HEIGHT, ListCell, SortState, filter_row, inline_form, page_header, panel,
    row_action_button, swatch, type_tag,
};
use gpui_kit::component::theme::ActiveTheme as _;

// ---------------------------------------------------------------------------
// 3.89 design-system components. `ListHost` is the proof surface for
// `DataList`: a `Render` view with no page state, painting synthetic
// cells, so a test can measure the heading row, the 40 px rows, the
// collapsible 36 px group headers and the hover-revealed actions column
// without a database. The `wh:` selector prefix belongs to this test
// alone.
// ---------------------------------------------------------------------------

struct ListHost {
    columns: Vec<DataCol>,
    /// Plain rows before any group; each string is the row's selector.
    rows: Vec<String>,
    /// `(group id, candidate rows)`.
    groups: Vec<(i64, usize)>,
    collapsed: Vec<i64>,
    actions: bool,
    sort: bool,
    sorted: Option<(&'static str, SortState)>,
    /// Paint every cell with a 400-character label instead of its short
    /// synthetic one, to show columns keep their widths.
    long: bool,
    /// When set, the `Col::Flex` column's cells are [`ListCell::text`]
    /// carrying this full text, to show the list's ellipsis tooltip.
    tip: Option<String>,
}

impl Render for ListHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut items = Vec::new();
        for (i, sel) in self.rows.iter().enumerate() {
            items.push(DataItem::Row {
                id: i,
                selected: false,
                selector: sel.clone(),
            });
        }
        for (gid, n) in &self.groups {
            items.push(DataItem::Group {
                id: *gid,
                collapsed: self.collapsed.contains(gid),
                name: format!("group {gid}").into(),
                count: Some(format!("{n} items").into()),
                selector: format!("wh:group:{gid}"),
            });
            if !self.collapsed.contains(gid) {
                for i in 0..*n {
                    items.push(DataItem::Row {
                        id: (gid * 100) as usize + i,
                        selected: false,
                        selector: format!("wh:row:{gid}-{i}"),
                    });
                }
            }
        }
        let mut list =
            DataList::new(cx.entity(), self.columns.clone(), items).cells(|view, item, _, _| {
                match item {
                    DataItem::Row { id, selector, .. } => view
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(c, col)| -> ListCell {
                            if let Some(tip) = &view.tip
                                && matches!(col.width, Col::Flex)
                            {
                                return ListCell::text(tip.clone());
                            }
                            let text = if view.long && c > 0 {
                                "x".repeat(400)
                            } else {
                                format!("r{id}c{c}")
                            };
                            div()
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(text)
                                .debug_selector({
                                    let s = format!("{selector}:c{c}");
                                    move || s.clone()
                                })
                                .into_any_element()
                                .into()
                        })
                        .collect(),
                    DataItem::Group { selector, .. } => vec![
                        div()
                            .child("sum")
                            .debug_selector({
                                let s = format!("{selector}:sum");
                                move || s.clone()
                            })
                            .into_any_element()
                            .into(),
                    ],
                }
            });
        list = list.toggle(|this, gid, _, cx| {
            match this.collapsed.iter().position(|g| *g == gid) {
                Some(pos) => {
                    this.collapsed.remove(pos);
                }
                None => this.collapsed.push(gid),
            }
            cx.notify();
        });
        if self.actions {
            list = list.actions(90., |_, item, _, _| match item {
                DataItem::Row { selector, .. } => vec![
                    div()
                        .child("A")
                        .debug_selector({
                            let s = format!("{selector}:act");
                            move || s.clone()
                        })
                        .into_any_element(),
                ],
                DataItem::Group { selector, .. } => vec![
                    div()
                        .child("G")
                        .debug_selector({
                            let s = format!("{selector}:gact");
                            move || s.clone()
                        })
                        .into_any_element(),
                ],
            });
        }
        if self.sort {
            list = list.sort(|this, title, state, _, cx| {
                this.sorted = Some((title, state));
                cx.notify();
            });
        }
        if let Some((title, state)) = self.sorted {
            list = list.sorted_by(title, state);
        }
        v_flex()
            .size_full()
            // A base-text probe (the theme's 16 px): row cells must
            // measure at the heading's `text_sm`, not at this size.
            .child(div().debug_selector(|| "wh:base-text".into()).child("base"))
            .child(list.heading_selectors("wh").selector("wh:list"))
    }
}

fn wh_columns() -> Vec<DataCol> {
    vec![
        DataCol::new("Date", Col::Fixed(110.), ColAlign::Left),
        DataCol::new("Description", Col::Flex, ColAlign::Left),
        DataCol::new("Amount", Col::Fixed(120.), ColAlign::Right),
    ]
}

fn wh_host(rows: usize) -> ListHost {
    ListHost {
        columns: wh_columns(),
        rows: (0..rows).map(|i| format!("wh:row:{i}")).collect(),
        groups: Vec::new(),
        collapsed: Vec::new(),
        actions: false,
        sort: false,
        sorted: None,
        long: false,
        tip: None,
    }
}

/// Open a list-host window (after `open_shell`, so gpui-kit is
/// initialized) and draw one frame. Returns the host entity too, so a
/// test can read what the list callbacks recorded.
fn open_list_host(
    cx: &mut TestAppContext,
    host: ListHost,
) -> (AnyWindowHandle, VisualTestContext, Entity<ListHost>) {
    let (shell_handle, _shell) = open_shell(cx, |_| {});
    let _ = shell_handle;
    let mut host = Some(host);
    let handle = cx.add_window(|_, _| host.take().unwrap());
    let entity = handle.root(cx).unwrap();
    let handle: AnyWindowHandle = handle.into();
    draw_frame(cx, handle);
    (handle, VisualTestContext::from_window(handle, cx), entity)
}

/// The 32 px heading, 40 px rows, matching columns (heading cell over
/// row cell, same left edge and width, right-aligned amounts sharing
/// their right edge), and an actions column that is reserved but hidden
/// until the row is hovered — without shifting any column.
#[gpui::test]
fn the_shared_list_lines_headings_and_rows_up(cx: &mut TestAppContext) {
    let mut host = wh_host(3);
    host.actions = true;
    let (handle, mut vtc, _entity) = open_list_host(cx, host);
    let b = |vtc: &mut VisualTestContext, s: &str| {
        let s = leaked(s.to_string());
        vtc.debug_bounds(s)
            .unwrap_or_else(|| panic!("{s} does not paint"))
    };
    // Token heights (ui-conventions *Lists*).
    let heading = b(&mut vtc, "wh:heading");
    assert!(
        (heading.size.height.as_f32() - LIST_HEADING_HEIGHT).abs() <= 1.,
        "heading row is {} px, not {LIST_HEADING_HEIGHT}",
        heading.size.height
    );
    let row = b(&mut vtc, "wh:row:1");
    assert!(
        (row.size.height.as_f32() - LIST_ROW_HEIGHT).abs() <= 1.,
        "data row is {} px, not {LIST_ROW_HEIGHT}",
        row.size.height
    );
    // Every heading sits over its column with the same width.
    for (title, cell) in [("Date", "c0"), ("Description", "c1"), ("Amount", "c2")] {
        let head = b(&mut vtc, &format!("wh:heading:{title}"));
        let row_cell = b(&mut vtc, &format!("wh:row:1:{cell}"));
        assert!(
            (head.origin.x - row_cell.origin.x).abs().as_f32() <= 1.
                && (head.size.width - row_cell.size.width).abs().as_f32() <= 1.,
            "heading {title} {:?} is not over its row cell {:?}",
            (head.origin.x, head.size.width),
            (row_cell.origin.x, row_cell.size.width),
        );
    }
    // The actions are invisible until the row is hovered…
    assert!(
        vtc.debug_bounds("wh:row:1:act").is_none(),
        "a resting row's actions stay hidden"
    );
    // …and revealing them never moves a column.
    let before = b(&mut vtc, "wh:row:1:c2");
    let row = b(&mut vtc, "wh:row:1");
    vtc.simulate_mouse_move(row.center(), None, Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        vtc.debug_bounds("wh:row:1:act").is_some(),
        "hovering the row reveals its actions"
    );
    let after = b(&mut vtc, "wh:row:1:c2");
    assert!(
        (before.origin.x - after.origin.x).abs().as_f32() <= 1.,
        "the reserved actions column shifted the last column \
         ({before:?} -> {after:?})",
    );
}

/// Group headers are 36 px, collapse and expand from their chevron
/// alone, and their trailing sum lines up with the last column.
#[gpui::test]
fn group_headers_collapse_and_expand(cx: &mut TestAppContext) {
    let mut host = wh_host(0);
    host.groups = vec![(1, 3), (2, 2)];
    host.actions = true;
    let (handle, mut vtc, _entity) = open_list_host(cx, host);
    let b = |vtc: &mut VisualTestContext, s: &str| {
        let s = leaked(s.to_string());
        vtc.debug_bounds(s)
            .unwrap_or_else(|| panic!("{s} does not paint"))
    };
    let group = b(&mut vtc, "wh:group:1");
    assert!(
        (group.size.height.as_f32() - LIST_GROUP_HEIGHT).abs() <= 1.,
        "group header is {} px, not {LIST_GROUP_HEIGHT}",
        group.size.height
    );
    for i in 0..3 {
        assert!(
            vtc.debug_bounds(leaked(format!("wh:row:1-{i}"))).is_some(),
            "group 1's rows paint while it is open"
        );
    }
    // The group's sum shares the Amount column's right edge.
    let sum = b(&mut vtc, "wh:group:1:sum");
    let amount = b(&mut vtc, "wh:heading:Amount");
    assert!(
        ((sum.origin.x + sum.size.width) - (amount.origin.x + amount.size.width))
            .abs()
            .as_f32()
            <= 1.,
        "the group sum {:?} does not end on the Amount column's right edge {:?}",
        sum,
        amount,
    );
    // The chevron collapses only its own group.
    click(cx, handle, "wh:group:1:toggle");
    draw_frame(cx, handle);
    assert!(
        vtc.debug_bounds("wh:row:1-0").is_none()
            && vtc.debug_bounds("wh:row:1-2").is_none()
            && vtc.debug_bounds("wh:group:1").is_some(),
        "the chevron collapsed group 1"
    );
    assert!(
        vtc.debug_bounds("wh:row:2-0").is_some(),
        "group 2 stayed open"
    );
    click(cx, handle, "wh:group:1:toggle");
    draw_frame(cx, handle);
    assert!(
        (0..3).all(|i| { vtc.debug_bounds(leaked(format!("wh:row:1-{i}"))).is_some() }),
        "a second click expanded group 1 again"
    );
}

/// A label too long for the flex column truncates inside it: the column
/// keeps its width and the columns after it stay put.
#[gpui::test]
fn a_long_label_stays_in_its_flex_column(cx: &mut TestAppContext) {
    let mut host = wh_host(1);
    host.long = true;
    host.actions = true;
    let (_handle, mut vtc, _entity) = open_list_host(cx, host);
    let head = vtc.debug_bounds("wh:heading:Description").unwrap();
    let cell = vtc.debug_bounds("wh:row:0:c1").unwrap();
    assert!(
        (head.size.width - cell.size.width).abs().as_f32() <= 1.,
        "the flex column resized to its content: heading {head:?} vs cell {cell:?}",
    );
    let amount = vtc.debug_bounds("wh:row:0:c2").unwrap();
    let amount_head = vtc.debug_bounds("wh:heading:Amount").unwrap();
    assert!(
        (amount.origin.x - amount_head.origin.x).abs().as_f32() <= 1.,
        "a long label pushed the Amount column: cell {amount:?} vs heading {amount_head:?}",
    );
}

/// Tokens → Typography: a list is a data region, so row cells paint at
/// `text_sm` like the heading, never at the base size. (The 3.89 review
/// measured 22.5 px heading text vs 26 px row text; the assertion is the
/// relation — row cell like the heading, clearly below the base-text
/// probe — not absolute pixels, which depend on the font.)
#[gpui::test]
fn row_cells_paint_at_the_heading_text_size(cx: &mut TestAppContext) {
    let host = wh_host(1);
    let (_handle, mut vtc, _entity) = open_list_host(cx, host);
    let h = |vtc: &mut VisualTestContext, s: &str| {
        let s = leaked(s.to_string());
        vtc.debug_bounds(s)
            .unwrap_or_else(|| panic!("{s} does not paint"))
            .size
            .height
    };
    let base = h(&mut vtc, "wh:base-text");
    let heading = h(&mut vtc, "wh:heading:Date");
    let cell = h(&mut vtc, "wh:row:0:c0");
    assert!(
        (heading - cell).abs().as_f32() <= 1.,
        "row cells must paint at the heading's text size: heading {heading} vs row cell {cell}",
    );
    assert!(
        (base - cell).as_f32() >= 2.,
        "row cells still inherit the base text size: base {base} vs row cell {cell}",
    );
}

/// Components → `DataList`: a Flex text cell carries a tooltip holding
/// the full text, attached by the list itself (`ListCell::text`), not by
/// the page. Hover the flex column of a row — the heading cell gives the
/// column's x, the row its y — and the tooltip must paint with the
/// cell's full text (the tooltip body carries a `financi:tooltip:<text>`
/// debug selector, like [`filter_tip`]).
#[gpui::test]
fn a_flex_text_cell_tooltips_its_full_text(cx: &mut TestAppContext) {
    let tip = "COSTCO COFFEE #1234 SEATTLE WA".to_string();
    let mut host = wh_host(1);
    host.tip = Some(tip.clone());
    let (handle, mut vtc, _entity) = open_list_host(cx, host);
    let head = vtc
        .debug_bounds("wh:heading:Description")
        .unwrap_or_else(|| panic!("the flex heading paints"));
    let row = vtc
        .debug_bounds("wh:row:0")
        .unwrap_or_else(|| panic!("row 0 paints"));
    vtc.simulate_mouse_move(
        point(
            head.origin.x + head.size.width / 2.,
            row.origin.y + row.size.height / 2.,
        ),
        None,
        Modifiers::default(),
    );
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("financi:tooltip:{tip}")),
        "hovering the flex text cell shows no tooltip with the full text",
    );
}

/// Clicking a heading sorts: the active column gets an arrow, and a
/// repeat click flips the direction the callback is handed.
#[gpui::test]
fn a_heading_click_reports_the_next_sort_direction(cx: &mut TestAppContext) {
    let mut host = wh_host(2);
    host.sort = true;
    let (handle, _vtc, entity) = open_list_host(cx, host);
    // First click on the Amount heading: ascending.
    click(cx, handle, "wh:heading:Amount");
    draw_frame(cx, handle);
    assert_eq!(
        entity.update(cx, |h, _| h.sorted),
        Some(("Amount", SortState::Ascending)),
        "the first heading click asks for ascending"
    );
    // A repeat click on the same heading flips the direction handed to
    // the callback; the arrow paints for the new state.
    click(cx, handle, "wh:heading:Amount");
    draw_frame(cx, handle);
    assert_eq!(
        entity.update(cx, |h, _| h.sorted),
        Some(("Amount", SortState::Descending)),
        "the repeat click flips to descending"
    );
    // A different column starts ascending again.
    click(cx, handle, "wh:heading:Date");
    draw_frame(cx, handle);
    assert_eq!(
        entity.update(cx, |h, _| h.sorted),
        Some(("Date", SortState::Ascending)),
        "another column's heading starts at ascending"
    );
}

/// The smaller pieces of the design system paint as specified: the panel's
/// 16 px padded and 0 px variants, the page header with its actions pushed
/// right, the 32 px filter strip, the bordered inline form, the 10 px
/// swatch, the outline tag and the ghost xsmall row action.
#[gpui::test]
fn the_panel_header_forms_swatch_and_tag_paint(cx: &mut TestAppContext) {
    struct BitsHost;
    impl Render for BitsHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let red = {
                let theme = cx.theme();
                theme.red
            };
            v_flex()
                .size_full()
                .gap_2()
                .child(
                    page_header(
                        cx,
                        "Header",
                        "9 things",
                        vec![
                            div()
                                .child("act")
                                .debug_selector(|| "wh:ph:action".to_string())
                                .into_any_element(),
                        ],
                    )
                    .debug_selector(|| "wh:ph".to_string()),
                )
                .child(
                    filter_row()
                        .child(
                            div()
                                .size_full()
                                .debug_selector(|| "wh:filter:fill".to_string()),
                        )
                        .debug_selector(|| "wh:filter".to_string()),
                )
                .child(inline_form(cx).debug_selector(|| "wh:form".to_string()))
                .child(
                    panel(cx, true)
                        .child(
                            div()
                                .size_full()
                                .debug_selector(|| "wh:panel:inner".to_string()),
                        )
                        .debug_selector(|| "wh:panel".to_string()),
                )
                .child(swatch(red).debug_selector(|| "wh:swatch".to_string()))
                .child(
                    div()
                        .debug_selector(|| "wh:tag".to_string())
                        .child(type_tag("Income")),
                )
                .child(
                    row_action_button("wh-row-act", IconName::X, "Delete")
                        .debug_selector(|| "wh:row-act".to_string()),
                )
        }
    }
    let (_shell_handle, _shell) = open_shell(cx, |_| {});
    let handle: AnyWindowHandle = cx.add_window(|_, _| BitsHost).into();
    draw_frame(cx, handle);
    let mut vtc = VisualTestContext::from_window(handle, cx);
    let b = |vtc: &mut VisualTestContext, s: &str| {
        let s = leaked(s.to_string());
        vtc.debug_bounds(s)
            .unwrap_or_else(|| panic!("{s} does not paint"))
    };
    // The padded panel insets its child by the standard 16 px.
    let panel_b = b(&mut vtc, "wh:panel");
    let inner = b(&mut vtc, "wh:panel:inner");
    assert!(
        (inner.origin.x.as_f32() - panel_b.origin.x.as_f32() - 16.).abs() <= 1.
            && (inner.origin.y.as_f32() - panel_b.origin.y.as_f32() - 16.).abs() <= 1.,
        "the padded panel does not inset its child 16 px: {panel_b:?} vs {inner:?}",
    );
    // The header's actions sit at its right edge.
    let header = b(&mut vtc, "wh:ph");
    let action = b(&mut vtc, "wh:ph:action");
    assert!(
        ((action.origin.x + action.size.width) - (header.origin.x + header.size.width))
            .abs()
            .as_f32()
            <= 1.,
        "the page header's actions are not at its right edge: {header:?} vs {action:?}",
    );
    // The filter strip is one 32 px row; the swatch is a 10 px square;
    // the row action is xsmall (well under a medium button's height).
    let filter = b(&mut vtc, "wh:filter");
    assert!(
        (filter.size.height.as_f32() - 32.).abs() <= 1.,
        "{filter:?}"
    );
    let swatch = b(&mut vtc, "wh:swatch");
    assert!(
        (swatch.size.width.as_f32() - 10.).abs() <= 1.
            && (swatch.size.height.as_f32() - 10.).abs() <= 1.,
        "the swatch is not 10×10: {swatch:?}",
    );
    let act = b(&mut vtc, "wh:row-act");
    assert!(
        act.size.height.as_f32() < 28.,
        "row action not xsmall: {act:?}"
    );
    assert!(b(&mut vtc, "wh:tag").size.width.as_f32() > 0.);
    assert!(b(&mut vtc, "wh:form").size.width.as_f32() > 0.);
}
/// 3.27: success and info toasts dismiss themselves after ~4 s. The manager
/// runs on a 100 ms ticker, so the whole life of a toast is entrance (400 ms)
/// + timeout (4 s) + exit (200 ms) of virtual time.
#[gpui::test]
fn info_toasts_dismiss_themselves_after_a_few_seconds(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    // 5.3 took the Export… placeholder toast away; a finished import
    // pushes the same plain info toast.
    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Simplii");
    wait_until(cx, "the toast is up", |cx| !toasts(&shell, cx).is_empty());
    // 3 s in: still mounted, the timeout is 4 s.
    tick_toasts(cx, 30);
    cx.update(|cx| {
        assert_eq!(
            toasts(&shell, cx).len(),
            1,
            "3 s in the toast is still mounted"
        );
    });
    // 2.5 s more: the timeout fires and the exit animation runs out.
    tick_toasts(cx, 25);
    cx.update(|cx| {
        assert!(toasts(&shell, cx).is_empty(), "the toast went away");
    });
}

/// 3.27: an error toast outlives the auto-dismiss and closes when clicked.
#[gpui::test]
fn an_error_toast_stays_until_the_user_closes_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    let dir = std::env::temp_dir().join(format!("financi-ui-err-toast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let odd = dir.join("odd.csv");
    std::fs::write(&odd, "Posted,Payee,Debit\n2026-11-01,X,1.00\n").unwrap();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.open_file(odd.clone(), window, cx));
    });
    cx.run_until_parked();
    let msg = cx.update(|cx| {
        let toasts = toasts(&shell, cx);
        assert_eq!(toasts.len(), 1);
        assert!(toasts[0].1, "the toast is an error");
        toasts[0].0.clone()
    });
    // Ten seconds of ticker: well past the 4 s auto-dismiss, and still there.
    tick_toasts(cx, 100);
    cx.update(|cx| {
        let toasts = toasts(&shell, cx);
        assert_eq!(toasts.len(), 1, "the error toast outlived the timeout");
        assert!(toasts[0].1);
    });
    // A click on the card closes it, once the exit animation runs out.
    draw_frame(cx, handle);
    let mut vtc = VisualTestContext::from_window(handle, cx);
    let card = bounds(cx, handle, format!("toast:{msg}"));
    vtc.simulate_click(card.center(), Modifiers::default());
    cx.run_until_parked();
    tick_toasts(cx, 5);
    cx.update(|cx| {
        assert!(toasts(&shell, cx).is_empty(), "the click dismissed it");
    });
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------
// Import end-to-end (task 3.9): run both demo exports through the real
// import panel, then visit every page and check the values shown against
// `fixtures/demo/expected.json`. Runs on the "All time" preset so the
// result never depends on the machine's date.

/// 3.14: a checked box draws its check from the `icons/check.svg` asset,
/// loaded through GPUI's application asset source. The app used to run with
/// GPUI's default source, which loads nothing, so the box changed background
/// but showed no check; it must register the bundled gpui-kit icons.
#[test]
fn a_checked_box_finds_its_check_glyph() {
    use gpui_kit::AssetSource as _;
    let check = IconName::Check.path();
    assert_eq!(check, "icons/check.svg");
    assert!(
        ().load(&check).ok().flatten().is_none(),
        "GPUI's default asset source — the bug's baseline — loads nothing"
    );
    let svg = crate::icon_assets()
        .load(&check)
        .ok()
        .flatten()
        .expect("the app registers the icon a checked Checkbox draws for its mark");
    assert!(std::str::from_utf8(&svg).is_ok_and(|s| s.contains("<svg")));
}

// 3.92: every UI job reaches the worker through `on_db`. A panicking job
// (an undo or a category edit that blows up) must report the *job panic*
// — the toast text the user sees — not the old worker-gone message a
// dropped result sender produced, and the worker must keep serving the
// next job. Worker-layer twin: `panicking_typed_job_reports_job_panicked`.

#[test]
fn a_panicking_on_db_job_reports_the_job_panic() {
    use crate::ui::widgets::on_db;
    let db = DbWorker::spawn(open_in_memory().unwrap());
    let err = block_on(on_db(&db, |c| -> Result<i64, std::io::Error> {
        c.execute_batch("BEGIN; INSERT INTO settings (key, value) VALUES ('x', '1');")
            .unwrap();
        panic!("boom")
    }))
    .expect_err("a panicking job fails");
    assert!(err.contains("panicked"), "{err}");
    assert!(!err.contains("gone"), "{err}");

    // The worker survives the panic and still answers reads.
    let rows = block_on(on_db(&db, |c| {
        c.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
    }))
    .expect("the worker still serves jobs after a panic");
    assert_eq!(rows, 0, "the panicking job's transaction was rolled back");
}

/// 3.92: only panics are re-classified — a job that fails normally keeps
/// passing its own message through, unchanged from the untyped path.
#[test]
fn on_db_passes_ordinary_job_errors_through_unchanged() {
    use crate::ui::widgets::on_db;
    let db = DbWorker::spawn(open_in_memory().unwrap());
    let err = block_on(on_db(&db, |_c| -> Result<i64, std::io::Error> {
        Err(std::io::Error::other("nope"))
    }))
    .expect_err("the job fails");
    assert_eq!(err, "nope");

    let value = block_on(on_db(&db, |c| {
        c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
    }))
    .expect("a job that succeeds still returns its value");
    assert_eq!(value, 1);
}

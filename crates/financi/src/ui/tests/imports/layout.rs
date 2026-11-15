//! Drawn layout of the Imports page: history-table column widths (3.20)
//! and the header and in-row buttons' icons and labels (3.32, 3.37).

use super::*;

// --------------------------------------------------- 3.20 history table width

/// The width plan itself: the Undo column fits both button labels, and
/// from the minimum window's page width up every column fits the page —
/// File and Account shrink and grow with the window instead of the old
/// fixed 240 px each.
#[test]
fn history_table_columns_fit_every_page_width() {
    // The Undo column budgets the longest label ("Click again to undo")
    // at the generous per-character width, plus the xsmall button's icon
    // and padding and the cell's, so neither it nor the icon-plus-"Undo"
    // rest state (3.37) can clip.
    assert!(
        history_undo_col_w()
            >= UNDO_LABEL_MAX.chars().count() as f32 * TEXT_SM_CHAR_PX
                + BUTTON_PAD_X
                + UNDO_ICON_W
                + CELL_PAD_X
    );
    // 3.37: the idle label fits too, and the column is exactly as wide as
    // the widest label needs — no wider, so it steals nothing from `File`
    // and `Account`.
    assert!(
        history_undo_col_w()
            >= "Undo".chars().count() as f32 * TEXT_SM_CHAR_PX
                + BUTTON_PAD_X
                + UNDO_ICON_W
                + CELL_PAD_X
    );
    assert_eq!(
        history_undo_col_w(),
        (UNDO_LABEL_MAX.chars().count() as f32 * TEXT_SM_CHAR_PX
            + BUTTON_PAD_X
            + UNDO_ICON_W
            + CELL_PAD_X)
            .ceil()
    );
    for win in [1200., 1440., 1920., 2560., 4000.] {
        let page_w = win - SIDEBAR_W - 2. * PAGE_PAD;
        let (file, account) = history_flex_widths(page_w);
        let total = HISTORY_TIME_W
            + file
            + account
            + HISTORY_ROWS_W
            + HISTORY_DATES_W
            + history_undo_col_w()
            + TABLE_BORDER_X;
        assert!(
            total <= page_w + 0.01,
            "window {win}: columns sum to {total}, page is {page_w}"
        );
        assert!(
            file >= HISTORY_FLEX_MIN_W && account >= HISTORY_FLEX_MIN_W,
            "window {win}"
        );
    }
    // File and Account grow with the window, and at the minimum size
    // they are below the old fixed 240 px that overflowed the page.
    let small = history_flex_widths(1200. - SIDEBAR_W - 2. * PAGE_PAD).0;
    let big = history_flex_widths(2560. - SIDEBAR_W - 2. * PAGE_PAD).0;
    assert!(small < 240., "{small} at the minimum window");
    assert!(big > small, "{big} vs {small}");
    // Below the minimum window the flexible columns stop at their floor.
    assert_eq!(
        history_flex_widths(100.),
        (HISTORY_FLEX_MIN_W, HISTORY_FLEX_MIN_W)
    );
}

/// 3.20: with long file and account names the history table draws at the
/// minimum window size and at a large one, its columns — the widths the
/// table actually renders at — never summing past the page beside the
/// sidebar, so it never scrolls sideways.
#[gpui::test]
fn the_imports_history_table_fits_the_available_width(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_long_history);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.read(cx).import_history.len() == 1
    });

    let resize = |cx: &mut TestAppContext, w: f32, h: f32| {
        with_window(cx, handle, |window, cx| {
            window.resize(size(px(w), px(h)));
            window.bounds_changed(cx);
        });
        cx.run_until_parked();
        draw_frame(cx, handle);
    };
    let widths = |cx: &App| -> Vec<f32> {
        let table = shell.read(cx).imports.read(cx).history_table.read(cx);
        (0..6)
            .map(|i| table.col_width(i).unwrap().as_f32())
            .collect()
    };

    // Minimum window: the drawn columns fit the page beside the sidebar.
    resize(cx, 1200., 720.);
    let small = cx.update(|cx| widths(cx));
    let page_w = 1200. - SIDEBAR_W - 2. * PAGE_PAD;
    assert!(
        small.iter().sum::<f32>() <= page_w + 0.01,
        "columns sum to {}, page is {page_w}",
        small.iter().sum::<f32>()
    );
    // Undo is sized for both labels; File and Account shrank below the
    // old fixed 240 px and truncate their long names with an ellipsis.
    assert!(
        small[5] >= history_undo_col_w(),
        "Undo column {} cannot fit both labels",
        small[5]
    );
    assert!(small[1] < 240. && small[2] < 240., "{small:?}");
    assert!((small[1] - small[2]).abs() < 0.01, "{small:?}");

    // A large window: still fitting, and the flexible columns grew.
    resize(cx, 2560., 1440.);
    let big = cx.update(|cx| widths(cx));
    assert!(
        big.iter().sum::<f32>() <= 2560. - SIDEBAR_W - 2. * PAGE_PAD + 0.01,
        "columns sum to {}",
        big.iter().sum::<f32>()
    );
    assert!(big[1] > small[1], "File column grows with the window");
}

/// Horizontal space a medium `Button` draws around its label when it has
/// an icon: the medium icon (`size_4`, 16 px) plus the `gap_2` (8 px)
/// between icon and label, and the button's own `px_2p5` per side (3.32).
const HEADER_ICON_BLOCK_W: f32 = 24.;

const HEADER_BUTTON_PAD_X: f32 = 20.;

/// Slack for comparing a drawn button width against label + icon +
/// padding: layout rounds to physical pixels. Tight enough that a
/// dropped `.icon()` call — 24 px on a header button, 16 on the armed
/// in-row one — fails the comparison.
const WIDTH_TOL: f32 = 3.;

/// 3.32: a window of plain labels — each at the text size the button
/// with that label renders (medium buttons `text_sm`, xsmall `text_xs`)
/// — so a test can pin that a button's drawn width is its label plus
/// exactly the icon block and padding. A debug selector records how a
/// button was built; only its width shows the icon actually paints.
struct IconWidthRefs {
    /// `(selector, text, xsmall?)` per label.
    labels: Vec<(&'static str, &'static str, bool)>,
}

impl Render for IconWidthRefs {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex().children(self.labels.iter().map(|(sel, text, xs)| {
            // `self_start` so the label's width is its text, not the
            // full window (columns stretch their children by default).
            let label = div().self_start().child(*text);
            if *xs {
                label.text_xs()
            } else {
                label.text_sm()
            }
            .debug_selector(move || (*sel).to_string())
        }))
    }
}

/// Open the reference-label window (after `open_shell`, so gpui-kit is
/// initialized) and draw one frame; the returned context reads the
/// labels' drawn widths.
fn open_icon_width_refs(
    cx: &mut TestAppContext,
    labels: Vec<(&'static str, &'static str, bool)>,
) -> VisualTestContext {
    let handle: AnyWindowHandle = cx.add_window(|_, _| IconWidthRefs { labels }).into();
    draw_frame(cx, handle);
    VisualTestContext::from_window(handle, cx)
}

/// Assert a drawn button's width is its label's drawn width plus the
/// icon block and horizontal padding (3.32) — the icon must paint.
fn assert_button_paints_icon(
    refs: &mut VisualTestContext,
    what: &str,
    btn: Bounds<Pixels>,
    text_sel: &'static str,
    icon_block_w: f32,
    pad_x: f32,
) {
    let text_w = refs
        .debug_bounds(text_sel)
        .unwrap_or_else(|| panic!("the reference window draws {text_sel:?}"))
        .size
        .width
        .as_f32();
    let got = btn.size.width.as_f32();
    let want = text_w + icon_block_w + pad_x;
    assert!(
        (got - want).abs() <= WIDTH_TOL,
        "{what}: drawn width {got} is not its label {text_w} + icon block \
         {icon_block_w} + padding {pad_x} (± {WIDTH_TOL}) — is the icon drawn?",
    );
}

/// 3.32: the history table's in-row Undo button is an xsmall ghost button.
/// Idle and armed it stays fully inside the fixed 32 px row with at least
/// 4 px above and below, arming never makes the newest row taller than the
/// others, and the armed label fits the Undo column. Hovering it shows the
/// "Undo this import" tooltip.
#[gpui::test]
fn the_in_row_undo_button_stays_inside_its_row(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.read(cx).import_history.len() == 3
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // Each Undo cell carries a `h_full` marker spanning the cell's content
    // box, so the button's centring and the row heights are measurable.
    // The table's 32 px row height and 4 px-per-side cell padding are the
    // gpui-kit `Size::Medium` defaults the width constants already assume.
    const ROW_H: f32 = 32.;
    const CELL_PAD_Y: f32 = 4.;
    let cell = |cx: &mut TestAppContext, handle: AnyWindowHandle, row: usize| {
        bounds(cx, handle, format!("history:undo-cell:{row}"))
    };
    let btn = |window: &mut VisualTestContext, label: &str| {
        window
            .debug_bounds(leaked(format!("history:undo-btn:{label}")))
            .expect("the newest row draws its Undo button")
    };
    // Distance from the button's edges to the row's, the cell padding
    // included; the spec's floor is 4 px above and below.
    let clearance = |b: Bounds<Pixels>, c: Bounds<Pixels>| {
        (
            b.origin.y.as_f32() - c.origin.y.as_f32() + CELL_PAD_Y,
            c.origin.y.as_f32() + c.size.height.as_f32()
                - b.origin.y.as_f32()
                - b.size.height.as_f32()
                + CELL_PAD_Y,
        )
    };

    // Every row is one row height and every Undo cell the same height.
    let cells: Vec<_> = (0..3).map(|r| cell(cx, handle, r)).collect();
    for (above, below) in cells.iter().zip(cells[1..].iter()) {
        assert_eq!(
            above.size.height, below.size.height,
            "Undo cells are the same height in every row"
        );
        let pitch = below.origin.y.as_f32() - above.origin.y.as_f32();
        assert!(
            (pitch - ROW_H).abs() <= 1.,
            "rows sit one row height apart, pitch {pitch}"
        );
    }

    // Idle: the icon-and-"Undo"-label button (3.37) is centred with
    // clearance to spare, and its label fits the Undo column.
    let (above, below) = clearance(btn(&mut window, "Undo"), cells[0]);
    assert!(
        above >= 4. && below >= 4.,
        "idle: {above} above, {below} below"
    );
    let col_w = cx.update(|cx| {
        shell
            .read(cx)
            .imports
            .read(cx)
            .history_table
            .read(cx)
            .col_width(5)
            .unwrap()
    });
    assert!(
        btn(&mut window, "Undo").size.width + px(CELL_PAD_X) <= col_w,
        "the idle label overflows the Undo column: button {:?} + {CELL_PAD_X} > {col_w:?}",
        btn(&mut window, "Undo").size.width,
    );

    // Hovering the button shows the tooltip naming the action.
    let b = btn(&mut window, "Undo");
    let centre = point(b.center().x, b.center().y);
    window.simulate_mouse_move(centre, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("history:undo-tooltip").is_some(),
        "hovering the Undo button shows its tooltip"
    );
    // Move off so the tooltip cannot hold the hover while the button is armed.
    window.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);

    // Arm the confirmation (first click): the button grows its prompt —
    // still with clearance, the row no taller than its neighbours, and the
    // armed label fitting the Undo column.
    let newest = cx.update(|cx| shell.read(cx).imports.read(cx).import_history[0].id);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.imports
                .update(cx, |p, cx| p.undo_import(newest, window, cx))
        });
    });
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).imports.read(cx).confirm_undo_import,
            Some(newest),
            "the first click only arms the confirmation"
        );
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    let (above, below) = clearance(btn(&mut window, UNDO_LABEL_MAX), cell(cx, handle, 0));
    assert!(
        above >= 4. && below >= 4.,
        "armed: {above} above, {below} below"
    );
    let cells: Vec<_> = (0..3).map(|r| cell(cx, handle, r)).collect();
    assert_eq!(
        cells[0].size.height, cells[1].size.height,
        "the armed newest row is no taller than the others"
    );
    assert!(
        btn(&mut window, UNDO_LABEL_MAX).size.width + px(CELL_PAD_X) <= col_w,
        "the armed label overflows the Undo column: button {:?} + {CELL_PAD_X} > {col_w:?}",
        btn(&mut window, UNDO_LABEL_MAX).size.width,
    );
    // The header Undo shares the confirmation state: same moment, armed text.
    assert!(
        window
            .debug_bounds("imports:header:Click again to undo:icons/undo-2.svg")
            .is_some(),
        "the header Undo arms with the in-row one"
    );
}

/// 3.37: the in-row Undo button shows the `undo-2` icon plus the label
/// "Undo" — 3.32's icon-only button read as unclear. The selector records
/// the label it was built with; the drawn width proves the icon paints
/// beside it; and the labelled button still fits the Undo column at rest.
#[gpui::test]
fn the_in_row_undo_button_shows_the_undo_label(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.read(cx).import_history.len() == 3
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // At rest the button reads "Undo".
    let idle = window
        .debug_bounds(leaked("history:undo-btn:Undo".into()))
        .expect("the newest row's Undo button reads \"Undo\"");
    // And it paints the icon beside the label: the drawn width is the
    // label plus the xsmall icon block and the button's padding, so a
    // dropped `.icon()` call — 16 px narrower — fails here.
    let mut refs = open_icon_width_refs(cx, vec![("ref:undo-xs", "Undo", true)]);
    assert_button_paints_icon(
        &mut refs,
        "the idle in-row Undo",
        idle,
        "ref:undo-xs",
        UNDO_ICON_W,
        BUTTON_PAD_X,
    );
    // With its label the idle button still fits the Undo column.
    let col_w = cx.update(|cx| {
        shell
            .read(cx)
            .imports
            .read(cx)
            .history_table
            .read(cx)
            .col_width(5)
            .unwrap()
    });
    assert!(
        idle.size.width + px(CELL_PAD_X) <= col_w,
        "the idle label overflows the Undo column: {} + {CELL_PAD_X} > {col_w:?}",
        idle.size.width
    );
}

/// 3.32: the Imports header buttons carry their Lucide icon left of the
/// label, and no label ends in an ellipsis — the action runs straight away,
/// and any confirmation is the button's own armed state. Each button's
/// selector records the exact label it was built with and the icon asset it
/// draws; the old "…" labels draw nothing.
#[gpui::test]
fn the_imports_header_buttons_show_icons_without_ellipses(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.read(cx).import_history.len() == 3
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for sel in [
        "imports:header:Import CSV:icons/file-up.svg",
        "imports:header:Undo:icons/undo-2.svg",
        "imports:header:Export:icons/download.svg",
        "imports:header:Restore:icons/archive-restore.svg",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the header draws {sel:?}"
        );
    }
    for sel in [
        "imports:header:Import CSV…:icons/file-up.svg",
        "imports:header:Undo…:icons/undo-2.svg",
        "imports:header:Export…:icons/download.svg",
        "imports:header:Restore…:icons/archive-restore.svg",
    ] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "no header label ends in an ellipsis ({sel:?})"
        );
    }

    // Pin the icons for real (the selectors above only record how each
    // button was built): a button's drawn width must be its label plus
    // the medium icon block and the button's padding, so dropping an
    // `.icon()` call — 24 px narrower — fails here.
    let mut refs = open_icon_width_refs(
        cx,
        vec![
            ("ref:import-csv", "Import CSV", false),
            ("ref:undo", "Undo", false),
            ("ref:export", "Export", false),
            ("ref:restore", "Restore", false),
            ("ref:click-again", "Click again to undo", true),
        ],
    );
    for (sel, text_sel) in [
        (
            "imports:header:Import CSV:icons/file-up.svg",
            "ref:import-csv",
        ),
        ("imports:header:Undo:icons/undo-2.svg", "ref:undo"),
        ("imports:header:Export:icons/download.svg", "ref:export"),
        (
            "imports:header:Restore:icons/archive-restore.svg",
            "ref:restore",
        ),
    ] {
        let b = window
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("the header draws {sel:?}"));
        assert_button_paints_icon(
            &mut refs,
            sel,
            b,
            text_sel,
            HEADER_ICON_BLOCK_W,
            HEADER_BUTTON_PAD_X,
        );
    }

    // The shared confirm, seen from the header: the header button's own
    // first click arms it, and the in-row Undo reflects it — the armed
    // in-row button swaps its idle "Undo" label for the prompt together
    // with its icon (width as above).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.undo_last_import(window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds("imports:header:Click again to undo:icons/undo-2.svg")
            .is_some(),
        "the header button arms the confirm"
    );
    let armed = window
        .debug_bounds(leaked(format!("history:undo-btn:{UNDO_LABEL_MAX}")))
        .expect("the newest row draws its Undo button");
    assert_button_paints_icon(
        &mut refs,
        "the armed in-row Undo",
        armed,
        "ref:click-again",
        UNDO_ICON_W,
        BUTTON_PAD_X,
    );
}

/// 3.32: the four icons the Imports page draws are bundled with the app and
/// resolve through its asset source, the way the check glyph (3.14) must.
#[test]
fn the_imports_icons_load_from_the_bundled_assets() {
    use gpui_kit::AssetSource as _;
    let icons = [
        IconName::FileUp,
        IconName::Undo2,
        IconName::Download,
        IconName::ArchiveRestore,
    ];
    for icon in icons {
        let path = icon.path();
        let svg = crate::icon_assets()
            .load(&path)
            .ok()
            .flatten()
            .unwrap_or_else(|| panic!("the app bundles the {path} icon"));
        assert!(
            std::str::from_utf8(&svg).is_ok_and(|s| s.contains("<svg")),
            "{path} is an SVG"
        );
    }
}

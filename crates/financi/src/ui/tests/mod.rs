//! Headless UI tests: real views in a test window and a real DbWorker
//! thread, driven exactly as a user drives the app (3.62 split them by page;
//! one `mod` per page module plus the shared helpers here).

use self::dashboard::dash;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate};
use financi_core::dates::{DatePreset, preset_range};
use financi_core::parse::merchant;
use financi_core::rules::Rule;
use financi_core::{AccountKind, CategoryKind};
use financi_db::import::TransactionRow;
use financi_db::settings::{self, DateRange, UI_DATE_RANGE, UI_INCLUDE_TRANSFERS};
use financi_db::{
    CategorySpend, DbWorker, TxSort, TxSortColumn, ViewFilter, create_account, open_in_memory,
};
use gpui_kit::component::table::{ColumnSort, TableDelegate};
// No glob import: `gpui_kit::*` brings GPUI's `test` macro into scope, which
// shadows the built-in #[test] that #[gpui::test] expands to.
use gpui_kit::assets::IconName;
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Entity, Focusable as _, Modifiers, OwnedMenuItem,
    Pixels, Point, Render, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, div, point, px, size,
};

use super::dashboard_chart::{
    BAR_LABEL_MAX_CHARS, CHART_LABEL_GAP, CHART_LABEL_MAX_CHARS, CHART_LABEL_MIN_CHARS,
    band_label_budget, bar_label_plan, bar_label_slots, bar_share_label, chart_tooltip_value,
    short_chart_labels,
};
use super::{
    BUTTON_PAD_X, CELL_PAD_X, CHART_CHAR_PX, DashboardBody, ExportData, HISTORY_DATES_W,
    HISTORY_FLEX_MIN_W, HISTORY_ROWS_W, HISTORY_TIME_W, ImportCsv, ImportPanel, LEGEND_W,
    LegendSelection, NEW_ACCOUNT, NEW_CATEGORY, NEW_RULE, PAGE_PAD, Page, PendingCatWrite,
    PendingRuleAction, PlanState, RestoreBackup, RowMenuItem, RowShowRule, SIDEBAR_W,
    SUBCATEGORY_PH, Shell, SummaryRowKind, TABLE_BORDER_X, TEXT_SM_CHAR_PX, TX_FIXED_W,
    UNDO_ICON_W, UNDO_LABEL_MAX, UndoLastImport, history_flex_widths, history_undo_col_w,
    row_menu_items, row_show_rule, tx_flex_widths,
};
use crate::ui::sidebar::NAV_ROW_INSET;

mod catalog;
mod categories;
mod dashboard;
mod imports;
mod page;
mod queue;
mod reloads;
mod rules;
mod sidebar;
mod transfers;
mod widgets;

fn demo(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/demo")
        .join(name)
}

/// Open the shell over a fresh in-memory database prepared by `setup`.
fn open_shell(
    cx: &mut TestAppContext,
    setup: impl FnOnce(&financi_db::rusqlite::Connection),
) -> (AnyWindowHandle, Entity<Shell>) {
    // A real path beside a scratch directory so master CSV writes during
    // import/undo never land in the repository. The database itself stays
    // in memory. 3.75: one directory per test — the tests of one binary
    // share a process and run in parallel threads, and a shared path let
    // them write the same master CSVs behind each other's back.
    open_shell_at(cx, setup, temp_db_path())
}

/// A scratch DB path unique to the caller (3.75).
fn temp_db_path() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("financi-ui-db-{pid}-{n}/financi.sqlite"))
}

/// `open_shell` with a chosen database path (3.38: the footer tooltip
/// test uses a path longer than the sidebar). The database itself stays
/// in memory.
fn open_shell_at(
    cx: &mut TestAppContext,
    setup: impl FnOnce(&financi_db::rusqlite::Connection),
    db_path: PathBuf,
) -> (AnyWindowHandle, Entity<Shell>) {
    let conn = open_in_memory().unwrap();
    setup(&conn);
    open_shell_with_conn(cx, conn, db_path)
}

/// Open the shell over a real database file at a unique path (5.3):
/// restore refuses an in-memory database, so the export and restore
/// flows need the on-disk one. Its `csv/` tree lands beside the file,
/// inside the same per-test scratch directory.
fn open_shell_on_file(
    cx: &mut TestAppContext,
    setup: impl FnOnce(&financi_db::rusqlite::Connection),
) -> (AnyWindowHandle, Entity<Shell>) {
    let db_path = temp_db_path();
    let conn = financi_db::open(&db_path).unwrap();
    setup(&conn);
    open_shell_with_conn(cx, conn, db_path)
}

/// The window half of the `open_shell*` helpers: init, spawn the worker
/// over `conn` and open the shell as the app does.
fn open_shell_with_conn(
    cx: &mut TestAppContext,
    conn: financi_db::rusqlite::Connection,
    db_path: PathBuf,
) -> (AnyWindowHandle, Entity<Shell>) {
    // The DB worker answers from its own OS thread.
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    // 3.76: the global wiring is the app's, not the Shell's — the real
    // main() does this once before the first window, and every test App
    // needs it too (idempotent, so a second shell in one App is fine).
    cx.update(crate::ui::init);
    let db = DbWorker::spawn(conn);
    cx.update(|cx| {
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1200.), px(720.)),
        };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Shell::new(db_path, db, window, cx)),
        )
        .unwrap();
        (window, shell)
    })
}

/// Drive the executor until `done` holds, up to 10 s of real time (the DB
/// worker runs on a real thread).
fn wait_until(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if cx.update(|cx| done(cx)) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 3.75: Wait until the shell has settled — the DB worker has nothing in
/// flight and every started refresh has had its result applied. The worker
/// runs its jobs in order, so a no-op job completing proves the refresh's
/// own query finished too, and the counter check covers the continuation
/// that applies the data. Use this before asserting that something is
/// *not* shown: `run_until_parked` alone returns while the worker is still
/// mid-query, so the "still empty" check would race it.
fn wait_idle(cx: &mut TestAppContext, shell: &Entity<Shell>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let db = shell.update(cx, |s, _| s.db.clone());
        let _ = block_on(db.call(|_| ()));
        // 3.78: partial reloads run as keyed *read* jobs; drain the reader
        // queue too (in-memory test databases have none, and this is a
        // no-op there).
        let _ = block_on(db.call_read(|_| ()));
        cx.run_until_parked();
        let (started, applied) = shell.update(cx, |s, _| {
            (s.refresh_started.get(), s.refresh_applied.get())
        });
        if started == applied {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the shell to go idle (started {started}, applied {applied})"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn with_window<R>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    cx.update_window(handle, |_, window, cx| f(window, cx))
        .unwrap()
}

fn panel(shell: &Entity<Shell>, cx: &App) -> Entity<ImportPanel> {
    shell
        .read(cx)
        .imports
        .read(cx)
        .import
        .as_ref()
        .map(|(panel, _)| panel.clone())
        .expect("import panel open")
}

fn plan_ready(shell: &Entity<Shell>, cx: &App) -> bool {
    shell
        .read(cx)
        .imports
        .read(cx)
        .import
        .as_ref()
        .is_some_and(|(panel, _)| matches!(panel.read(cx).plan, PlanState::Ready(_)))
}

/// Open a file's import panel, wait until its account list offers
/// `account` (it loads from the DB worker), then choose it.
fn open_and_choose(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    file: &str,
    account: &str,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.open_file(demo(file), window, cx));
    });
    let offered = format!("{account} offered in the account picker");
    wait_until(cx, &offered, |cx| {
        panel(shell, cx).read(cx).names.iter().any(|n| n == account)
    });
    with_window(cx, handle, |_, cx| {
        panel(shell, cx).update(cx, |p, cx| p.choose(account.into(), cx));
    });
}

fn click_import(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>) {
    with_window(cx, handle, |_, cx| {
        let p = panel(shell, cx);
        // The Import button is disabled exactly when can_import() is false.
        if p.read(cx).can_import() {
            p.update(cx, |p, cx| p.import(cx));
        }
    });
}

/// 3.27: the toasts mounted in the bottom-right overlay, oldest first, as
/// `(text, is_error)` — including any mid-exit. The feedback assertion of
/// choice now that `Shell::status` is gone. Toast timers only run when the
/// test advances the executor clock, so a toast pushed earlier in a test
/// stays mounted until then.
fn toasts(shell: &Entity<Shell>, cx: &App) -> Vec<(String, bool)> {
    shell
        .read(cx)
        .toasts
        .iter()
        .map(|(_, toast, _)| (toast.message.to_string(), toast.is_error))
        .collect()
}

/// The newest mounted toast, if any.
fn last_toast(shell: &Entity<Shell>, cx: &App) -> Option<(String, bool)> {
    toasts(shell, cx).pop()
}

/// Advance the toast ticker by `ticks` intervals of 100 ms. `advance_clock`
/// readies a self-re-arming timer once per call, so tests drive the ticker
/// tick by tick instead of advancing seconds at once.
fn tick_toasts(cx: &mut TestAppContext, ticks: u32) {
    for _ in 0..ticks {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
}

/// Seed one import whose file name and account name are both far longer
/// than any column, the worst case for the history table.
fn seed_long_history(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(
        conn,
        "simplii",
        "Simplii Chequing at Home and Abroad Forever",
        AccountKind::Chequing,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'simplii-chequing',
                 'simplii-chequing-statement-2026-10-to-2026-11-final-v2.csv',
                 '2026-11-06T10:00:00', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
}

/// `fixtures/demo/expected.json` — the reference totals, computed outside
/// financi by a Python script.
fn expected_totals() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo/expected.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Drive one demo file through the whole flow: open it, create a new
/// account named `account`, and click Import.
fn import_demo_file(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    file: &str,
    account: &str,
) {
    open_and_choose(cx, handle, shell, file, NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        let p = panel(shell, cx);
        p.update(cx, |p, cx| {
            p.new_name.update(cx, |input, cx| {
                input.set_value(account, window, cx);
            });
            p.create_account(window, cx);
        });
    });
    wait_until(cx, "plan ready after naming the account", |cx| {
        plan_ready(shell, cx)
    });
    click_import(cx, handle, shell);
    wait_until(cx, "import finished", |cx| {
        shell.read(cx).imports.read(cx).import.is_none()
    });
}

/// 3.25: resize to `w` × `h` and paint, so `debug_bounds` reports the
/// geometry of that size (the alignment pass is tested at the minimum and
/// at a large window).
fn resize_and_draw(cx: &mut TestAppContext, handle: AnyWindowHandle, w: f32, h: f32) {
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(w), px(h)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
}

/// 3.75: every debug selector the app emits follows `page:thing[:id]`.
/// gpui keeps a rendered frame's selector map private (`Window`'s
/// `debug_bounds` field is `pub(crate)`), so the walk runs over the
/// source the frames are built from: every string literal inside a
/// `.debug_selector(…)` call in the view code (`ui/*.rs`; test-only
/// selectors are the tests' business). Selectors that reach a widget
/// through a helper (`control_slot`, `month_viewport`) carry no literal
/// at the `debug_selector` — the walk checks the literals those helpers
/// receive at their call sites instead. A selector is at least two
/// colon-separated segments, lowercase kebab-case with `{…}` marking an
/// interpolated id, and its first segment must be one of the page (or
/// shared-chrome) namespaces below — a stray `rule:`, a typo or a
/// widget id mistaken for a selector fails the walk.
#[test]
fn selectors_follow_the_page_thing_id_pattern() {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("ui");
    let mut files: Vec<_> = std::fs::read_dir(&ui_dir)
        .expect("the view directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    let mut checked = 0usize;
    let mut bad: Vec<String> = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // `.debug_selector(` contributes one selector; the helpers take
        // their selectors as literal arguments at the call site.
        for (needle, expect) in [
            (".debug_selector(", 1),
            ("control_slot(", 2),
            ("month_viewport(", 1),
        ] {
            let mut at = 0usize;
            while let Some(found) = text[at..].find(needle) {
                let open = at + found + needle.len();
                let lits = call_literals(&text, open, expect);
                for (sel, pos) in lits {
                    checked += 1;
                    if !selector_ok(&sel) {
                        let line = text[..pos].matches('\n').count() + 1;
                        bad.push(format!("{name}:{line}: {sel}"));
                    }
                }
                // Keep scanning inside the call: arguments nest their own
                // `debug_selector` calls (tooltips, slot controls).
                at = open;
            }
        }
    }
    assert!(
        bad.is_empty(),
        "selectors must follow page:thing[:id], fix:\n{}",
        bad.join("\n")
    );
    assert!(
        checked > 100,
        "the walk found only {checked} selectors — did it parse the view at all?"
    );
}

/// The namespaces the view's selectors live in: one per page, plus the
/// shared chrome (`label` region/field titles, `panel` wrappers, `page`
/// content root, `toast` popups). New pages register here.
const PAGE_NAMESPACES: &[&str] = &[
    "accounts",
    "cat",
    "chart",
    "dashboard",
    "filters",
    "history",
    "imports",
    "label",
    "page",
    "panel",
    "queue",
    "rules",
    "sidebar",
    "summary",
    "toast",
    "transfers",
];

/// The first `max` string literals written inside the call whose open
/// paren ends at `open`, paired with their source positions. String- and
/// escape-aware: parens and quotes inside strings do not count, and a
/// call with no literal (`debug_selector(move || selector.clone())`)
/// yields nothing instead of grabbing a literal from later code.
fn call_literals(text: &str, open: usize, max: usize) -> Vec<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 1usize;
    let mut i = open;
    while i < bytes.len() && depth > 0 && out.len() < max {
        match bytes[i] {
            b'"' => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != b'"' {
                    if bytes[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                out.push((text[start..j.min(text.len())].to_string(), start));
                i = j + 1;
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// `page:thing[:id]`: two or more colon-separated segments of lowercase
/// kebab-case letters, digits and hyphens, where `{…}` stands for an
/// interpolated id. The first segment is the namespace and must be a
/// registered page (or shared-chrome) namespace, never interpolated.
fn selector_ok(sel: &str) -> bool {
    let mut parts = sel.split(':');
    let Some(namespace) = parts.next() else {
        return false;
    };
    if !PAGE_NAMESPACES.contains(&namespace) {
        return false;
    }
    let mut saw_segment = false;
    for part in parts {
        saw_segment = true;
        if part.is_empty() {
            return false;
        }
        let mut chars = part.char_indices().peekable();
        while let Some((ix, c)) = chars.next() {
            if c == '{' {
                let Some(close) = part[ix..].find('}') else {
                    return false;
                };
                if !part[ix + 1..ix + close]
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c == '_')
                {
                    return false;
                }
                // `{` was consumed above; skip the placeholder body and `}`.
                for _ in 0..close {
                    chars.next();
                }
            } else if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return false;
            }
        }
    }
    saw_segment
}

/// 3.43: no button label ends in "…" — a button names its action with an
/// icon left of the label instead (ui-conventions, *Buttons*). The walk
/// reads the view sources (`ui/*.rs`; test code is the tests' business)
/// and fails on a string literal ending in "…" inside a `.label(…)` call,
/// or on a `const NAME: &str = "…"` that is passed to `.label(NAME)`.
/// Select placeholders (`.placeholder(…)`) and picker rows are not
/// buttons, so the walk ignores them. The one exception is the empty
/// dashboard's *Restore* **link**: a link, not a button.
#[test]
fn no_button_label_ends_in_an_ellipsis() {
    const ALLOWED: &[&str] = &["Restore from export…"];
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("ui");
    let mut files: Vec<_> = std::fs::read_dir(&ui_dir)
        .expect("the view directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    let texts: Vec<(String, String)> = files
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            (p.file_name().unwrap().to_string_lossy().into_owned(), text)
        })
        .collect();
    let mut bad: Vec<String> = Vec::new();
    let mut checked = 0usize;
    // Every `const NAME: &str = "…";`, checked against the uses below.
    let mut ellipsis_consts: Vec<(String, String)> = Vec::new();
    for (name, text) in &texts {
        // Literal labels: every string literal inside a `.label(…)` call.
        let mut at = 0usize;
        while let Some(found) = text[at..].find(".label(") {
            let open = at + found + ".label(".len();
            for (lit, pos) in call_literals(text, open, 4) {
                checked += 1;
                if lit.ends_with('…') && !ALLOWED.contains(&lit.as_str()) {
                    let line = text[..pos].matches('\n').count() + 1;
                    bad.push(format!("{name}:{line}: .label({lit:?})"));
                }
            }
            at = open;
        }
        // `const NAME: &str = "…";` definitions (any visibility).
        let mut at = 0usize;
        while let Some(found) = text[at..].find("const ") {
            let start = at + found + "const ".len();
            let rest = &text[start..];
            let Some(q) = rest.find('"') else { break };
            let Some((nm, ty)) = rest[..q].trim_end().split_once(':') else {
                at = start;
                continue;
            };
            let nm = nm.trim();
            let named = !nm.is_empty()
                && nm
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit());
            // Between the `:` and the string: ` &str =` / ` &'static str =`
            // for a `const NAME: &str = "…";`; anything else (an `f32`, a
            // prose "const " in a doc comment) is not a label const.
            let ty = ty.trim().trim_end_matches('=').trim_end();
            if !named || (ty != "&str" && ty != "&'static str") {
                at = start;
                continue;
            }
            let lit_start = start + q + 1;
            let bytes = text.as_bytes();
            let mut j = lit_start;
            while j < bytes.len() && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            if text[lit_start..j.min(text.len())].ends_with('…') {
                ellipsis_consts.push((nm.to_string(), name.clone()));
            }
            at = start;
        }
    }
    // A "…" const used as a label fails wherever it is used; the same
    // const as a placeholder or a picker row is none of this guard's
    // business.
    for (nm, defined) in &ellipsis_consts {
        for (name, text) in &texts {
            if let Some(pos) = text.find(&format!(".label({nm})")) {
                let line = text[..pos].matches('\n').count() + 1;
                bad.push(format!(
                    "{name}:{line}: .label({nm}) — {defined} defines it with a trailing \"…\""
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "no button label may end in \"…\" (3.43: an icon replaces it), fix:\n{}",
        bad.join("\n")
    );
    assert!(
        checked > 40,
        "the walk found only {checked} .label literals — did it parse the view at all?"
    );
}

/// 3.75: the painted bounds of a debug selector, panicking with its name
/// when the element did not paint at all. The one bounds helper — call
/// this instead of reaching for `debug_bounds` per test.
fn bounds(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    sel: impl Into<String>,
) -> Bounds<Pixels> {
    let sel = sel.into();
    let mut window = VisualTestContext::from_window(handle, cx);
    window
        .debug_bounds(leaked(sel.clone()))
        .unwrap_or_else(|| panic!("{sel} paints"))
}

/// The centre-line y of a selector (see `bounds`).
fn centre_y(cx: &mut TestAppContext, handle: AnyWindowHandle, sel: impl Into<String>) -> f32 {
    let b = bounds(cx, handle, sel);
    f32::from(b.origin.y) + f32::from(b.size.height) / 2.
}

/// 3.75: a real click at the centre of the widget's painted bounds — the
/// user's own path, through the widget's click handler. The selector must
/// paint in a real frame, so this paints one first; afterwards the
/// executor is parked, so handlers and the state changes they start have
/// run (wait for the state itself, or `wait_idle`, before asserting on it).
fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, sel: impl Into<String>) {
    let sel = sel.into();
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    let b = window
        .debug_bounds(leaked(sel.clone()))
        .unwrap_or_else(|| panic!("{sel} paints"));
    window.simulate_click(b.center(), Modifiers::default());
    cx.run_until_parked();
}

/// Type a parent name, pick the Expense kind (index 0) and add it.
fn add_parent(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>, name: &str) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.new_parent_name.update(cx, |i, cx| {
                    i.set_value(name, window, cx);
                });
            });
            s.add_parent(window, cx);
        });
    });
    let what = format!("parent {name}");
    wait_until(cx, &what, |cx| {
        shell.read(cx).categories.iter().any(|p| p.name == name)
    });
}

/// Add a child under the currently selected parent.
fn add_child(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>, name: &str) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.new_child_name.update(cx, |i, cx| {
                    i.set_value(name, window, cx);
                });
            });
            s.add_child(window, cx);
        });
    });
    let what = format!("child {name}");
    wait_until(cx, &what, |cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .any(|p| p.children.iter().any(|c| c.name == name))
    });
}

fn find_child(shell: &Entity<Shell>, cx: &App, name: &str) -> Option<i64> {
    shell
        .read(cx)
        .categories
        .iter()
        .find_map(|p| p.children.iter().find(|c| c.name == name).map(|c| c.id))
}

/// The busiest queue group as `(merchant, count)`. 3.83: read off the
/// `Catalog` snapshot — the To-categorize page is an entity that defers
/// applying while hidden, but the snapshot is always current.
fn top_group(shell: &Entity<Shell>, cx: &App) -> (String, i64) {
    shell
        .read(cx)
        .catalog
        .read(cx)
        .snap
        .queue
        .first()
        .map(|g| (g.merchant.clone(), g.count))
        .expect("queue has a group")
}

// -- rule edit & delete (3.24) -------------------------------------------------

/// 3.24 fixture: one account, two child categories and three transactions,
/// plus a rule that puts `COFFEE` rows on `first` (added through
/// `add_rule`, so the two unlocked rows are categorized by it). "COFFEE
/// SHOP LOCKED" is then manually overridden to `second` and locked — a
/// rule re-run must never touch it (2.6). Returns `(rule, first, second)`.
fn seed_rule_fixture(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let first = financi_db::categories::create_child(conn, drinks, "First").unwrap();
    let second = financi_db::categories::create_child(conn, drinks, "Second").unwrap();
    for (n, desc) in ["COFFEE SHOP A", "COFFEE SHOP B", "COFFEE SHOP LOCKED"]
        .iter()
        .enumerate()
    {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
            (acct, import, n as i64, format!("rule-edit-hash-{n}"), *desc),
        )
        .unwrap();
    }
    let rule = financi_db::rules::add_rule(conn, "COFFEE", false, first).unwrap();
    conn.execute(
        "UPDATE transactions SET category_id = ?1, category_locked = 1 \
         WHERE description_raw = 'COFFEE SHOP LOCKED'",
        [second],
    )
    .unwrap();
    (rule, first, second)
}

/// Open the shell over `seed_rule_fixture`, wait until the rule, the two
/// child categories and the counts have loaded, and switch to Categories.
fn open_rule_fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Shell>, i64, i64, i64) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_rule_fixture(c));
    let (rule, first, second) = ids;
    wait_until(cx, "rule fixture loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 1
            && s.child_choices.iter().any(|(id, _)| *id == first)
            && s.child_choices.iter().any(|(id, _)| *id == second)
            && s.cat_counts.get(&first) == Some(&2)
            && s.cat_counts.get(&second) == Some(&1)
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: the page lists only the selected category's rules, so the
    // fixture selects the rules' parent to put them on screen.
    let drinks = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.children.iter().any(|c| c.id == first))
            .map(|p| p.id)
            .unwrap()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(drinks, window, cx));
    });
    cx.run_until_parked();
    (handle, shell, rule, first, second)
}

/// Click a rule row's Edit — the same `start_edit_rule` the button calls.
fn start_rule_edit(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    rule: i64,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.start_edit_rule(rule, window, cx))
        });
    });
}

/// 3.12: synthetic data for the long-list tests — 2,000 parents with one
/// child each (4,000 tree rows) and 2,000 rules, inserted directly.
fn seed_thousands(conn: &financi_db::rusqlite::Connection) {
    // 3.23: the page lists only the selected category's rules, so the
    // long-list tests need them all under one: every rule targets the
    // first subcategory, under "Cat 0000".
    let mut first_child = 0i64;
    for i in 0..2_000 {
        conn.execute(
            "INSERT INTO categories (name, kind) VALUES (?1, 'expense')",
            financi_db::rusqlite::params![format!("Cat {i:04}")],
        )
        .unwrap();
        let parent = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO categories (name, parent_id, kind) VALUES (?1, ?2, 'expense')",
            financi_db::rusqlite::params![format!("Kid {i:04}"), parent],
        )
        .unwrap();
        let child = conn.last_insert_rowid();
        if i == 0 {
            first_child = child;
        }
        conn.execute(
            "INSERT INTO category_rules (pattern, category_id) VALUES (?1, ?2)",
            financi_db::rusqlite::params![format!("PAT{i:04}"), first_child],
        )
        .unwrap();
    }
}

/// 3.23: select a top-level category by name — the same `select_category`
/// call `tree_row` makes. The page's rule list only paints with a
/// selection, so most Categories tests start here.
fn select_parent(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    name: &str,
) {
    let id = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no parent category {name:?}"))
            .id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(id, window, cx));
    });
    cx.run_until_parked();
}

/// Draw one real frame so the virtualized lists lay out and build rows.
fn draw_frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    with_window(cx, handle, |window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// 3.86: draw until the bounds probes have settled. A probe reports in
/// prepaint through `window.defer`, so each report needs a completed
/// frame *and* a `run_until_parked` to land; the notify it raises wants
/// one more frame, whose probe reports the same width and stays quiet.
/// Three rounds covers probe → store → repaint → stable.
fn settle(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    for _ in 0..3 {
        draw_frame(cx, handle);
        cx.run_until_parked();
    }
}

/// 3.21: wheel `dx` × `dy` pixels over a point — `v_virtual_list` scrolls
/// under the mouse, so tests aim at the centre of the list handle's
/// viewport. Sideways scrolling is `dx` (3.30: on Linux Shift+wheel
/// arrives as a horizontal delta, which is what the month strip scrolls on).
fn wheel(cx: &mut TestAppContext, handle: AnyWindowHandle, at: Point<Pixels>, dx: f32, dy: f32) {
    let mut window = VisualTestContext::from_window(handle, cx);
    window.simulate_event(ScrollEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
        ..Default::default()
    });
}

/// The centre of a window-space rectangle's viewport, to aim `wheel`.
fn viewport_centre(b: Bounds<Pixels>) -> Point<Pixels> {
    point(
        b.origin.x + b.size.width / 2.0,
        b.origin.y + b.size.height / 2.0,
    )
}

/// Does a widget with this debug selector paint in the last frame?
fn paints(cx: &mut TestAppContext, handle: AnyWindowHandle, sel: impl Into<String>) -> bool {
    let sel = sel.into();
    let mut window = VisualTestContext::from_window(handle, cx);
    window.debug_bounds(leaked(sel)).is_some()
}

// --------------------------------------------------- 3.43 button icon widths

/// Horizontal space a medium `Button` draws around its label once it has
/// an icon: the medium icon (`size_4`, 16 px) plus the `gap_2` (8 px)
/// between icon and label, and the button's own `px_2p5` per side — the
/// same values the 3.32 Imports tests pin.
pub(crate) const MEDIUM_ICON_BLOCK_W: f32 = 24.;

pub(crate) const MEDIUM_BUTTON_PAD_X: f32 = 20.;

/// Slack for comparing a drawn button width against label + icon +
/// padding: layout rounds to physical pixels. Tight enough that a
/// dropped `.icon()` call — 24 px on these buttons — fails.
pub(crate) const ICON_WIDTH_TOL: f32 = 3.;

/// 3.43: a window of plain labels at the text size a medium button
/// draws (`text_sm` — the button sets it on its own content, so a
/// `.text_xs()` on the row buttons does not change it), so a test can
/// pin that a button's drawn width is its label plus exactly the icon
/// block and padding. A debug selector records how a button was built;
/// only its width shows the icon actually paints.
struct IconTextRefs {
    labels: Vec<(&'static str, &'static str)>,
}

impl Render for IconTextRefs {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex().children(self.labels.iter().map(|(sel, text)| {
            // `self_start` so the label's width is its text, not the
            // full window (columns stretch their children by default).
            div()
                .self_start()
                .text_sm()
                .child(*text)
                .debug_selector(move || (*sel).to_string())
        }))
    }
}

/// Open the reference-label window (after `open_shell`, so gpui-kit is
/// initialized) and draw one frame; the returned context reads the
/// labels' drawn widths.
fn open_icon_text_refs(
    cx: &mut TestAppContext,
    labels: Vec<(&'static str, &'static str)>,
) -> VisualTestContext {
    let handle: AnyWindowHandle = cx.add_window(|_, _| IconTextRefs { labels }).into();
    draw_frame(cx, handle);
    VisualTestContext::from_window(handle, cx)
}

/// Assert a drawn button's width is its label's drawn width plus the
/// icon block and horizontal padding (3.43) — the icon must paint.
#[track_caller]
fn assert_button_paints_icon(
    refs: &mut VisualTestContext,
    what: &str,
    btn: Bounds<Pixels>,
    text_sel: &'static str,
    icon_block_w: f32,
    pad_x: f32,
) {
    let text_w = refs
        .debug_bounds(leaked(text_sel.into()))
        .unwrap_or_else(|| panic!("the reference window draws {text_sel:?}"))
        .size
        .width
        .as_f32();
    let got = btn.size.width.as_f32();
    let want = text_w + icon_block_w + pad_x;
    assert!(
        (got - want).abs() <= ICON_WIDTH_TOL,
        "{what}: drawn width {got} is not its label {text_w} + icon block \
         {icon_block_w} + padding {pad_x} (± {ICON_WIDTH_TOL}) — is the icon drawn?",
    );
}

/// 3.11: after a page switch only the current page paints. Walks every page,
/// then opens the accounts popover with a real click and switches away: the
/// popover must not stay painted, nor come back open.
#[gpui::test]
fn page_switches_paint_only_the_current_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    let mut window = VisualTestContext::from_window(handle, cx);

    // Everything a painted frame may reveal: the page-area selector and
    // two page-specific widgets. Exactly one page root may appear per frame.
    const PAINTED: [&str; 5] = [
        "page:page-dashboard",
        "page:page-queue",
        "page:page-categories",
        "page:page-rules",
        // 3.65: the account checkbox's selector carries its checked state;
        // the seeded account is checked.
        "checkbox:account-1:checked",
    ];
    let painted = |window: &mut VisualTestContext| -> Vec<&'static str> {
        PAINTED
            .iter()
            .copied()
            .filter(|s| window.debug_bounds(s).is_some())
            .collect()
    };
    let expect = |page: Page| -> Vec<&'static str> {
        match page {
            Page::Dashboard => vec!["page:page-dashboard"],
            Page::ToCategorize => vec!["page:page-queue"],
            Page::Categories => vec!["page:page-categories"],
            Page::Rules => vec!["page:page-rules"],
            Page::Transfers | Page::Imports => vec![],
        }
    };

    for page in [
        Page::Dashboard,
        Page::ToCategorize,
        Page::Categories,
        Page::Rules,
        Page::Transfers,
        Page::Imports,
        Page::Dashboard,
    ] {
        cx.update(|cx| shell.update(cx, |s, cx| s.go_page(page, cx)));
        draw_frame(cx, handle);
        assert_eq!(
            painted(&mut window),
            expect(page),
            "painted after switching to {page:?}"
        );
    }

    // The accounts popover (Dashboard content) opened with a real click:
    // switching away must close it, and returning must not show it open.
    click(cx, handle, "accounts:filter");
    draw_frame(cx, handle);
    assert!(
        painted(&mut window).contains(&"checkbox:account-1:checked"),
        "the popover opened on the Dashboard"
    );
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    draw_frame(cx, handle);
    assert_eq!(
        painted(&mut window),
        expect(Page::Categories),
        "the popover is gone after switching pages"
    );
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    draw_frame(cx, handle);
    assert_eq!(
        painted(&mut window),
        expect(Page::Dashboard),
        "the popover does not come back open"
    );
}

/// 3.11: `go_page` closes the transient UI it owns — the two-click
/// confirmations, the rule offer and the inline rename editor. 3.51: the
/// detail panel's inline form joins that list.
#[gpui::test]
fn go_page_closes_transient_ui(cx: &mut TestAppContext) {
    use crate::ui::categories::DetailForm;
    let (_handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.imports.update(cx, |p, _| p.confirm_undo_import = Some(1));
            s.categories_view.update(cx, |p, _| {
                p.renaming = true;
                p.confirm_delete_category = Some(1);
                p.detail_form = Some(DetailForm::Move);
            });
        });
        // 3.82: the rule offer lives on the Dashboard entity now, and only
        // that entity may be borrowed at a time.
        dash(cx, &shell).update(cx, |d, _| d.rule_offer = Some((1, 2)));
        shell.update(cx, |s, cx| s.go_page(Page::Categories, cx));
        let s = shell.read(cx);
        assert!(
            s.imports.read(cx).confirm_undo_import.is_none(),
            "undo confirmation cleared"
        );
        assert!(
            dash(cx, &shell).read(cx).rule_offer.is_none(),
            "rule offer cleared"
        );
        assert!(!s.categories_view.read(cx).renaming, "rename editor closed");
        assert!(
            s.categories_view.read(cx).confirm_delete_category.is_none(),
            "delete confirmation cleared"
        );
        assert!(
            s.categories_view.read(cx).detail_form.is_none(),
            "the open detail form is closed"
        );
    });
}

/// 3.23 fit-test names: long enough that nothing but truncation fits.
const CAT_FIT_PARENT: &str = "Food, Groceries & Dining Out On Both Sides Of The Boulevard Downtown";
const CAT_FIT_CHILD: &str = "Restaurants, Cafés, Takeaway & Late-Night Snack Stands Far Uptown";
const CAT_FIT_PATTERN: &str = "LONG RULE PATTERN THAT WILL NOT FIT THE RULE ROW AT THE MINIMUM WINDOW SIZE SO IT MUST TRUNCATE";

/// One long-named parent and subcategory with one long-pattern rule.
fn seed_categories_fit(conn: &financi_db::rusqlite::Connection) -> (i64, i64) {
    let parent =
        financi_db::categories::create_parent(conn, CAT_FIT_PARENT, CategoryKind::Expense).unwrap();
    let child = financi_db::categories::create_child(conn, parent, CAT_FIT_CHILD).unwrap();
    let rule = financi_db::rules::add_rule(conn, CAT_FIT_PATTERN, false, child).unwrap();
    (child, rule)
}

/// The dashboard table's rows.
fn table_rows(shell: &Entity<Shell>, cx: &App) -> Vec<TransactionRow> {
    dash(cx, shell)
        .read(cx)
        .table
        .read(cx)
        .delegate()
        .rows
        .clone()
}

// ---------------------------------------------------------------------------
// Transfer pairing (task 4.2)

/// `Window::debug_bounds` takes a `&'static str`; dynamic selectors are
/// leaked (test processes are short-lived).
fn leaked(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Insert one row the way the importer's final insert does, returning its id.
fn seed_txn(
    conn: &financi_db::rusqlite::Connection,
    import: i64,
    account: i64,
    line: i64,
    date: &str,
    cents: i64,
    description: &str,
) -> i64 {
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents, description_raw)
         VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7)",
        (
            account,
            import,
            line,
            format!("hash-{account}-{line}"),
            date,
            cents,
            description,
        ),
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Import October onto a fresh Cobalt and return the busiest merchant with
/// a target row and every row id sharing that merchant.
fn import_and_top_merchant(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
) -> (String, i64, Vec<i64>) {
    wait_until(cx, "accounts", |cx| shell.read(cx).accounts.len() == 1);
    open_and_choose(
        cx,
        handle,
        shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(shell, cx));
    click_import(cx, handle, shell);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "import", |cx| dash(cx, shell).read(cx).row_count == 67);
    wait_until(cx, "queue populated", |cx| {
        // 3.83: the queue page defers applying while hidden; the snapshot
        // is what's always current.
        !shell.read(cx).catalog.read(cx).snap.queue.is_empty()
    });
    cx.update(|cx| {
        let (m, _) = top_group(shell, cx);
        let group: Vec<i64> = table_rows(shell, cx)
            .iter()
            .filter(|r| merchant(&r.description) == m)
            .map(|r| r.id)
            .collect();
        let target = *group.first().unwrap();
        (m, target, group)
    })
}

/// Read one raw `settings` row through the shell's own DB worker. Jobs run
/// on the worker thread in order, so a write already queued by the shell
/// has landed by the time this returns.
/// Block this thread until `future` completes. The `DbWorker` answers from
/// its own OS thread, so no test-executor pumping is needed.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::sync::mpsc;
    use std::task::{Context, Poll, Wake, Waker};
    struct ThreadWaker(mpsc::Sender<()>);
    impl Wake for ThreadWaker {
        fn wake(self: std::sync::Arc<Self>) {
            let _ = self.0.send(());
        }
    }
    let (tx, rx) = mpsc::channel();
    let waker = Waker::from(std::sync::Arc::new(ThreadWaker(tx)));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        rx.recv().expect("waker vanished");
    }
}

/// 5.3: hold the DB worker's writer thread so a test can act while a load
/// or a restore is in flight. The worker runs its jobs in order, so while
/// the blocker runs every later job waits and the shell's tasks park
/// mid-await. `wait_idle` first, so the blocker is the job that runs.
/// Dropping the returned guard releases the thread.
fn block_worker(cx: &mut TestAppContext, shell: &Entity<Shell>) -> WorkerBlock {
    use std::sync::mpsc;
    let db = shell.update(cx, |s, _| s.db.clone());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = block_on(db.call(move |_conn| {
            let _ = started_tx.send(());
            // Hold the writer thread until the test drops `release_tx`.
            let _ = release_rx.recv();
        }));
    });
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("worker blocker took the writer thread");
    WorkerBlock {
        _release: release_tx,
    }
}

/// Releases the worker-thread blocker (see [`block_worker`]) when dropped.
struct WorkerBlock {
    _release: std::sync::mpsc::Sender<()>,
}

fn db_setting(cx: &mut TestAppContext, shell: &Entity<Shell>, key: &str) -> Option<String> {
    let db = shell.update(cx, |s, _| s.db.clone());
    let key = key.to_string();
    block_on(db.call(move |c| {
        c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    }))
    .ok()
    .flatten()
}

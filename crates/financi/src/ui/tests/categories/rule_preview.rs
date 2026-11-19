//! The add-rule row's live preview: matching rows, tags, debounce,
//! clearing and invalid regex.

use super::*;

/// 3.103: fixture for the add form's live match preview — parent `Drinks`
/// with children `Coffee` and `Other`, an existing rule `TEA → Coffee`,
/// and transactions: nine `COFFEE n` (2026-01-01..09), a locked
/// `COFFEE LOCKED` (01-10), a `COFFEE TEA HOUSE` (01-11) the TEA rule
/// wins (ties go to the lower id), and a `BOOKS` row nothing matches.
/// Returns `(tea_rule, coffee_ids, locked_id, tie_id)`.
fn seed_preview_fixture(conn: &financi_db::rusqlite::Connection) -> (i64, Vec<i64>, i64, i64) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 12)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let coffee = financi_db::categories::create_child(conn, drinks, "Coffee").unwrap();
    let other = financi_db::categories::create_child(conn, drinks, "Other").unwrap();
    let tea = financi_db::rules::add_rule(conn, "TEA HOUSE", false, coffee).unwrap();
    let insert = |n: i64, desc: &str, date: String| -> i64 {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7)",
            (
                acct,
                import,
                n,
                format!("preview-hash-{n}"),
                date,
                -(n * 100),
                desc,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let coffees: Vec<i64> = (1..=9)
        .map(|n| insert(n, &format!("COFFEE {n}"), format!("2026-01-0{n}")))
        .collect();
    let locked = insert(10, "COFFEE LOCKED", "2026-01-10".into());
    let tie = insert(11, "COFFEE TEA HOUSE", "2026-01-11".into());
    insert(12, "BOOKS", "2026-01-12".into());
    conn.execute(
        "UPDATE transactions SET category_id = ?1, category_locked = 1 WHERE id = ?2",
        (other, locked),
    )
    .unwrap();
    (tea, coffees, locked, tie)
}
/// 3.103: open the shell over `seed_preview_fixture` on the Categories
/// page with the parent selected, so the add-rule form and its preview
/// are on screen.
fn open_preview_fixture(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Shell>, i64, Vec<i64>, i64, i64) {
    let mut ids = (0i64, Vec::new(), 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_preview_fixture(c));
    let (tea, coffees, locked, tie) = ids;
    wait_until(cx, "the preview fixture loaded", |cx| {
        // +1: the system `Transfers` parent from migration 0002.
        shell.read(cx).rules.len() == 1 && shell.read(cx).categories.len() == 2
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");
    (handle, shell, tea, coffees, locked, tie)
}
/// Type into the add form's Pattern input the way a keystroke does:
/// `set_value` plus the emitted `Change` the preview subscription rides
/// on — without moving the clock, so a test can watch the debounce run.
fn type_add_pattern(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    v: &str,
) {
    let v = v.to_string();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern.update(cx, |i, cx| {
                    i.set_value(v.clone(), window, cx);
                    cx.emit(InputEvent::Change);
                })
            });
        });
    });
}
/// Type, then run the fake clock past the 150 ms debounce (3.58's search
/// tests' pattern — the clock only moves when a test advances it).
fn set_add_pattern(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    v: &str,
) {
    type_add_pattern(cx, handle, shell, v);
    cx.executor().advance_clock(Duration::from_millis(160));
    cx.run_until_parked();
}
/// 3.103: typing a pattern shows the summary and up to [`PREVIEW_ROWS`]
/// preview rows — Decides first, newest first — while the rows past the
/// cap stay hidden and the summary still counts them.
#[gpui::test]
fn typing_a_pattern_previews_the_matching_rows(cx: &mut TestAppContext) {
    let (handle, shell, _tea, coffees, _locked, _tie) = open_preview_fixture(cx);
    set_add_pattern(cx, handle, &shell, "COFFEE");
    wait_until(cx, "the preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| p.total == 11 && p.decides == 9 && p.rows.len() == 8)
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:add-rule-preview"),
        "the form's second line carries the summary"
    );
    assert!(
        paints(cx, handle, "cat:preview-list"),
        "the preview list paints"
    );
    for id in &coffees[1..] {
        assert!(
            paints(cx, handle, format!("cat:preview-row:{id}")),
            "row {id} previews"
        );
    }
    assert!(
        !paints(cx, handle, format!("cat:preview-row:{}", coffees[0])),
        "the ninth match (the oldest) is past the 8-row cap"
    );
}
/// 3.103: rows the preview can't move get their muted tags — "locked"
/// for a hand-categorized row, "kept by `<pattern>`" for one an existing
/// rule wins.
#[gpui::test]
fn the_preview_tags_locked_and_kept_rows(cx: &mut TestAppContext) {
    let (handle, shell, tea, _coffees, locked, tie) = open_preview_fixture(cx);
    set_add_pattern(cx, handle, &shell, "COFFEE LOCKED");
    wait_until(cx, "the locked preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| {
                p.total == 1
                    && p.decides == 0
                    && p.rows.first().is_some_and(|r| r.transaction_id == locked)
            })
    });
    draw_frame(cx, handle);
    let status = cx.update(|cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| {
                p.rows
                    .first()
                    .is_some_and(|r| matches!(r.status, RuleMatchStatus::Locked))
            })
    });
    assert!(status, "the locked row reports Locked");
    assert!(
        paints(cx, handle, format!("cat:preview-row:{locked}"))
            && paints(cx, handle, format!("cat:preview-tag:{locked}:locked")),
        "the locked row previews with its \"locked\" tag"
    );
    set_add_pattern(cx, handle, &shell, "TEA HOUSE");
    wait_until(cx, "the kept-by preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| {
                p.total == 1
                    && p.decides == 0
                    && p.rows.first().is_some_and(|r| r.transaction_id == tie)
            })
    });
    draw_frame(cx, handle);
    let kept = cx.update(|cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| {
                p.rows.first().is_some_and(|r| {
                    r.transaction_id == tie
                        && matches!(&r.status, RuleMatchStatus::WonBy(w) if *w == tea)
                })
            })
    });
    assert!(kept, "the row reports WonBy the TEA HOUSE rule");
    assert!(
        paints(
            cx,
            handle,
            format!("cat:preview-tag:{tie}:kept by TEA HOUSE")
        ),
        "the row the TEA HOUSE rule wins previews with its \"kept by\" tag"
    );
}
/// H5 pin, the preview analogue of `rules_page_search_debounces_and_enter_applies_at_once`:
/// typing alone reads nothing — the read waits for the 150 ms debounce —
/// and a pattern typed inside the window supersedes the pending one, so
/// when the clock finally moves only the last pattern is ever read.
#[gpui::test]
fn the_preview_debounces_and_only_the_last_pattern_shows(cx: &mut TestAppContext) {
    let (handle, shell, _tea, _coffees, _locked, _tie) = open_preview_fixture(cx);
    type_add_pattern(cx, handle, &shell, "COFFEE");
    cx.run_until_parked();
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert!(
            shell.read(cx).rules_page.read(cx).pattern_preview.is_none(),
            "no read while the debounce runs"
        );
    });
    assert!(
        !paints(cx, handle, "cat:preview-list"),
        "nothing previews while the debounce runs"
    );

    // A second pattern inside the window replaces the pending timer:
    // when the clock moves, the read that runs is the last pattern's.
    type_add_pattern(cx, handle, &shell, "BOOKS");
    cx.executor().advance_clock(Duration::from_millis(160));
    cx.run_until_parked();
    wait_until(cx, "the BOOKS preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| {
                p.total == 1 && p.rows.first().is_some_and(|r| r.description == "BOOKS")
            })
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:preview-list"),
        "the last pattern's preview paints"
    );

    // And the superseded "COFFEE" timer never lands late: moving the
    // clock further keeps the BOOKS result on screen.
    cx.executor().advance_clock(Duration::from_millis(320));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .rules_page
                .read(cx)
                .pattern_preview
                .as_deref()
                .is_some_and(|p| p.total == 1),
            "the preview stays the BOOKS read, never the superseded COFFEE one"
        );
    });
}
/// 3.103: an empty pattern previews nothing at all — no summary, no list,
/// no rows — and a pattern that matches nothing says so.
#[gpui::test]
fn clearing_the_pattern_hides_the_preview(cx: &mut TestAppContext) {
    let (handle, shell, _tea, _coffees, _locked, _tie) = open_preview_fixture(cx);
    set_add_pattern(cx, handle, &shell, "COFFEE");
    wait_until(cx, "the preview", |cx| {
        shell.read(cx).rules_page.read(cx).pattern_preview.is_some()
    });
    draw_frame(cx, handle);
    assert!(paints(cx, handle, "cat:preview-list"));
    set_add_pattern(cx, handle, &shell, "");
    wait_until(cx, "the preview cleared", |cx| {
        shell.read(cx).rules_page.read(cx).pattern_preview.is_none()
    });
    draw_frame(cx, handle);
    for sel in ["cat:preview-list", "cat:add-rule-preview"] {
        assert!(
            !paints(cx, handle, sel),
            "the empty pattern shows nothing ({sel} still paints)"
        );
    }
    // A pattern that matches nothing reads "No transactions match".
    set_add_pattern(cx, handle, &shell, "ZZZZ");
    wait_until(cx, "the empty preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| p.rows.is_empty())
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:preview-empty:No transactions match"),
        "no match shows the \"No transactions match\" line"
    );
}
/// 3.103: with the regex switch on, a pattern that doesn't compile shows
/// the error text in place of the preview rows.
#[gpui::test]
fn an_invalid_regex_shows_the_error_instead_of_the_preview(cx: &mut TestAppContext) {
    let (handle, shell, _tea, _coffees, _locked, _tie) = open_preview_fixture(cx);
    click(cx, handle, "cat:switch:add");
    wait_until(cx, "the regex switch", |cx| {
        shell.read(cx).rules_page.read(cx).rule_regex
    });
    set_add_pattern(cx, handle, &shell, "(unclosed");
    wait_until(cx, "the preview error", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview_error
            .is_some()
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:preview-error"),
        "the error line paints where the preview would be"
    );
    for sel in [
        "cat:preview-list",
        "cat:preview-empty:No transactions match",
        "cat:add-rule-preview",
    ] {
        assert!(
            !paints(cx, handle, sel),
            "the error replaces the preview ({sel} still paints)"
        );
    }
    cx.update(|cx| {
        let s = shell.read(cx);
        let error = s.rules_page.read(cx).pattern_preview_error.clone().unwrap();
        assert!(
            error.contains("(unclosed"),
            "the error names the pattern: {error}"
        );
    });
}
/// 3.103: at 1200 × 720 the form and a full preview fit inside the detail
/// panel without pushing the rule list out of view entirely.
#[gpui::test]
fn the_form_and_a_full_preview_fit_at_1200x720(cx: &mut TestAppContext) {
    let (handle, shell, tea, coffees, _locked, _tie) = open_preview_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    set_add_pattern(cx, handle, &shell, "COFFEE");
    wait_until(cx, "the full preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| p.rows.len() == 8)
    });
    draw_frame(cx, handle);
    let form = bounds(cx, handle, "cat:add-rule-row");
    assert!(
        f32::from(form.bottom()) <= 720.,
        "the add-rule form (bottom {:?}) is inside the window",
        form.bottom()
    );
    // Every preview row paints, and the last one ends inside the window:
    // the full preview is visible, not scrolled away. Rows are newest
    // first, so the last is the oldest of the eight shown.
    let last = bounds(cx, handle, format!("cat:preview-row:{}", coffees[1]));
    assert!(
        f32::from(last.bottom()) <= 720.,
        "the preview's last row (bottom {:?}) is inside the window",
        last.bottom()
    );
    assert!(
        paints(cx, handle, format!("cat:rule-row:{tea}")),
        "the rule list is still in view under the preview"
    );
}
/// 3.53: a parent with no subcategories cannot hold a rule (rules target
/// a child), so a muted hint stands in for the add-rule row; a parent
/// with children keeps the row.
#[gpui::test]
fn a_childless_parent_offers_no_rule_row(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let drinks =
            financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, drinks, "Coffee").unwrap();
        financi_db::categories::create_parent(conn, "Books", CategoryKind::Expense).unwrap();
    });
    // +1: the system `Transfers` parent from migration 0002.
    wait_until(cx, "both parents loaded", |cx| {
        shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Books");
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("cat:add-rule-need-child").is_some(),
        "the childless parent shows the add-a-subcategory hint"
    );
    assert!(
        window.debug_bounds("cat:add-rule-row").is_none(),
        "and no add-rule row"
    );
    select_parent(cx, handle, &shell, "Drinks");
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:add-rule-need-child").is_none(),
        "the hint is gone once the selection has children"
    );
    assert!(
        window.debug_bounds("cat:add-rule-row").is_some(),
        "the parent with children keeps the add-rule row"
    );
}
/// 3.26: the preview region is sized to its [`PREVIEW_ROWS`] rows, so
/// even a pattern with more matches never overflows it — no thumb shows.
#[gpui::test]
fn the_preview_region_shows_no_scrollbar(cx: &mut TestAppContext) {
    let (handle, shell, _tea, _coffees, _locked, _tie) = open_preview_fixture(cx);
    set_add_pattern(cx, handle, &shell, "COFFEE");
    wait_until(cx, "the preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pattern_preview
            .as_deref()
            .is_some_and(|p| p.rows.len() == 8)
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:preview-list"),
        "the preview list paints"
    );
    let max = cx.update(|cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .preview_scroll
            .max_offset()
            .y
    });
    assert_eq!(
        f32::from(max),
        0.,
        "the 8-row region fits its rows by design"
    );
}

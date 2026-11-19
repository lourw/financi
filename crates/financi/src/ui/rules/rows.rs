//! The shared rule rows (S1 split from `ui/rules.rs`): one row of a
//! category's rule list, the in-place editor row, and the pattern-preview
//! block under both forms — free functions over the Rules page entity so the
//! Categories page paints the same state (3.80).

use super::*;

/// One row of a category's rule list (3.12: built lazily by the
/// virtualized rule list, fixed height to match its `item_sizes`). 3.24:
/// every row carries Edit and Delete at its right end — small ghost
/// buttons, visible on every row, built only for the rows the list
/// renders. 3.23: the columns line up with the list's headings
/// (`RULE_TYPE_W`/`RULE_USES_W`); the pattern column is the flex one and
/// truncates with a tooltip, like the queue's merchant column.
///
/// 3.49: the third column is the rule's Uses count — transactions this
/// rule categorizes, the same number the Rules page shows — right-aligned
/// through `flow::count`; 0 is muted, nonzero takes the row's own color.
/// `subcat` is the tag the Categories page passes when its selection is a
/// parent: the subcategory the rule targets, painted after the pattern
/// inside the Pattern column (the pattern ellipsizes first).
///
/// 3.80: a free function over the page entity — the Categories page paints
/// these rows from the Rules page's state (the module docs). The handlers
/// update the entity directly, so they work from either page.
pub(super) fn rule_row(
    rule: &Rule,
    muted: Hsla,
    uses: usize,
    subcat: Option<SharedString>,
    page: &RulesPage,
    entity: &Entity<RulesPage>,
) -> AnyElement {
    let id = rule.id;
    let armed = page.armed_delete_rule == Some(id);
    let delete_pending = page.rule_action_pending(PendingRuleAction::Delete(id));
    let edit_entity = entity.clone();
    let delete_entity = entity.clone();
    h_flex()
        .h_6()
        .w_full()
        .gap_2()
        .items_center()
        .debug_selector(move || format!("cat:rule-row:{id}"))
        .child(
            h_flex()
                .id(SharedString::from(format!("rule-pattern-{id}")))
                .flex_1()
                .min_w_0()
                .debug_selector(move || format!("cat:rule-pattern-col:{id}"))
                .child(
                    div()
                        .id(SharedString::from(format!("rule-pattern-text-{id}")))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .tooltip({
                            let full = rule.pattern.clone();
                            move |window, cx| {
                                let tip = full.clone();
                                Tooltip::element(move |_, _| {
                                    div()
                                        .debug_selector({
                                            let p = tip.clone();
                                            move || format!("cat:rule-pattern-tooltip:{p}")
                                        })
                                        .child(tip.clone())
                                })
                                .build(window, cx)
                            }
                        })
                        .child(rule.pattern.clone()),
                )
                .when_some(subcat, |col, name| {
                    let sel = format!("cat:rule-subcat:{id}:{name}");
                    col.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .debug_selector(move || sel.clone())
                            .child(name),
                    )
                }),
        )
        .child(
            div()
                .w(px(RULE_TYPE_W))
                .flex_shrink_0()
                .text_xs()
                .text_color(muted)
                .debug_selector(move || format!("cat:rule-type-col:{id}"))
                .child(if rule.is_regex { "regex" } else { "substring" }),
        )
        .child(
            div()
                .w(px(RULE_USES_W))
                .flex_shrink_0()
                .text_right()
                .text_xs()
                // 3.49: a rule nothing has categorized yet reads muted.
                .when(uses == 0, |cell| cell.text_color(muted))
                .debug_selector(move || format!("cat:rule-uses-col:{id}"))
                .child(
                    div()
                        .debug_selector(move || format!("cat:rule-uses:{id}:{uses}"))
                        .child(flow::count(uses as i64)),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("rule-edit-{id}")))
                .icon(IconName::Pencil)
                .label("Edit")
                .ghost()
                .text_xs()
                .flex_shrink_0()
                .debug_selector(move || format!("cat:rule-edit:{id}:{}", "icons/pencil.svg"))
                .on_click(move |_, window, cx| {
                    edit_entity.update(cx, |page, cx| page.start_edit_rule(id, window, cx));
                }),
        )
        .child(
            Button::new(SharedString::from(format!("rule-delete-{id}")))
                .icon(IconName::Trash)
                // 3.43: "Delete" with a trash icon, no "…"; armed it still
                // reads "Click again to delete".
                .label(if armed {
                    "Click again to delete"
                } else {
                    "Delete"
                })
                .ghost()
                .text_xs()
                .flex_shrink_0()
                .debug_selector(move || format!("cat:rule-delete:{id}:{}", "icons/trash.svg"))
                // 3.40: loading + disabled while the delete runs.
                .loading(delete_pending)
                .disabled(delete_pending)
                .on_click(move |_, window, cx| {
                    delete_entity.update(cx, |page, cx| page.arm_delete_rule(id, window, cx));
                }),
        )
        .into_any_element()
}

/// 3.103: the live match preview shared by the Categories add-rule form and
/// the rule editor (both pages) — a one-line summary over a heading-less
/// [`DataList`] of at most [`PREVIEW_ROWS`] rows (Date, Merchant, Amount),
/// each match tagged muted "kept by `<pattern>`" when an existing rule wins
/// it or "locked" when the row is locked, and nothing at all while the
/// pattern is empty or the first read is in flight. An empty preview reads
/// "No transactions match"; a bad regex shows the compiler's words in
/// place of the rows.
///
/// `prefix` namespaces the test selectors (`{prefix}:preview-list`,
/// `…:preview-row:{id}`, `…:preview-tag:{id}:{tag}`,
/// `…:preview-empty:{text}`, `…:preview-error`); `summary_selector` is
/// the summary line's selector with `{total}:{decides}` appended (`None`
/// on the Categories add form, whose summary sits in the form's
/// second line). H5:
/// the tag text, the empty line's text and the summary's counts ride in
/// the selectors — the 3.52 matches panel's pin (`rules:match-tag:won by
/// …`) — so tests assert the exact words, not just that a widget painted.
/// The rows come from [`RulesPage::pattern_preview`], already
/// excluded for the edited rule (2.5 re-run semantics), so this only paints
/// what the read returned.
pub(in crate::ui) fn pattern_preview_block(
    page: &RulesPage,
    host: &Entity<RulesPage>,
    prefix: &str,
    summary_selector: Option<&'static str>,
    cx: &App,
) -> AnyElement {
    let (fg, muted, danger) = {
        let theme = cx.theme();
        (theme.foreground, theme.muted_foreground, theme.danger)
    };
    let prefix: SharedString = prefix.into();
    let preview = page.pattern_preview.clone();
    if let Some(error) = page.pattern_preview_error.clone() {
        // One ellipsized line: like the editor's error, the virtualized
        // editor row can't grow past what it reserves. The selector is
        // hoisted (no literal at the call) — the 3.75 walk only checks
        // literals, and this one's namespace is interpolated.
        let error_sel = format!("{prefix}:preview-error");
        return div()
            .h(px(PREVIEW_LINE_H))
            .text_sm()
            .text_color(danger)
            .text_ellipsis()
            .debug_selector(move || error_sel.clone())
            .child(error)
            .into_any_element();
    }
    let Some(preview) = preview else {
        return div().into_any_element();
    };
    let summary = summary_selector.map(|sel| {
        // The counts ride in the selector (3.52's matches-summary pin),
        // so a test asserts the exact summary line, not just that one
        // painted.
        let sel = format!("{sel}:{}:{}", preview.total, preview.decides);
        div()
            .h(px(PREVIEW_LINE_H))
            .text_sm()
            .text_color(muted)
            .text_ellipsis()
            .debug_selector(move || sel.clone())
            .child(format!(
                "Matches {} transactions · {} would move here",
                flow::count(preview.total),
                flow::count(preview.decides)
            ))
    });
    let body = if preview.rows.is_empty() {
        let empty_sel = format!("{prefix}:preview-empty:{PREVIEW_EMPTY}");
        div()
            .h(px(PREVIEW_LINE_H))
            .text_sm()
            .text_color(muted)
            .debug_selector(move || empty_sel.clone())
            .child(PREVIEW_EMPTY)
            .into_any_element()
    } else {
        // The "kept by …" tag names the winning rule by its pattern, the
        // 3.52 matches panel's wording; the map is built once per render.
        let pattern_by_id: Rc<HashMap<i64, String>> = Rc::new(
            page.rules
                .iter()
                .map(|r| (r.id, r.pattern.clone()))
                .collect(),
        );
        let rows = preview.rows.clone();
        let list_h = rows.len() as f32 * PREVIEW_ROW_H;
        let items: Vec<DataItem> = rows
            .iter()
            .map(|row| DataItem::Row {
                id: row.transaction_id as usize,
                selected: false,
                selector: format!("{prefix}:preview-row:{}", row.transaction_id),
            })
            .collect();
        let list_selector = format!("{prefix}:preview-list");
        let list = DataList::new(host.clone(), preview_columns(), items)
            .hide_heading()
            .sizes(|_| PREVIEW_ROW_H)
            .scroll(page.preview_scroll.clone())
            .selector(list_selector)
            .cells(move |_this, item, _measuring, _window, _cx| {
                let DataItem::Row { id, .. } = item else {
                    return Vec::new();
                };
                let Some(row) = rows.iter().find(|r| r.transaction_id == *id as i64) else {
                    return Vec::new();
                };
                // Merchant first, like the transactions list: the raw
                // description behind it stays in the tooltip.
                let merchant = financi_core::parse::merchant(&row.description);
                let merchant_cell = div()
                    .id(SharedString::from(format!(
                        "preview-merchant-{}",
                        row.transaction_id
                    )))
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(fg)
                    .tooltip({
                        let text = row.description.clone();
                        move |window, cx| {
                            let text = text.clone();
                            Tooltip::element(move |_, _| {
                                let text = text.clone();
                                div().child(text)
                            })
                            .build(window, cx)
                        }
                    })
                    .child(merchant)
                    .into_any_element();
                let tag = match &row.status {
                    RuleMatchStatus::WonBy(winner) => Some(format!(
                        "kept by {}",
                        pattern_by_id
                            .get(winner)
                            .cloned()
                            .unwrap_or_else(|| format!("#{winner}"))
                    )),
                    RuleMatchStatus::Locked => Some("locked".into()),
                    RuleMatchStatus::Decides => None,
                };
                let amount = h_flex()
                    .justify_end()
                    .gap_1()
                    .text_color(fg)
                    .child(flow::money(row.amount_cents))
                    .when_some(tag, |d, t| {
                        // The selector is built outside the
                        // `debug_selector` call — the 3.75 walk only
                        // checks literals inside it, and this one's
                        // namespace is interpolated. The tag text rides
                        // in it too (3.52's matches-tag pin): tests pin
                        // "locked" / "kept by <pattern>" exactly.
                        let sel = format!("{prefix}:preview-tag:{}:{t}", row.transaction_id);
                        d.child(
                            div()
                                .id(SharedString::from(format!(
                                    "preview-tag-{}",
                                    row.transaction_id
                                )))
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(muted)
                                .debug_selector(move || sel.clone())
                                .child(t),
                        )
                    });
                vec![
                    ListCell::from(
                        div()
                            .text_color(fg)
                            .child(row.posted_date.clone())
                            .into_any_element(),
                    ),
                    ListCell::from(merchant_cell),
                    ListCell::from(amount.into_any_element()),
                ]
            });
        // The list keeps its own scroll: tight space shrinks the preview to
        // one visible row instead of pushing the rule list out of view.
        div()
            .w_full()
            .h(px(list_h))
            .flex_shrink_1()
            .min_h(px(PREVIEW_ROW_H))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(list)
            .into_any_element()
    };
    v_flex()
        .w_full()
        .children(summary)
        .child(body)
        .into_any_element()
}

/// The preview list's columns: Date, the one flex Merchant, and a
/// right-aligned Amount — the matches panel's shape without the Account
/// column (3.103).
fn preview_columns() -> Vec<DataCol> {
    vec![
        DataCol::new("Date", Col::Fixed(110.), ColAlign::Left),
        DataCol::new("Merchant", Col::Flex, ColAlign::Left),
        DataCol::new("Amount", Col::Fixed(110.), ColAlign::Right),
    ]
}

/// 3.24: the in-place editor that replaces one rule's row, pre-filled
/// from the rule, with the same live match preview. A
/// validation error shows in `danger` under the editor and keeps it open —
/// clamped to one ellipsized line, because the virtual list doesn't clip
/// per item and a taller row would paint over the rows below.
///
/// 3.48: the editor is 3.89's [`inline_form`] — a bordered form opened in
/// place of the row, two lines tall so the Pattern input gets the editor's
/// full width (the old single row shrank it past editing in the detail
/// panel). Line 1 is the "Pattern" label over the input; line 2 is one
/// `items_center` row of the 32 px controls: the regex switch, then Save
/// and Cancel pushed right with `ml_auto`. Line 2 carries no labels, so
/// the switch lines up on its own — no 3.42 `control_slot` here (the
/// Rules page add row keeps its own; 3.103 gave the Categories one this
/// same two-line shape). The preview block and error line stay
/// under both lines; the row reserves [`RULE_EDITOR_H`]
/// ([`RULE_EDITOR_ERROR_H`] with an error open) plus the preview's own
/// height.
///
/// 3.53: no Category picker — the editor changes pattern and kind only,
/// and the rule keeps its category (moving is the Rules page Move bar's
/// job). 3.80: a free function over the page entity, like [`rule_row`].
///
/// 3.103: under the form sits [`pattern_preview_block`] — the live match
/// preview of the editor's draft (that rule excluded), the same rows the
/// Categories add-rule form shows. The row reserves [`RULE_EDITOR_H`] plus
/// [`RulesPage::editor_preview_extra_h`].
pub(super) fn rule_editor_row(
    rule: &Rule,
    muted: Hsla,
    danger: Hsla,
    page: &RulesPage,
    entity: &Entity<RulesPage>,
    cx: &App,
) -> AnyElement {
    let preview = pattern_preview_block(page, entity, "rules", Some("rules:edit-preview"), cx);
    let switch_entity = entity.clone();
    let save_entity = entity.clone();
    let cancel_entity = entity.clone();
    v_flex()
        .w_full()
        .gap_1()
        .debug_selector(|| "rules:edit-form".into())
        .child(
            inline_form(cx).child(
                v_flex()
                    .w_full()
                    .gap_2()
                    // Line 1: the "Pattern" label over the input, at the
                    // editor's full width (the user-reported bug: the
                    // one-row editor squeezed the input to nothing).
                    .child(
                        v_flex().gap_1().child(field_label(muted, "Pattern")).child(
                            div()
                                .w_full()
                                .debug_selector(|| "rules:edit-pattern-input".into())
                                .child(Input::new(&page.rule_edit_pattern)),
                        ),
                    )
                    // Line 2: the controls, no labels — the switch at the
                    // left, Save and Cancel at the right end (`ml_auto`
                    // on the first of them). `items_center` lines the
                    // switch up with the buttons without a slot (3.48).
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .debug_selector(|| "rules:edit-controls-row".into())
                            .child(
                                div()
                                    .flex_shrink_0()
                                    // 3.42's probe selector: the
                                    // alignment test measures the switch
                                    // itself, not the row around it.
                                    .debug_selector(|| "rules:switch:edit".into())
                                    .child(
                                        Switch::new(SharedString::from(format!(
                                            "rule-edit-regex-{}",
                                            rule.id
                                        )))
                                        .label("regex")
                                        .checked(page.rule_edit_regex)
                                        .on_click(
                                            move |checked: &bool, _, cx| {
                                                switch_entity.update(cx, |page, cx| {
                                                    page.rule_edit_regex = *checked;
                                                    page.run_pattern_preview(cx);
                                                    cx.notify();
                                                });
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "rule-edit-save-{}",
                                    rule.id
                                )))
                                .label("Save")
                                .primary()
                                .ml_auto()
                                .flex_shrink_0()
                                // 3.42: the alignment test measures its centre.
                                .debug_selector(|| "rules:edit-save-btn".into())
                                // 3.40: loading + disabled while the save runs.
                                .loading(page.rule_action_pending(PendingRuleAction::Save(rule.id)))
                                .disabled(
                                    page.rule_action_pending(PendingRuleAction::Save(rule.id)),
                                )
                                .on_click(
                                    move |_, window, cx| {
                                        save_entity
                                            .update(cx, |page, cx| page.save_rule_edit(window, cx));
                                    },
                                ),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "rule-edit-cancel-{}",
                                    rule.id
                                )))
                                .label("Cancel")
                                .ghost()
                                .flex_shrink_0()
                                .debug_selector(|| "rules:edit-cancel-btn".into())
                                .on_click(move |_, _, cx| {
                                    cancel_entity.update(cx, |page, cx| page.cancel_edit_rule(cx));
                                }),
                            ),
                    ),
            ),
        )
        // 3.23: the count sits under the fields (as in the add-rule
        // row): the detail panel is too narrow to keep it inline.
        .child(preview)
        .children(
            page.rule_error
                .clone()
                // One line, ellipsized (newlines flattened): the slot
                // reserves room for exactly one error line, and the
                // regex crate's messages are multi-line.
                .map(|e| e.replace('\n', " "))
                .map(|e| {
                    div()
                        .text_sm()
                        .text_color(danger)
                        .text_ellipsis()
                        .debug_selector(|| "rules:edit-error".into())
                        .child(e)
                }),
        )
        .into_any_element()
}

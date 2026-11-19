//! `Render for RulesPage` (S1 split from `ui/rules.rs`): the Rules page
//! view — header, add-rule row, filters and the shared `DataList` panel.

use super::*;

impl Render for RulesPage {
    /// The Rules page (3.28, D5; 3.58 restyle): header with the "N rules ·
    /// M unused" summary and New rule, the add-rule row under it, the
    /// filter row (Search, Type, the Options popover), and one bordered
    /// panel holding the shared `DataList` — 40 px rows, 36 px group
    /// headers with summed Uses and Total, sortable from the heading, the
    /// Move bar taking the heading slot while rows are checked.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 3.28: opening the Rules page focuses its search box (D5). The
        // focus waits for the end of the frame so the input's element
        // exists first (3.80: raised by `set_visible`, like the other
        // pages' root focus).
        if self.focus_pending {
            self.focus_pending = false;
            let handle = self.rules_search.focus_handle(cx);
            window.defer(cx, move |window, cx| {
                window.focus(&handle, cx);
            });
        }
        // 3.54: a new shared category list lands through
        // `refresh_cat_pickers`, from `render` and from the actions that
        // read those pickers — a hidden page defers the re-set until it
        // next renders or acts (the queue page's lazy-picker pattern).
        self.refresh_cat_pickers(window, cx);
        // 3.59: the menu width both category pickers share: the longest
        // "Category › Subcategory" label up to 320 px.
        let cat_menu_w = sub_choices_menu_width(&self.catalog.read(cx).cat_choices());
        let (muted, fg, danger, border, success, primary) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.foreground,
                theme.danger,
                theme.border,
                theme.success,
                theme.primary,
            )
        };
        let mono = cx.theme().mono_font_family.clone();
        // 3.58: the header summary (ui-conventions *Rules*).
        let summary = format!(
            "{} rules · {} unused",
            flow::count(self.rules.len() as i64),
            flow::count(
                self.rules
                    .iter()
                    .filter(|r| self.rule_uses.get(&r.id).copied().unwrap_or(0) == 0)
                    .count() as i64
            )
        );
        // The "wins" marker follows [`winning_rule`] exactly, so the page
        // marks the rule the categoriser would actually apply (D5). 3.58:
        // the list filters on the debounced query, so the marker moves
        // with the list, not mid-keystroke.
        let winner = if self.rules_test_mode && !self.rules_query.is_empty() {
            self.catalog
                .read(cx)
                .compiled_rules()
                .winning(&self.rules_query)
        } else {
            None
        };
        // 3.85: the grouped list comes from the page's caches; a frame
        // that changes no input rebuilds nothing. 3.58: the group sums
        // ride along with the same cache build.
        let items = self.rules_items_mut(cx);
        let sums = Rc::clone(&self.rules_group_sums);
        let editing = self.editing_rule;
        let edit_error = self.rule_error.is_some();
        // 3.103: the editor row grows by its preview's height.
        let preview_extra = self.editor_preview_extra_h();
        // One [`DataItem`] per line for the shared list (3.89): a row's
        // `id` carries the rule id (always positive), and only the row
        // *Show rule* (3.29) landed on is `selected` (`list_active`).
        let data_items: Vec<DataItem> = items
            .iter()
            .map(|it| match it {
                RulesItem::Group {
                    id,
                    label,
                    count,
                    collapsed,
                } => DataItem::Group {
                    id: *id,
                    collapsed: *collapsed,
                    name: label.clone().into(),
                    count: Some(format!("{count} rules").into()),
                    selector: format!("rules:group:{id}"),
                },
                RulesItem::Rule { id } => {
                    // 3.52: the focused rule takes the same `list_active`
                    // background as the *Show rule* reveal; its selector
                    // says which is which.
                    let reveal = self.rules_reveal == Some(*id);
                    let focused = self.focused_rule == Some(*id);
                    DataItem::Row {
                        id: *id as usize,
                        selected: reveal || focused,
                        selector: if reveal {
                            format!("rules:row-{id}:active")
                        } else if focused {
                            format!("rules:row-{id}:focused")
                        } else {
                            format!("rules:row-{id}")
                        },
                    }
                }
            })
            .collect();
        // 3.58: the list's columns (ui-conventions *Rules*): checkbox,
        // Pattern (the one flex column), Type, Uses and Total right —
        // the heading shares these widths, so every heading sits over its
        // column and the old spacer hack is gone.
        let columns = vec![
            DataCol::new("", Col::Fixed(RULE_SEL_W), ColAlign::Left),
            DataCol::new("Pattern", Col::Flex, ColAlign::Left).sortable(),
            DataCol::new("Type", Col::Fixed(RULE_TYPE_W), ColAlign::Left),
            DataCol::new("Uses", Col::Fixed(RULE_USES_W), ColAlign::Right).sortable(),
            DataCol::new("Total", Col::Fixed(RULE_TOTAL_W), ColAlign::Right).sortable(),
        ];
        let list = DataList::new(cx.entity(), columns, data_items)
            .on_pass(move |this, range| {
                // The long-list tests pin this frame's exact pass window.
                #[cfg(test)]
                {
                    this.rules_page_rows_built.set(range.len());
                    this.rules_page_range.set((range.start, range.end));
                }
                #[cfg(not(test))]
                {
                    let _ = (&this, range);
                }
            })
            // 3.48: the row the editor replaced reserves the editor slot.
            .sizes(move |item| match item {
                DataItem::Row { id, .. } => {
                    if editing == Some(*id as i64) {
                        (if edit_error {
                            RULE_EDITOR_ERROR_H
                        } else {
                            RULE_EDITOR_H
                        }) + preview_extra
                    } else {
                        LIST_ROW_HEIGHT
                    }
                }
                DataItem::Group { .. } => LIST_GROUP_HEIGHT,
            })
            .cells(move |this, item, _measuring, _window, cx| match item {
                DataItem::Row { id, .. } => {
                    let id = *id as i64;
                    let Some(rule) = this.rules.iter().find(|r| r.id == id) else {
                        return Vec::new();
                    };
                    if this.editing_rule == Some(id) {
                        // 3.24: the shared in-place editor replaces the
                        // row — one full-width cell, no actions column.
                        return vec![
                            rule_editor_row(rule, muted, danger, this, &cx.entity(), cx).into(),
                        ];
                    }
                    let selected = this.selected_rules.contains(&id);
                    let uses = this.rule_uses.get(&id).copied().unwrap_or(0);
                    let total = this.rule_totals.get(&id).copied().unwrap_or(0);
                    let pattern = rule.pattern.clone();
                    vec![
                        div()
                            .id(SharedString::from(format!("rules-selwrap-{id}")))
                            // 3.52: the checkbox takes its click — it must
                            // not also toggle the row's matches focus.
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(
                                checkbox(SharedString::from(format!("rule-sel-{id}")))
                                    .checked(selected)
                                    .on_click(cx.listener(move |this, on: &bool, _, cx| {
                                        this.set_rule_selected(id, *on, cx);
                                    })),
                            )
                            .into_any_element()
                            .into(),
                        // 3.58: the pattern is `mono` (regexes read
                        // better), ellipsized with a tooltip holding the
                        // full text; the "wins" chip (3.28) rides along.
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .id(SharedString::from(format!("rules-pattern-{id}")))
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .font_family(mono.clone())
                                    .debug_selector(move || format!("rules:pattern-col:{id}"))
                                    .tooltip({
                                        let full = pattern.clone();
                                        move |window, cx| {
                                            let tip = full.clone();
                                            Tooltip::element(move |_, _| {
                                                div()
                                                    .debug_selector({
                                                        let p = tip.clone();
                                                        move || format!("rules:pattern-tooltip:{p}")
                                                    })
                                                    .child(tip.clone())
                                            })
                                            .build(window, cx)
                                        }
                                    })
                                    .child(rule.pattern.clone()),
                            )
                            .when(winner == Some(id), |row| {
                                row.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(primary)
                                        .debug_selector(move || format!("rules:wins-{id}"))
                                        .child("wins"),
                                )
                            })
                            .into_any_element()
                            .into(),
                        div()
                            .debug_selector(move || format!("rules:type-col:{id}"))
                            .child(type_tag(if rule.is_regex { "regex" } else { "text" }))
                            .into_any_element()
                            .into(),
                        div()
                            .text_color(if uses == 0 { muted } else { fg })
                            .debug_selector(move || format!("rules:uses-col:{id}"))
                            .child(flow::count(uses as i64))
                            .into_any_element()
                            .into(),
                        div()
                            // money in is `success`; money out stays plain.
                            .text_color(if total > 0 { success } else { fg })
                            .debug_selector(move || format!("rules:total-col:{id}"))
                            .child(
                                div()
                                    .debug_selector(move || format!("rules:total:{id}:{total}"))
                                    .child(flow::money(total)),
                            )
                            .into_any_element()
                            .into(),
                    ]
                }
                DataItem::Group { id, .. } => {
                    // 3.58: the group's trailing cells land under Uses
                    // and Total — the group's summed counts (3.87 stats,
                    // summed once per list build).
                    let (uses, total) = sums.get(id).copied().unwrap_or((0, 0));
                    vec![
                        div()
                            .text_color(muted)
                            .debug_selector(move || format!("rules:group-uses:{id}"))
                            .child(
                                div()
                                    .debug_selector(move || format!("rules:group-uses:{id}:{uses}"))
                                    .child(flow::count(uses as i64)),
                            )
                            .into_any_element()
                            .into(),
                        div()
                            .text_color(if total > 0 { success } else { muted })
                            .debug_selector(move || format!("rules:group-total:{id}"))
                            .child(
                                div()
                                    .debug_selector(move || {
                                        format!("rules:group-total:{id}:{total}")
                                    })
                                    .child(flow::money(total)),
                            )
                            .into_any_element()
                            .into(),
                    ]
                }
            })
            // 3.58: Edit / Delete are `row_action_button`s — icon-only
            // ghost buttons with tooltips — revealed on row hover only;
            // the column keeps its width so nothing shifts.
            .actions(RULE_ACTIONS_W, move |this, item, _window, cx| {
                let DataItem::Row { id, .. } = item else {
                    return Vec::new();
                };
                let id = *id as i64;
                if this.editing_rule == Some(id) {
                    return Vec::new();
                }
                let armed = this.armed_delete_rule == Some(id);
                let pending = this.rule_action_pending(PendingRuleAction::Delete(id));
                // 3.52: each button swallows its click so Edit / Delete
                // never reach the row's focus handler.
                vec![
                    div()
                        .id(SharedString::from(format!("rules-editwrap-{id}")))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            row_action_button(
                                SharedString::from(format!("rule-edit-{id}")),
                                IconName::Pencil,
                                "Edit rule",
                            )
                            .debug_selector(move || {
                                format!("rules:row-edit:{id}:{}", "icons/pencil.svg")
                            })
                            .on_click(cx.listener(
                                move |this, _, window, cx| this.start_edit_rule(id, window, cx),
                            )),
                        )
                        .into_any_element(),
                    div()
                        .id(SharedString::from(format!("rules-delwrap-{id}")))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            row_action_button(
                                SharedString::from(format!("rule-delete-{id}")),
                                IconName::Trash,
                                if armed {
                                    "Click again to delete"
                                } else {
                                    "Delete rule"
                                },
                            )
                            .debug_selector(move || {
                                format!("rules:row-delete:{id}:{}", "icons/trash.svg")
                            })
                            // 3.40: loading + disabled while the delete runs.
                            .loading(pending)
                            .disabled(pending)
                            .on_click(cx.listener(
                                move |this, _, window, cx| this.arm_delete_rule(id, window, cx),
                            )),
                        )
                        .into_any_element(),
                ]
            })
            .toggle(move |this, id, _window, cx| this.toggle_rule_group(id, cx))
            // 3.52: a row click focuses the rule and opens its matches
            // panel under the list; the same row again closes it.
            .row_click(move |this, ix, _window, cx| this.toggle_rule_focus(ix as i64, cx))
            .sort(move |this, title, next, _window, cx| {
                // 3.58: a heading click picks the column and direction.
                // A repeat click flips (the `next` the list computed); a
                // fresh click on a count or amount column starts
                // biggest-first, like the transactions table's Date and
                // Amount (ui-conventions *Lists*).
                let state = match this.rules_sort {
                    Some((col, _)) if col == title => next,
                    _ if matches!(title, "Uses" | "Total") => SortState::Descending,
                    _ => next,
                };
                this.rules_sort = Some((title, state));
                cx.notify();
            })
            .heading_selectors("rules")
            .scroll(self.rules_scroll.clone())
            .selector("rules:list");
        let list = match self.rules_sort {
            Some((col, state)) => list.sorted_by(col, state),
            None => list,
        };
        // 3.58: with rows checked the Move bar replaces the heading row
        // inside the panel (same 32 px), so the list never moves down.
        let moving = !self.selected_rules.is_empty();
        let list = if moving { list.hide_heading() } else { list };
        let n_sel = self.selected_rules.len();
        let move_bar = h_flex()
            .w_full()
            .h(px(RULE_MOVE_BAR_H))
            .px_3()
            .gap_2()
            .items_center()
            .flex_shrink_0()
            .border_b_1()
            .border_color(border)
            .debug_selector(|| "rules:move-bar".into())
            .child(
                div()
                    .text_sm()
                    .flex_shrink_0()
                    .debug_selector(|| "rules:selected-count".into())
                    .child(format!("{n_sel} selected")),
            )
            .child(
                div()
                    .w(px(240.))
                    .flex_shrink_0()
                    .debug_selector(|| "rules:move-target".into())
                    .child(
                        Select::new(&self.move_rules_target)
                            .menu_width(cat_menu_w)
                            .placeholder(MOVE_RULES_TO_PH),
                    ),
            )
            .child(
                Button::new("rules-move")
                    .label("Move")
                    .primary()
                    .flex_shrink_0()
                    .debug_selector(|| "rules:move-btn".into())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.move_selected_rules(window, cx)),
                    ),
            )
            .child(
                Button::new("rules-clear")
                    .label("Clear")
                    .ghost()
                    .flex_shrink_0()
                    .debug_selector(|| "rules:clear-btn".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected_rules.clear();
                        cx.notify();
                    })),
            );
        // 3.52: with a rule focused the panel splits — the list on top
        // (~55%), the matches panel below it (~45%), divided by a border.
        // Both areas keep a minimum height and scroll independently.
        let matches_el = self.focused_rule.is_some().then(|| self.matches_panel(cx));
        let panel = panel(cx, false)
            .debug_selector(|| "rules:panel".into())
            .when(moving, |page| page.child(move_bar))
            .children(self.rules_move_error.clone().map(|e| {
                div()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .text_color(danger)
                    .debug_selector(|| "rules:move-error".into())
                    .child(e)
            }))
            .when(items.is_empty(), |page| {
                page.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(muted)
                        .debug_selector(|| "rules:empty".into())
                        .child(if self.rules_test_mode {
                            "No rules match this description"
                        } else {
                            "No rules match"
                        }),
                )
            })
            .when(!items.is_empty(), move |page| match matches_el {
                None => page.child(list),
                Some(el) => page
                    .child(
                        v_flex()
                            .flex_grow(55.)
                            .flex_basis(px(0.))
                            .min_h(px(120.))
                            .overflow_hidden()
                            .debug_selector(|| "rules:list-area".into())
                            .child(list),
                    )
                    .child(
                        v_flex()
                            .flex_grow(45.)
                            .flex_basis(px(0.))
                            .min_h(px(160.))
                            .border_t_1()
                            .border_color(border)
                            .overflow_hidden()
                            .debug_selector(|| "rules:matches-area".into())
                            .child(el),
                    ),
            });
        let add_preview = match self
            .pattern_preview
            .as_deref()
            .filter(|_| self.editing_rule.is_none())
        {
            Some(preview) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "rules:add-preview".into())
                .child(format!(
                    "matches {} transactions",
                    flow::count(preview.total)
                ))
                .into_any_element(),
            None => div().into_any_element(),
        };
        // 3.58: New rule is primary, but outline while its add form is
        // open (one primary per page).
        let new_rule = Button::new("new-rule")
            .icon(IconName::Plus)
            .label(NEW_RULE)
            .debug_selector(move || format!("label:{NEW_RULE}"))
            .flex_shrink_0()
            .on_click(cx.listener(|this, _, _, cx| {
                // Only one rule editor is open at a time: opening the
                // add row closes any row editor.
                this.close_rule_editor();
                this.new_rule_open = !this.new_rule_open;
                cx.notify();
            }));
        let new_rule = if self.new_rule_open {
            new_rule.outline()
        } else {
            new_rule.primary()
        };
        // 3.58: the Options popover holds the two toggles that used to
        // sit in the filter row; its trigger counts how many are on
        // ("Options · N", like the dashboard's 3.35).
        let on = (self.rules_test_mode as usize) + (self.rules_unused_only as usize);
        let options_label: SharedString = if on > 0 {
            format!("Options · {on}").into()
        } else {
            "Options".into()
        };
        let entity = cx.entity();
        let (test_on, unused_on) = (self.rules_test_mode, self.rules_unused_only);
        let options = Popover::new("rules-options")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("rules-options-trigger")
                    .icon(IconName::ListFilter)
                    .label(options_label.clone())
                    .debug_selector(move || format!("rules:options-btn:{options_label}")),
            )
            .content(move |_, _, _| {
                let (s1, s2) = (entity.clone(), entity.clone());
                v_flex()
                    .gap_2()
                    .child(
                        Switch::new("rules-test-mode")
                            .label("Test a description")
                            .checked(test_on)
                            .on_click(move |on: &bool, _, cx| {
                                let on = *on;
                                s1.update(cx, |this, cx| {
                                    this.rules_test_mode = on;
                                    this.rules_scroll.set_offset(Point::default());
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        checkbox("rules-unused-only")
                            .label("Unused only")
                            .checked(unused_on)
                            .on_click(move |on: &bool, _, cx| {
                                let on = *on;
                                s2.update(cx, |this, cx| {
                                    this.rules_unused_only = on;
                                    this.rules_scroll.set_offset(Point::default());
                                    cx.notify();
                                });
                            }),
                    )
            });

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            // 3.80: the page's own key context, so Escape is *this* page's
            // Cancel (`ui::init` binds it per context); the root takes
            // focus when the page is shown.
            .track_focus(&self.focus)
            .key_context("Rules")
            .on_action(cx.listener(|this, _: &Cancel, _, cx| {
                // Escape unwinds one level per press (3.80,
                // *Interaction*): the open editor first, then the armed
                // delete, then the ticked rows; with nothing left it
                // propagates to the window-level bindings.
                if this.close_rule_editor() {
                    cx.notify();
                    return;
                }
                if this.armed_delete_rule.take().is_some() {
                    cx.notify();
                    return;
                }
                // 3.52: then the focused rule's matches panel.
                if this.focused_rule.is_some() {
                    this.clear_rule_focus();
                    cx.notify();
                    return;
                }
                if !this.selected_rules.is_empty() {
                    this.selected_rules.clear();
                    cx.notify();
                    return;
                }
                cx.propagate();
            }))
            // 3.58: `page_header` builds the header row.
            .child(
                page_header(
                    cx,
                    "Rules",
                    summary.clone(),
                    vec![new_rule.into_any_element()],
                )
                .debug_selector(move || format!("rules:summary:{summary}")),
            )
            // Add-rule row (3.28): the same row the Categories page has,
            // hidden until the header button opens it.
            .when(self.new_rule_open, |page| {
                page.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .debug_selector(|| "rules:add-row".into())
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(field_label(muted, "Pattern"))
                                        .child(
                                            div()
                                                .w_full()
                                                .debug_selector(|| "rules:pattern-input:add".into())
                                                .child(Input::new(&self.rule_pattern)),
                                        ),
                                )
                                .child(
                                    // 3.42: control-height slot, centred.
                                    control_slot(
                                        "rules:regex-slot:add".into(),
                                        "rules:switch:add".into(),
                                        Switch::new("rule-regex")
                                            .label("regex")
                                            .checked(self.rule_regex)
                                            .on_click(cx.listener(
                                                |this, checked: &bool, _, cx| {
                                                    this.rule_regex = *checked;
                                                    this.run_pattern_preview(cx);
                                                    cx.notify();
                                                },
                                            )),
                                    ),
                                )
                                .child(
                                    v_flex()
                                        .flex_shrink_0()
                                        .gap_1()
                                        .child(field_label(muted, "Category"))
                                        .child(
                                            div().w(px(160.)).child(
                                                Select::new(&self.rule_target)
                                                    .menu_width(cat_menu_w)
                                                    .placeholder("Category…"),
                                            ),
                                        ),
                                )
                                .child(
                                    Button::new("add-rule")
                                        .label("Add rule")
                                        .primary()
                                        .flex_shrink_0()
                                        // 3.42: the alignment test measures its centre.
                                        .debug_selector(|| "rules:add-rule-btn:add".into())
                                        // 3.40: loading + disabled while the add runs.
                                        .loading(self.rule_action_pending(PendingRuleAction::Add))
                                        .disabled(self.rule_action_pending(PendingRuleAction::Add))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.add_rule(window, cx)
                                        })),
                                ),
                        )
                        .child(add_preview)
                        .children(
                            self.rule_error
                                .clone()
                                .filter(|_| self.editing_rule.is_none())
                                .map(|e| div().text_sm().text_color(danger).child(e)),
                        ),
                )
            })
            // 3.58: `filter_row` (ui-conventions *Filtering*): Search
            // flexes with a leading icon; the Type picker and the Options
            // popover take the rest.
            .child(
                filter_row()
                    .debug_selector(|| "rules:filter-row".into())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .debug_selector(|| "rules:search".into())
                            .child(
                                Input::new(&self.rules_search)
                                    .prefix(Icon::new(IconName::Search).small().text_color(muted)),
                            ),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_shrink_0()
                            .debug_selector(|| "rules:type-select".into())
                            .child(Select::new(&self.rules_type_select).placeholder("All types")),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .debug_selector(|| "rules:options".into())
                            .child(options),
                    ),
            )
            // 3.58: one bordered panel holds the list (with the Move bar
            // in the heading slot while rows are checked).
            .child(panel)
            .into_any_element()
    }
}

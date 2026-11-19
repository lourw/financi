//! The Rules page list (3.28, S1 split from `ui/rules.rs`): the grouped,
//! filtered and sorted item list, the checked rows and the Move bar, and the
//! focused rule's matches panel (3.52).

use super::*;

impl RulesPage {
    // -- Rules page (3.28) ----------------------------------------------------

    /// "Parent › Child" label of the rule's target category, shared by the
    /// row, the group headers and the search matching. 3.85: the labels
    /// are the Catalog's map, built once per rules or categories reload
    /// (the `#{id}` fallback for a missing child baked in); `labels` is
    /// that map, fetched once per list build.
    fn rule_target_label(labels: &HashMap<i64, SharedString>, rule: &Rule) -> String {
        labels
            .get(&rule.id)
            .map(|l| l.to_string())
            .unwrap_or_else(|| format!("#{}", rule.category_id))
    }

    /// 3.58: apply the search text the debounce timer (or Enter) just
    /// settled on. Only a real change rebuilds the list and rewinds it.
    pub(super) fn apply_rules_query(&mut self, value: &str, cx: &mut Context<Self>) {
        let query = value.trim().to_ascii_lowercase();
        if query == self.rules_query {
            return;
        }
        self.rules_query = query;
        self.rules_scroll.set_offset(Point::default());
        cx.notify();
    }

    /// The Rules page list's entries after the page filters (D5). Normal
    /// mode groups the rules by target subcategory (groups in label order,
    /// rows within a group sorted by the heading sort — Pattern by
    /// default, Uses or Total when one is picked — 3.58), with a group
    /// header row per group. "Test a description" mode shows a flat list
    /// of the rules matching the text, in the order [`winning_rule`] tries
    /// them (substring before regex, longest pattern first, lowest id), so
    /// the winner is the first row. The second return value is each
    /// group's summed Uses and Total (3.58), keyed by group id.
    fn rules_page_items(&self, cx: &App) -> (Vec<RulesItem>, HashMap<i64, (usize, i64)>) {
        let query = self.rules_query.clone();
        let query = (!query.is_empty()).then_some(query);
        // Type picker: row 1 keeps substrings, row 2 regexes, nothing
        // selected or row 0 shows all.
        let type_ix = self
            .rules_type_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        let test = self.rules_test_mode;
        // 3.85: the Catalog's derived maps, fetched once per list build.
        let rule_labels = self.catalog.read(cx).rule_labels();
        let rule_patterns = self.catalog.read(cx).rule_patterns();
        let mut filtered: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| match type_ix {
                1 => !r.is_regex,
                2 => r.is_regex,
                _ => true,
            })
            .filter(|r| {
                !self.rules_unused_only || self.rule_uses.get(&r.id).copied().unwrap_or(0) == 0
            })
            .filter(|r| {
                if test {
                    // The search box is the description: only rules that
                    // match it stay (3.85: the Catalog's compiled
                    // patterns, `pattern_matches` semantics unchanged).
                    query
                        .as_deref()
                        .is_some_and(|q| rule_patterns.get(&r.id).is_some_and(|p| p.is_match(q)))
                } else if let Some(q) = &query {
                    // Pattern and target category name, case-insensitive.
                    r.pattern.to_ascii_lowercase().contains(q)
                        || Self::rule_target_label(&rule_labels, r)
                            .to_ascii_lowercase()
                            .contains(q)
                } else {
                    true
                }
            })
            .collect();
        if test {
            filtered.sort_by(|a, b| {
                a.is_regex
                    .cmp(&b.is_regex)
                    .then_with(|| b.pattern.chars().count().cmp(&a.pattern.chars().count()))
                    .then_with(|| a.id.cmp(&b.id))
            });
            return (
                filtered
                    .into_iter()
                    .map(|r| RulesItem::Rule { id: r.id })
                    .collect(),
                HashMap::new(),
            );
        }
        // Group by target label with a map (3.85, spec): labels are
        // unique per category, so one hash lookup per rule replaces the
        // linear find over groups. The collect + sort below puts the
        // groups in label order whatever order the map iterates.
        let mut grouped: HashMap<String, (i64, Vec<&Rule>)> = HashMap::new();
        for r in &filtered {
            let label = Self::rule_target_label(&rule_labels, r);
            grouped
                .entry(label)
                .or_insert((r.category_id, Vec::new()))
                .1
                .push(r);
        }
        let mut groups: Vec<(String, i64, Vec<&Rule>)> = grouped
            .into_iter()
            .map(|(label, (category_id, rules))| (label, category_id, rules))
            .collect();
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        // 3.58: the heading sort — `None` is Pattern ascending; "Uses"
        // compares the counts and "Total" the Total's absolute amount
        // (largest flow first descending); Pattern breaks ties both ways.
        let sort = self.rules_sort;
        let reversed = matches!(sort, Some((_, SortState::Descending)));
        let uses = &self.rule_uses;
        let totals = &self.rule_totals;
        let mut sums: HashMap<i64, (usize, i64)> = HashMap::new();
        let mut items = Vec::with_capacity(filtered.len() + groups.len());
        for (label, cid, mut rules) in groups {
            let count = rules.len();
            let collapsed = self.collapsed_rule_groups.contains(&cid);
            let group_uses = rules
                .iter()
                .map(|r| uses.get(&r.id).copied().unwrap_or(0))
                .sum();
            let group_total = rules
                .iter()
                .map(|r| totals.get(&r.id).copied().unwrap_or(0))
                .sum();
            sums.insert(cid, (group_uses, group_total));
            items.push(RulesItem::Group {
                id: cid,
                label,
                count,
                collapsed,
            });
            if collapsed {
                continue;
            }
            rules.sort_by(|a, b| {
                let ord = match sort.map(|(col, _)| col) {
                    Some("Uses") => {
                        let ua = uses.get(&a.id).copied().unwrap_or(0);
                        let ub = uses.get(&b.id).copied().unwrap_or(0);
                        ua.cmp(&ub).then_with(|| a.pattern.cmp(&b.pattern))
                    }
                    Some("Total") => {
                        let ta = totals.get(&a.id).copied().unwrap_or(0).abs();
                        let tb = totals.get(&b.id).copied().unwrap_or(0).abs();
                        ta.cmp(&tb).then_with(|| a.pattern.cmp(&b.pattern))
                    }
                    _ => a.pattern.cmp(&b.pattern),
                };
                if reversed { ord.reverse() } else { ord }
            });
            items.extend(rules.into_iter().map(|r| RulesItem::Rule { id: r.id }));
        }
        (items, sums)
    }

    /// 3.85: the cached grouped list (see [`RulesPage::rules_items_cache`]):
    /// [`RulesPage::rules_page_items`] runs only when an input actually
    /// changed, and every frame after that reuses the same `Rc`. 3.58:
    /// the group sums are built in the same pass and swapped in with it.
    pub(super) fn rules_items_mut(&mut self, cx: &App) -> Rc<Vec<RulesItem>> {
        let type_ix = self
            .rules_type_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        let mut collapsed: Vec<i64> = self.collapsed_rule_groups.iter().copied().collect();
        collapsed.sort_unstable();
        let fp = (
            self.rules_query.clone(),
            type_ix,
            self.rules_test_mode,
            self.rules_unused_only,
            self.rules_sort,
            collapsed,
            self.rules_gen,
        );
        if self.rules_items_fp.as_ref() != Some(&fp) {
            let (items, sums) = self.rules_page_items(cx);
            #[cfg(test)]
            self.rules_items_built.set(self.rules_items_built.get() + 1);
            self.rules_items_cache = Some(Rc::new(items));
            self.rules_group_sums = Rc::new(sums);
            self.rules_items_fp = Some(fp);
        }
        Rc::clone(self.rules_items_cache.as_ref().unwrap())
    }

    pub(super) fn toggle_rule_group(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.collapsed_rule_groups.remove(&id) {
            self.collapsed_rule_groups.insert(id);
        }
        cx.notify();
    }

    pub(in crate::ui) fn set_rule_selected(&mut self, id: i64, on: bool, cx: &mut Context<Self>) {
        if on {
            self.selected_rules.insert(id);
        } else {
            self.selected_rules.remove(&id);
        }
        self.rules_move_error = None;
        cx.notify();
    }

    /// The Move bar's action: one [`move_rules`] call moves every checked
    /// rule and re-runs the rules over the unlocked rows once; then the
    /// selection clears and everything refreshes.
    pub(in crate::ui) fn move_selected_rules(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // 3.54: the pick resolves against current items (see the method).
        self.refresh_cat_pickers(window, cx);
        // 3.59: the pick carries the sub-category id value directly.
        let Some(category_id) = self.move_rules_target.read(cx).selected_value().copied() else {
            self.rules_move_error = Some("Choose the category to move to.".into());
            cx.notify();
            return;
        };
        let mut ids: Vec<i64> = self.selected_rules.iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        ids.sort();
        let moved = ids.clone();
        let db = self.db.clone();
        // 3.64: detached — see `commit_new_rule`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_rules_with_use(c, &ids, category_id)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(rule_use) => {
                        // Only the moved rules leave the selection: rules
                        // checked while this move ran keep their checkbox.
                        this.selected_rules.retain(|id| !moved.contains(id));
                        this.rules_move_error = None;
                        cx.emit(PageEvent::ChangedWithRuleUse(Domains::RULES_TX, rule_use));
                    }
                    Err(e) => {
                        this.rules_move_error = Some(e.into());
                        cx.emit(PageEvent::Changed(Domains::RULES_TX));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The *Show rule* item (3.29): open the Rules page on the given rule.
    /// The page's filters are cleared so the rule is certainly in the list
    /// (test mode, the Type picker and Unused could all hide it), its group
    /// is expanded, and the search box takes the rule's pattern — the list
    /// narrows to it find-in-page style, and a rule always matches its own
    /// pattern. The row is scrolled into view and carries the `list_active`
    /// background until the user searches or leaves the page.
    pub(in crate::ui) fn show_rule(
        &mut self,
        rule_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Reached from the dashboard menu, the page may not have been
        // shown since the last change: apply first, so the mirrors name
        // real rows.
        self.apply_if_dirty(cx);
        let Some(rule) = self.rules.iter().find(|r| r.id == rule_id).cloned() else {
            // a refresh dropped the rule between menu and click
            cx.emit(PageEvent::Go(crate::ui::Page::Rules));
            return;
        };
        self.rules_test_mode = false;
        self.rules_unused_only = false;
        self.collapsed_rule_groups.remove(&rule.category_id);
        self.rules_type_select.update(cx, |s, cx| {
            s.set_selected_index(Some(gpui_kit::component::IndexPath::new(0)), window, cx)
        });
        // `set_value` scrolls the list back to the top, so the reveal's
        // `scroll_to_item` must come after it. Any later keystroke fires
        // the search subscription, which drops the reveal.
        let pattern = rule.pattern.clone();
        self.rules_search
            .update(cx, |i, cx| i.set_value(pattern.clone(), window, cx));
        // 3.58: the list filters on the debounced query, so apply this
        // one straight away (the keystroke's own timer re-applies the
        // same text and does nothing) — the index below must name the
        // list as it will render.
        self.rules_search_task = None;
        self.apply_rules_query(&pattern, cx);
        let (items, _) = self.rules_page_items(cx);
        if let Some(ix) = items
            .iter()
            .position(|it| matches!(it, RulesItem::Rule { id } if *id == rule_id))
        {
            self.rules_reveal = Some(rule_id);
            self.rules_scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        // Navigate last (3.80): the emit runs at the end of this update,
        // so `go_page` and `set_visible(true)` — which keep the state set
        // here — cannot clobber the search text or the scroll.
        cx.emit(PageEvent::Go(crate::ui::Page::Rules));
    }

    /// Drop the page's transient state after a restore replaced the whole
    /// database behind the UI (5.3): the ids here would name rows the new
    /// file may not have, or ids reused for other rules.
    pub(in crate::ui) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
        self.close_rule_editor();
        self.armed_delete_rule = None;
        self.new_rule_open = false;
        self.selected_rules.clear();
        self.collapsed_rule_groups.clear();
        self.rules_move_error = None;
        self.rules_reveal = None;
        self.clear_rule_focus();
        cx.notify();
    }

    /// Clears the focused rule and its matches panel (3.52).
    pub(in crate::ui) fn clear_rule_focus(&mut self) {
        self.focused_rule = None;
        self.rule_matches = Rc::new(Vec::new());
        self.rule_matches_task = None;
        self.rule_matches_loading = false;
    }

    /// A rule-row click focuses the rule and opens its matches panel
    /// under the list; clicking the focused row again closes it (3.52).
    /// Clicks are ignored while an editor is open (the row click would
    /// fight the in-place form); the checkbox and the Edit / Delete
    /// buttons stop their clicks, so they never reach this.
    pub(in crate::ui) fn toggle_rule_focus(&mut self, rule_id: i64, cx: &mut Context<Self>) {
        if self.editing_rule.is_some() {
            return;
        }
        if self.focused_rule == Some(rule_id) {
            self.clear_rule_focus();
        } else {
            self.focused_rule = Some(rule_id);
            self.matches_scroll = VirtualListScrollHandle::new();
            self.load_rule_matches(cx);
        }
        cx.notify();
    }

    /// Loads the focused rule's matches through the worker, keyed per
    /// rule id so switching rules supersedes the earlier job (the reader
    /// even interrupts the pending query). A result that arrives after
    /// the focus moved on is dropped; `Superseded` needs no message.
    pub(super) fn load_rule_matches(&mut self, cx: &mut Context<Self>) {
        let Some(rule_id) = self.focused_rule else {
            return;
        };
        self.rule_matches_loading = true;
        let db = self.db.clone();
        self.rule_matches_task = Some(cx.spawn(async move |this, cx| {
            let rows = db
                .call_keyed(&format!("rule_matches:{rule_id}"), move |conn| {
                    rule_matches(conn, rule_id)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.focused_rule != Some(rule_id) {
                    return;
                }
                match rows {
                    Ok(rows) => {
                        this.rule_matches = Rc::new(rows);
                        this.rule_matches_loading = false;
                        cx.notify();
                    }
                    Err(DbError::Superseded) => {}
                    Err(_) => {
                        this.rule_matches_loading = false;
                    }
                }
            });
        }));
    }

    /// The matches panel's columns: Date, the one flex Description,
    /// Account and a right-aligned Amount (3.52, ui-conventions *Rules*).
    fn matches_columns() -> Vec<DataCol> {
        vec![
            DataCol::new("Date", Col::Fixed(110.), ColAlign::Left),
            DataCol::new("Description", Col::Flex, ColAlign::Left),
            DataCol::new("Account", Col::Fixed(140.), ColAlign::Left),
            DataCol::new("Amount", Col::Fixed(110.), ColAlign::Right),
        ]
    }

    /// The panel under the list: a "Transactions matching `PATTERN`"
    /// header, the muted `N · M decided by this rule` summary, and the
    /// matches as a heading-less [`DataList`] — deciding rows plain, the
    /// rest muted with a trailing "won by …" / "locked" tag (3.52).
    pub(super) fn matches_panel(&self, cx: &Context<Self>) -> AnyElement {
        let (fg, muted) = {
            let theme = cx.theme();
            (theme.foreground, theme.muted_foreground)
        };
        let mono = cx.theme().mono_font_family.clone();
        let rows = self.rule_matches.clone();
        let pattern = self.focused_rule.and_then(|id| {
            self.rules
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.pattern.clone())
        });
        let decided = rows
            .iter()
            .filter(|r| r.status == RuleMatchStatus::Decides)
            .count();
        let n = rows.len();
        let body = if rows.is_empty() {
            div()
                .id("rules:matches-empty")
                .debug_selector(|| "rules:matches-empty".into())
                .px_3()
                .py_2()
                .text_sm()
                .text_color(muted)
                .child(if self.rule_matches_loading {
                    "…"
                } else {
                    "This rule matches no transactions"
                })
                .into_any_element()
        } else {
            let pattern_by_id: Rc<HashMap<i64, String>> = Rc::new(
                self.rules
                    .iter()
                    .map(|r| (r.id, r.pattern.clone()))
                    .collect(),
            );
            let items: Vec<DataItem> = rows
                .iter()
                .map(|row| DataItem::Row {
                    id: row.transaction_id as usize,
                    selected: false,
                    selector: format!("rules:match-{}", row.transaction_id),
                })
                .collect();
            DataList::new(cx.entity(), Self::matches_columns(), items)
                .hide_heading()
                .scroll(self.matches_scroll.clone())
                .selector("rules:matches-list")
                .cells(move |_this, item, _measuring, _window, _cx| {
                    let DataItem::Row { id, .. } = item else {
                        return Vec::new();
                    };
                    let Some(row) = rows.iter().find(|r| r.transaction_id == *id as i64) else {
                        return Vec::new();
                    };
                    let strong = row.status == RuleMatchStatus::Decides;
                    let c = if strong { fg } else { muted };
                    let desc = div()
                        .id(SharedString::from(format!(
                            "rules-match-desc-{}",
                            row.transaction_id
                        )))
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(c)
                        .tooltip({
                            let text = row.description.clone();
                            move |window, cx| {
                                let text = text.clone();
                                Tooltip::element(move |_, _| {
                                    let text = text.clone();
                                    let tip = format!("rules:match-tooltip:{text}");
                                    div().debug_selector(move || tip.clone()).child(text)
                                })
                                .build(window, cx)
                            }
                        })
                        .child(row.description.clone())
                        .into_any_element();
                    let tag = match &row.status {
                        RuleMatchStatus::WonBy(winner) => Some(format!(
                            "won by {}",
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
                        .text_color(c)
                        .child(flow::money(row.amount_cents))
                        .when_some(tag, |d, t| {
                            d.child(
                                div()
                                    .id(SharedString::from(format!(
                                        "rules-match-tag-{}",
                                        row.transaction_id
                                    )))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(muted)
                                    .debug_selector({
                                        let sel = format!("rules:match-tag:{t}");
                                        move || sel.clone()
                                    })
                                    .child(t),
                            )
                        });
                    vec![
                        ListCell::from(
                            div()
                                .text_color(c)
                                .child(row.posted_date.clone())
                                .into_any_element(),
                        ),
                        ListCell::from(desc),
                        ListCell::from(
                            div()
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_sm()
                                .text_color(muted)
                                .child(row.account.clone())
                                .into_any_element(),
                        ),
                        ListCell::from(amount.into_any_element()),
                    ]
                })
                .into_any_element()
        };
        v_flex()
            .id("rules:matches-panel")
            .debug_selector(|| "rules:matches-panel".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                v_flex()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .gap_0p5()
                    .flex_shrink_0()
                    .child(
                        h_flex()
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_sm()
                                    .text_color(fg)
                                    .child("Transactions matching"),
                            )
                            .when_some(pattern, |el, pattern| {
                                el.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "rules-matches-pattern-{pattern}"
                                        )))
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_family(mono.clone())
                                        .text_color(fg)
                                        .debug_selector({
                                            let sel = format!("rules:matches-pattern:{pattern}");
                                            move || sel.clone()
                                        })
                                        .tooltip({
                                            let full = pattern.clone();
                                            move |window, cx| {
                                                let text = full.clone();
                                                Tooltip::element(move |_, _| {
                                                    let text = text.clone();
                                                    let sel =
                                                        format!("rules:matches-pattern-tip:{text}");
                                                    div()
                                                        .debug_selector(move || sel.clone())
                                                        .child(text)
                                                })
                                                .build(window, cx)
                                            }
                                        })
                                        .child(pattern),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .debug_selector(move || format!("rules:matches-summary:{n}:{decided}"))
                            .child(format!(
                                "{} · {} decided by this rule",
                                flow::count(n as i64),
                                flow::count(decided as i64)
                            )),
                    ),
            )
            .child(body)
            .into_any_element()
    }
}

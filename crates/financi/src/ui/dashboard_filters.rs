//! The Dashboard page's filter row parts (split out of `dashboard.rs` in
//! 3.74): the accounts and options popovers and their labels.

use super::*;
use crate::ui::widgets::*;

/// The Options button's label (3.35): "Options", or "Options · N" when N
/// of its toggles (include transfers, uncategorized only, show hidden) are
/// on.
fn options_label(on: usize) -> String {
    if on == 0 {
        "Options".to_string()
    } else {
        format!("Options · {on}")
    }
}
/// One filter chip (3.46): the label of an active narrowing filter with a
/// × that clears it. The whole chip is the click target; `id` is the last
/// selector segment (`chip-month`), the chip painting `filters:{id}` and
/// its × `filters:{id}-x`.
fn filter_chip(
    id: &'static str,
    label: String,
    muted: gpui::Hsla,
    hover: gpui::Hsla,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(id))
        .debug_selector(move || format!("filters:{id}"))
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .pl_2()
        .pr_1()
        .py_0p5()
        .rounded_full()
        .cursor_pointer()
        .hover(|c| c.bg(hover))
        .on_click(on_close)
        .child(div().text_xs().child(label))
        .child(
            div()
                .flex_none()
                .debug_selector(move || format!("filters:{id}-x"))
                .child(Icon::new(IconName::X).xsmall().text_color(muted)),
        )
        .into_any_element()
}

// Filter row (3.35): the date-range select's width (fits "Last 12 months"
// on one line), and the search box's floor. The row itself and the shared
// control height moved to `widgets::filter_row` in 3.91.
pub(super) const FILTER_DATE_RANGE_W: Pixels = px(170.);
// 3.46: floor lowered from 140 px — with Clear all shown and the custom
// range open, the row must still fit 1200 × 720 (the search box stays the
// item that takes the slack).
pub(super) const FILTER_SEARCH_MIN_W: Pixels = px(100.);
/// The custom range's From/To pickers: 140 px when there is room, shrinking
/// first, down to [`FILTER_DATE_MIN_W`], when the window is at its minimum
/// and the rest of the row must still fit on the page (3.25, 3.35).
pub(super) const FILTER_DATE_W: Pixels = px(140.);
pub(super) const FILTER_DATE_MIN_W: Pixels = px(110.);
pub(super) const ALL_ACCOUNTS: &str = "All accounts";
/// One label per account for the filter dropdown: the account's own name,
/// or the institution-qualified name when two accounts share it
/// (ui-conventions, *Accounts*).
fn account_labels(accounts: &[Account]) -> Vec<String> {
    accounts
        .iter()
        .map(|a| {
            if accounts.iter().filter(|o| o.name == a.name).count() > 1 {
                a.display_name()
            } else {
                a.name.clone()
            }
        })
        .collect()
}

impl DashboardState {
    /// 3.46: the filter chips row (ui-conventions *Interaction*): each
    /// narrowing filter the row itself doesn't show — the category and
    /// month selections, the search term and "Uncategorized only" — is a
    /// chip with a × that clears just that filter. Dates, accounts,
    /// transfers and "Show hidden" aren't chips (the row and the Options
    /// button show them). Pinned between the filter row and the scroller;
    /// an empty row takes no space.
    pub(super) fn filter_chips_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, hover) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.list_hover)
        };
        let mut chips: Vec<AnyElement> = Vec::new();
        if let Some(name) = self.selected_category_name() {
            chips.push(filter_chip(
                "chip-category",
                name,
                muted,
                hover,
                cx.listener(|this, _: &ClickEvent, _, cx| this.clear_category_chip(cx)),
            ));
        }
        if let Some(month) = self.filter.summary_month.clone() {
            chips.push(filter_chip(
                "chip-month",
                flow::month_label(&month).to_string(),
                muted,
                hover,
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.click_summary_month(&month, cx);
                }),
            ));
        }
        if !self.filter.search.is_empty() {
            let term = self.filter.search.clone();
            chips.push(filter_chip(
                "chip-search",
                format!("“{term}”"),
                muted,
                hover,
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.clear_search_chip(window, cx);
                }),
            ));
        }
        if self.filter.uncategorized_only {
            chips.push(filter_chip(
                "chip-uncategorized",
                "Uncategorized only".to_string(),
                muted,
                hover,
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.set_uncategorized_only(false, window, cx);
                }),
            ));
        }
        if chips.is_empty() {
            return div().into_any_element();
        }
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .debug_selector(|| "filters:chips".into())
            .children(chips)
            .into_any_element()
    }

    /// The name behind `legend_selection`, from the loaded category tree,
    /// for the category chip (3.46).
    fn selected_category_name(&self) -> Option<String> {
        let id = match self.filter.legend_selection {
            Some(LegendSelection::Category(id)) => id,
            _ => return None,
        };
        self.categories.iter().find_map(|p| {
            if p.id == id {
                Some(p.name.clone())
            } else {
                p.children
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.name.clone())
            }
        })
    }

    /// Trigger label of the accounts dropdown: "All accounts" while
    /// nothing is unchecked, otherwise how many of the accounts remain.
    pub(super) fn account_filter_label(&self) -> String {
        let total = self.accounts.len();
        let selected = total.saturating_sub(self.filter.excluded_accounts.len());
        if self.filter.excluded_accounts.is_empty() {
            ALL_ACCOUNTS.to_string()
        } else if selected == 0 {
            "No accounts".to_string()
        } else {
            format!("{selected} of {total} accounts")
        }
    }
    /// Accounts filter dropdown (task 3.3): gpui-kit has no multi-select,
    /// so it is a [`Popover`] of one [`Checkbox`] per account, all checked
    /// by default. The popover stays open while boxes are clicked.
    pub(super) fn accounts_popover(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        let labels = account_labels(&self.accounts);
        let excluded = self.filter.excluded_accounts.clone();
        Popover::new("accounts-filter")
            .trigger(Button::new("accounts-filter-trigger").label(self.account_filter_label()))
            .content(move |_, _, _| {
                v_flex()
                    .gap_1()
                    .children(ids.iter().zip(labels.iter()).map(|(id, name)| {
                        let id = *id;
                        let shell = shell.clone();
                        filter_checkbox(format!("account-{id}"), !excluded.contains(&id))
                            .label(name.clone())
                            .on_click(move |on: &bool, window, cx| {
                                shell.update(cx, |this, cx| {
                                    this.set_account_checked(id, *on, window, cx);
                                });
                            })
                    }))
            })
    }
    /// Options popover (3.35): the three filter toggles that used to sit in
    /// the filter rows — same state and handlers. The trigger counts how
    /// many are on. Right-anchored so it opens inside the page.
    pub(super) fn options_popover(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let (transfers, uncategorized, hidden) = (
            self.filter.include_transfers,
            self.filter.uncategorized_only,
            self.filter.show_hidden,
        );
        let on = [transfers, uncategorized, hidden]
            .iter()
            .filter(|b| **b)
            .count();
        let label = options_label(on);
        Popover::new("filter-options")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("filter-options-trigger")
                    .icon(IconName::ListFilter)
                    // The selector carries the label so tests can pin the
                    // exact words ("Options · 2") the way `label:<text>` does.
                    .debug_selector(|| format!("filters:options-btn:{label}"))
                    .label(label),
            )
            .content(move |_, _, _| {
                let (s1, s2, s3) = (shell.clone(), shell.clone(), shell.clone());
                v_flex()
                    .gap_2()
                    .child(
                        filter_checkbox("include-transfers", transfers)
                            .label("Include transfers")
                            .on_click(move |on: &bool, window, cx| {
                                s1.update(cx, |this, cx| {
                                    this.set_include_transfers(*on, window, cx);
                                });
                            }),
                    )
                    .child(
                        filter_checkbox("uncategorized-only", uncategorized)
                            .label("Uncategorized only")
                            .on_click(move |on: &bool, window, cx| {
                                s2.update(cx, |this, cx| {
                                    this.set_uncategorized_only(*on, window, cx);
                                });
                            }),
                    )
                    .child(
                        filter_checkbox("show-hidden", hidden)
                            .label("Show hidden")
                            .on_click(move |on: &bool, window, cx| {
                                s3.update(cx, |this, cx| {
                                    this.set_show_hidden(*on, window, cx);
                                });
                            }),
                    )
            })
    }
}

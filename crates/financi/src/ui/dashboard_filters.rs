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
// Filter row (3.35): the height every filter control shares, the date-range
// select's width (fits "Last 12 months" on one line), and the search box's
// floor.
pub(super) const FILTER_CONTROL_H: Pixels = px(32.);
pub(super) const FILTER_DATE_RANGE_W: Pixels = px(170.);
pub(super) const FILTER_SEARCH_MIN_W: Pixels = px(140.);
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

impl Shell {
    /// Trigger label of the accounts dropdown: "All accounts" while
    /// nothing is unchecked, otherwise how many of the accounts remain.
    pub(super) fn account_filter_label(&self) -> String {
        let total = self.accounts.len();
        let selected = total.saturating_sub(self.dashboard.excluded_accounts.len());
        if self.dashboard.excluded_accounts.is_empty() {
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
        let excluded = self.dashboard.excluded_accounts.clone();
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
            self.dashboard.include_transfers,
            self.dashboard.uncategorized_only,
            self.dashboard.show_hidden,
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

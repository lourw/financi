//! Pickers, labels and widths shared by the Categories page's panes: the
//! parent-scoped add-rule picker's item type and builders (3.53), the
//! subcategory Select sizing, the kind labels and the action-row strings.

use super::*;
/// 3.53: one subcategory entry of the Categories page's parent-scoped
/// add-rule picker. Plain text as shown, plus debug selectors on the open
/// menu's row so headless tests can measure the drawn text against the
/// menu's width: the visible box fills the row, and an absolutely
/// positioned invisible copy keeps its natural (unclipped) text width —
/// the two bound each other exactly when the name fits.
#[derive(Clone, PartialEq)]
pub(in crate::ui) struct RuleChoice(SharedString);

impl SearchableListItem for RuleChoice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.0.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let full = self.0.clone();
        let probe = self.0.clone();
        div()
            .id("rule-choice")
            .debug_selector(move || format!("cat:rule-choice:{full}"))
            .w_full()
            .min_w_0()
            .relative()
            .child(self.0.clone())
            .child(
                div()
                    .id("rule-choice-natural")
                    .debug_selector({
                        let probe = probe.clone();
                        move || format!("cat:rule-choice-natural:{probe}")
                    })
                    .absolute()
                    .whitespace_nowrap()
                    .opacity(0.)
                    .child(probe),
            )
    }
}

/// 3.53: the parent-scoped picker's delegate. `SearchableVec` (not a
/// plain `Vec`) so the menu's search box actually narrows the list as you
/// type, like every other picker that can hold long names (3.59).
pub(in crate::ui) type RuleChoices = SearchableVec<RuleChoice>;

pub(in crate::ui) fn rule_choices(names: &[(i64, String)]) -> RuleChoices {
    SearchableVec::new(
        names
            .iter()
            .map(|(_, n)| RuleChoice(SharedString::from(n.clone())))
            .collect::<Vec<_>>(),
    )
}

/// 3.53: the Categories page's parent-scoped add-rule picker (the chosen
/// parent's subcategories by their own names). `trigger_w` fits the
/// longest child name up to 200 px — longer names ellipsize, and the
/// trigger carries a hover tooltip with the full name (ui-conventions
/// *Interaction*) — and the open menu is `menu_w` wide (at least the
/// longest name, max [`RULE_CATEGORY_MENU_W`]), so no item is cut off.
/// `id` tells the rows apart (`cat:rule-subcategory-select:<id>`).
pub(super) fn subcategory_select(
    target: &Entity<SelectState<RuleChoices>>,
    id: &'static str,
    trigger_w: f32,
    menu_w: f32,
    cx: &App,
) -> Stateful<Div> {
    let state = target.clone();
    let wrapper = div()
        .id(SharedString::from(id))
        .w(px(trigger_w))
        .flex_shrink_0()
        .debug_selector(move || format!("cat:rule-subcategory-select:{id}"));
    let tip = target.read(cx).selected_value().cloned();
    let wrapper = match tip {
        Some(fallback) => wrapper.tooltip(move |window, cx| {
            let shown = state
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            let for_text = shown.clone();
            Tooltip::element(move |_, _| {
                div()
                    .debug_selector({
                        let for_sel = shown.clone();
                        move || format!("cat:category-tooltip:{for_sel}")
                    })
                    .child(for_text.clone())
            })
            .build(window, cx)
        }),
        None => wrapper,
    };
    wrapper.child(
        div()
            .debug_selector(move || format!("label:{SUBCATEGORY_PH}"))
            .child(
                Select::new(target)
                    .menu_width(px(menu_w))
                    .placeholder(SUBCATEGORY_PH),
            ),
    )
}

/// 3.53: the Select chrome beside the label text — horizontal padding
/// plus the chevron — generous so the width estimates below err towards
/// controls that are too wide rather than names that clip (same spirit
/// as [`TEXT_SM_CHAR_PX`]).
pub(in crate::ui) const SELECT_CHROME_W: f32 = 40.;
/// 3.53: floor for the parent-scoped picker: its placeholder
/// ("Subcategory…") must fit whatever the child names are.
const SUBCATEGORY_MIN_W: f32 = 130.;

/// 3.53: widths for the parent-scoped picker from the longest child
/// name's character count — the trigger up to [`RULE_CATEGORY_W`] (names
/// past that ellipsize with a tooltip), the menu up to
/// [`RULE_CATEGORY_MENU_W`] and never narrower than the trigger.
/// Estimated, never measured: `measure_text_width` exists since 3.86 —
/// the Undo column measures — but this picker keeps the cheap estimate
/// from [`TEXT_SM_CHAR_PX`], which errs wide.
pub(super) fn subcategory_widths(longest_chars: usize) -> (f32, f32) {
    let est = longest_chars as f32 * TEXT_SM_CHAR_PX + SELECT_CHROME_W;
    let trigger = est.clamp(SUBCATEGORY_MIN_W, RULE_CATEGORY_W);
    (trigger, est.max(trigger).min(RULE_CATEGORY_MENU_W))
}

/// Words for Categories page controls whose painted text is reachable by
/// tests only through debug selectors (3.33): each const feeds both the
/// label/placeholder and its `label:<text>` selector, so the two can't
/// desync and the scan test pins the exact words.
pub(super) const ADD_CATEGORY: &str = "Add category";
pub(super) const ADD_SUBCATEGORY: &str = "Add subcategory";
pub(super) const MOVE_TO_CATEGORY_PH: &str = "Move to category…";
pub(super) const MOVE_TXNS_TO_PH: &str = "Move its transactions to…";
/// 3.53: the parent-scoped add-rule picker's placeholder; like the Move
/// pickers, its text is pinned by a `label:<text>` selector on the
/// control's wrapper.
pub(in crate::ui) const SUBCATEGORY_PH: &str = "Subcategory…";
/// 3.23: the Categories page's header action; it opens the new-category
/// form in place under the header (ui-conventions, *Pages*). 3.43: no
/// trailing "…" — the button carries a `plus` icon instead.
pub(in crate::ui) const NEW_CATEGORY: &str = "New category";
/// 3.49: the Uses heading's tooltip. H5: the tip's text and its
/// `cat:rule-heading-uses-tip:<text>` selector both read this const, so
/// the two can't desync and the hover test pins the exact words (the
/// 3.33 scheme above).
pub(super) const RULE_USES_TIP: &str = "Transactions this rule categorizes";
pub(in crate::ui) const KIND_LABELS: [(&str, CategoryKind); 3] = [
    ("Expense", CategoryKind::Expense),
    ("Income", CategoryKind::Income),
    ("Transfer", CategoryKind::Transfer),
];

pub(super) fn kind_label(kind: CategoryKind) -> &'static str {
    KIND_LABELS
        .iter()
        .find(|(_, k)| *k == kind)
        .map_or("Expense", |(l, _)| l)
}

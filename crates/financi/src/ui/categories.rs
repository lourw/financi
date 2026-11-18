//! The Categories page: the category tree and its rule list (3.62).
//!
//! 3.81: the page is its own entity (the 3.79 pattern, `ui/page.rs`). It
//! renders the tree and the detail pane from the `Catalog` snapshot, and
//! owns the selection, the new-category form, the rename editor and the
//! delete confirmation. The rule *editing* still lives in `Shell` with
//! the Rules page (3.80 moves it); the page renders its rule rows through
//! `Shell::rule_row` / `Shell::rule_editor_row` — the very same renderers
//! — so a rule takes exactly one row wherever it appears, and until 3.80
//! the page keeps a weak handle back to the shell for those rows and for
//! the shared add-rule row (a documented exception to the pattern, going
//! away with the rule list).
//!
//! While that handle exists, the selection is deliberately mirrored: the
//! page's `selected_category` drives the tree, and
//! `Shell::categories_page.selected_category` is the copy the Rules
//! page's add-rule flow and its tests read. `Shell::select_category` and
//! the page's row click each write both, through the two halves
//! (`Shell::select_category_host` and `CategoriesPage::note_selected`)
//! that together do what one function did before the extraction.

use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;
// 3.51: `Button::selected` — the open form's title-row button reads as
// selected. `ui/mod.rs` re-exports the other component traits; this one
// is imported here (the module-level import list belongs to the shell).
use gpui_kit::base::Selectable as _;

/// 3.53: one subcategory entry of the Categories page's parent-scoped
/// add-rule picker. Plain text as shown, plus debug selectors on the open
/// menu's row so headless tests can measure the drawn text against the
/// menu's width: the visible box fills the row, and an absolutely
/// positioned invisible copy keeps its natural (unclipped) text width —
/// the two bound each other exactly when the name fits.
#[derive(Clone, PartialEq)]
pub(super) struct RuleChoice(SharedString);

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
pub(super) type RuleChoices = SearchableVec<RuleChoice>;

pub(super) fn rule_choices(names: &[(i64, String)]) -> RuleChoices {
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
fn subcategory_select(
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
pub(super) const SELECT_CHROME_W: f32 = 40.;
/// 3.53: floor for the parent-scoped picker: its placeholder
/// ("Subcategory…") must fit whatever the child names are.
const SUBCATEGORY_MIN_W: f32 = 130.;

/// 3.53: widths for the parent-scoped picker from the longest child
/// name's character count — the trigger up to [`RULE_CATEGORY_W`] (names
/// past that ellipsize with a tooltip), the menu up to
/// [`RULE_CATEGORY_MENU_W`] and never narrower than the trigger.
/// Estimated, never measured: the app has no text-measurement path here
/// (the Undo column does the same, see [`TEXT_SM_CHAR_PX`]).
fn subcategory_widths(longest_chars: usize) -> (f32, f32) {
    let est = longest_chars as f32 * TEXT_SM_CHAR_PX + SELECT_CHROME_W;
    let trigger = est.clamp(SUBCATEGORY_MIN_W, RULE_CATEGORY_W);
    (trigger, est.max(trigger).min(RULE_CATEGORY_MENU_W))
}

/// Words for Categories page controls whose painted text is reachable by
/// tests only through debug selectors (3.33): each const feeds both the
/// label/placeholder and its `label:<text>` selector, so the two can't
/// desync and the scan test pins the exact words.
const ADD_CATEGORY: &str = "Add category";
const ADD_SUBCATEGORY: &str = "Add subcategory";
const MOVE_TO_CATEGORY_PH: &str = "Move to category…";
const MOVE_TXNS_TO_PH: &str = "Move its transactions to…";
/// 3.53: the parent-scoped add-rule picker's placeholder; like the Move
/// pickers, its text is pinned by a `label:<text>` selector on the
/// control's wrapper.
pub(super) const SUBCATEGORY_PH: &str = "Subcategory…";
/// 3.23: the Categories page's header action; it opens the new-category
/// form in place under the header (ui-conventions, *Pages*). 3.43: no
/// trailing "…" — the button carries a `plus` icon instead.
pub(super) const NEW_CATEGORY: &str = "New category";
pub(super) const KIND_LABELS: [(&str, CategoryKind); 3] = [
    ("Expense", CategoryKind::Expense),
    ("Income", CategoryKind::Income),
    ("Transfer", CategoryKind::Transfer),
];

fn kind_label(kind: CategoryKind) -> &'static str {
    KIND_LABELS
        .iter()
        .find(|(_, k)| *k == kind)
        .map_or("Expense", |(l, _)| l)
}

/// The state the category flow still shares with the Rules page while
/// the rule list lives there (3.81): the selected-category mirror the
/// add-rule flow reads, the parent-scoped add-rule picker, and the two
/// virtual lists' test counters (the page writes them through the shared
/// `Rc`s it takes in `share_counters`). This scaffold goes away when the
/// Rules page becomes its own entity (3.80).
pub(super) struct CategoriesState {
    /// The selected category, mirrored from the page (see the module
    /// docs): `rules.rs` reads it next to the tree's own copy.
    pub(super) selected_category: Option<i64>,
    /// 3.53: the Categories page's add-rule picker, shown only when the
    /// selection is a parent: its own child categories by their own names
    /// (no "Parent › Child"), searchable, the wide menu and the
    /// full-name tooltip. `cat_rule_child_ids` maps the picker's rows to
    /// category ids (the items carry names only).
    pub(super) cat_rule_target: Entity<SelectState<RuleChoices>>,
    pub(super) cat_rule_child_ids: Vec<i64>,
    /// 3.21: how many tree rows the virtualized list built for the last
    /// layout pass — the visible slice, not the whole tree. Observability
    /// for the long-list tests. Shared with the page (3.81) so the rules
    /// tests keep reading the counters here.
    #[cfg(test)]
    pub(super) tree_rows_built: Rc<std::cell::Cell<usize>>,
    /// 3.21: the range of rows the tree built last frame, so the scroll
    /// tests can see the window move.
    #[cfg(test)]
    pub(super) tree_range: Rc<std::cell::Cell<(usize, usize)>>,
    #[cfg(test)]
    pub(super) rule_rows_built: Rc<std::cell::Cell<usize>>,
    /// 3.21: the range of rows the rule list built last frame.
    #[cfg(test)]
    pub(super) rule_range: Rc<std::cell::Cell<(usize, usize)>>,
}

impl CategoriesState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        // 3.53: the Categories page's parent-scoped subcategory picker —
        // searchable, wide menu, full-name tooltip (see `subcategory_select`).
        let cat_rule_target =
            cx.new(|cx| SelectState::new(rule_choices(&[]), None, window, cx).searchable(true));
        Self {
            selected_category: None,
            cat_rule_target,
            cat_rule_child_ids: Vec::new(),
            #[cfg(test)]
            tree_rows_built: Rc::new(std::cell::Cell::new(0)),
            #[cfg(test)]
            tree_range: Rc::new(std::cell::Cell::new((0, 0))),
            #[cfg(test)]
            rule_rows_built: Rc::new(std::cell::Cell::new(0)),
            #[cfg(test)]
            rule_range: Rc::new(std::cell::Cell::new((0, 0))),
        }
    }
}

impl Shell {
    /// Apply this page's part of a refresh result (3.74): the shared tree
    /// and counts. `Shell` keeps the mirror the Rules page and the
    /// category-color palette read; the Categories page applies its own
    /// copy from the same snapshot (3.81).
    pub(super) fn apply_categories(&mut self, tree: Vec<CategoryNode>, counts: HashMap<i64, i64>) {
        self.categories = tree;
        self.cat_counts = counts;
    }
}

impl Shell {
    /// Recompute the derived choice lists after a refresh and drop stale
    /// selection state.
    pub(super) fn sync_category_choices(
        &mut self,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let known = |id: i64| {
            self.categories
                .iter()
                .any(|p| p.id == id || p.children.iter().any(|c| c.id == id))
        };
        // 3.81: the selection and the armed delete live on the page now;
        // read them once, then prune both copies (the scaffold's too —
        // the Rules page reads it next to the tree).
        let (selected, confirm) = {
            let page = self.categories_view.read(cx);
            (page.selected_category, page.confirm_delete_category)
        };
        // 3.78 review fix: the tree this refresh loaded is older than the
        // category the add wrote — the read and the write run on different
        // queues, and a reader job queued before the write can land after
        // it. Dropping the selection then loses a row the user just added
        // and is looking at, so the generation that carried the new
        // selection settles first and this stale pass leaves it alone.
        if generation >= self.category_selection_settled && selected.is_some_and(|id| !known(id)) {
            self.categories_page.selected_category = None;
            self.categories_view.update(cx, |page, _| {
                page.selected_category = None;
                page.renaming = false;
            });
        }
        // 3.82 review fix: the same staleness guard as the selection
        // prune above — a tree read dispatched before this category
        // existed must not dismiss a delete the user just armed.
        if generation >= self.category_selection_settled && confirm.is_some_and(|id| !known(id)) {
            self.categories_view
                .update(cx, |page, _| page.confirm_delete_category = None);
        }
        // 3.29: the system `Transfers > Card Payment` category, so the
        // dashboard menu can tell a built-in card payment from a hand-set
        // locked row. Same name-and-parent lookup `financi-db` uses; ids are
        // never hardcoded.
        self.card_payment_category = self
            .categories
            .iter()
            .find(|p| p.name == "Transfers")
            .and_then(|p| {
                p.children
                    .iter()
                    .find(|c| c.is_system && c.name == "Card Payment")
                    .map(|c| c.id)
            });
        self.child_choices = self
            .categories
            .iter()
            .flat_map(|p| {
                p.children
                    .iter()
                    .map(move |c| (c.id, format!("{} › {}", p.name, c.name)))
            })
            .collect();
        // 3.80: the Rules page owns its pickers, its checked rows and its
        // editor; fold in any refresh it missed first — pruning stale
        // checks, collapsed groups and a vanished editor lives in its
        // `apply`, and hidden pages only fold on demand. 3.59: the picker
        // feeds moved to `Shell::data_changed` (shared grouped items,
        // which needs the window this path has).
        self.rules_page.update(cx, |p, cx| p.apply_if_dirty(cx));
        // 3.53: the Categories add-rule picker follows the selection and
        // the (possibly renamed) child list; the pick survives while its
        // id is one of the parent's children.
        self.sync_cat_rule_picker(false, window, cx);
    }

    /// 3.53: the selected parent's subcategories by their own names (no
    /// "Parent › Child"), empty when the selection is a subcategory or
    /// none. Order matches the tree.
    fn parent_child_choices(&self) -> Vec<(i64, String)> {
        let Some(parent_id) = self.categories_page.selected_category else {
            return Vec::new();
        };
        // Only a parent selection has children to offer the picker; a
        // subcategory targets itself, so the picker is not shown.
        self.categories
            .iter()
            .find(|p| p.id == parent_id)
            .map(|p| p.children.iter().map(|c| (c.id, c.name.clone())).collect())
            .unwrap_or_default()
    }

    /// 3.53: repoint the Categories add-rule picker at the selected
    /// parent's subcategories. `reset` clears the pick (a fresh selection
    /// never pre-picks); a refresh keeps the pick when its id is still one
    /// of the parent's children.
    fn sync_cat_rule_picker(&mut self, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.parent_child_choices();
        let keep = if reset {
            None
        } else {
            self.categories_page
                .cat_rule_target
                .read(cx)
                .selected_index(cx)
                .and_then(|ix| self.categories_page.cat_rule_child_ids.get(ix.row).copied())
        };
        self.categories_page.cat_rule_child_ids = choices.iter().map(|(id, _)| *id).collect();
        let selected = keep.and_then(|id| {
            self.categories_page
                .cat_rule_child_ids
                .iter()
                .position(|c| *c == id)
        });
        self.categories_page.cat_rule_target.update(cx, |s, cx| {
            s.set_items(rule_choices(&choices), window, cx);
            s.set_selected_index(
                selected.map(gpui_kit::component::IndexPath::new),
                window,
                cx,
            );
        });
    }

    /// The selected node as (node, parent), parents having `None`. Reads
    /// the mirror, so the Rules page's add-rule flow (3.53) resolves the
    /// selection the Categories page shows.
    pub(super) fn selected_node(&self) -> Option<(&CategoryNode, Option<&CategoryNode>)> {
        let id = self.categories_page.selected_category?;
        for p in &self.categories {
            if p.id == id {
                return Some((p, None));
            }
            if let Some(c) = p.children.iter().find(|c| c.id == id) {
                return Some((c, Some(p)));
            }
        }
        None
    }

    /// Point the rule list at a category (3.13, 3.81): the `Shell` half.
    ///
    /// The page's copy is written by [`CategoriesPage::note_selected`];
    /// the two together do what one `select_category` did before the page
    /// was extracted, so the in-flight-selection marker (3.53) and the
    /// rule editor stay in step whichever entity the click arrives on.
    /// Must not touch `categories_view` — the page calls this from inside
    /// its own update.
    fn select_category_host(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.categories_page.selected_category != Some(id);
        // 3.78 review fix: from the next dispatch on, a refresh result
        // was queried with this selection in mind and may prune it;
        // results of older dispatches may not (see the field docs).
        self.category_selection_settled = self.refresh_started.get() + 1;
        self.categories_page.selected_category = Some(id);
        // 3.24: selecting another category closes the rule editor (with its
        // preview count) and disarms any armed rule delete — the same
        // "transient UI belongs to what it was opened on" rule as the page
        // switch. 3.80: that editor state lives on the Rules page.
        self.rules_page.update(cx, |p, _| {
            p.close_rule_editor();
            p.armed_delete_rule = None;
            // 3.52: the focused rule belongs to the Rules page as a
            // whole, but a category selection closes its matches panel
            // too — same "transient UI" rule as the editor above.
            p.clear_rule_focus();
        });
        // 3.53: the add-rule picker belongs to the selection: a
        // subcategory needs none, a parent picks among its own
        // children — never a default pick.
        if changed {
            self.sync_cat_rule_picker(true, window, cx);
        }
        cx.notify();
    }

    /// Point the rule list at a category (3.13), both halves: the tests'
    /// way in, and what the page's row click drives from the other side.
    #[cfg(test)]
    pub(super) fn select_category(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.select_category_host(id, window, cx);
        self.categories_view
            .update(cx, |page, cx| page.note_selected(id, window, cx));
    }

    #[cfg(test)]
    pub(super) fn add_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.categories_view
            .update(cx, |page, cx| page.add_parent(window, cx));
    }

    #[cfg(test)]
    pub(super) fn add_child(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.categories_view
            .update(cx, |page, cx| page.add_child(window, cx));
    }

    #[cfg(test)]
    pub(super) fn move_child_to_selected_parent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.categories_view.update(cx, |page, cx| {
            page.move_child_to_selected_parent(window, cx)
        });
    }

    #[cfg(test)]
    pub(super) fn arm_delete_category(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 3.59: the reassign target resolves from the picker's item value,
        // so the page needs no mirror of the shell's list here.
        self.categories_view
            .update(cx, move |page, cx| page.arm_delete_category(window, cx));
    }
}
/// Which of the detail panel's inline forms (3.51) is open, one at a
/// time. Opening one closes another; Cancel, Escape, a successful action,
/// a new selection or leaving the page closes the open one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailForm {
    AddSubcategory,
    Move,
    Delete,
}

/// Which category write the DB is still running (3.84), one at a time.
/// The matching button renders `.loading(true)` and disabled while it is,
/// so a slow write can't be double-submitted — the `PendingRuleAction`
/// pattern from 3.40, applied to this page's five writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingCatWrite {
    AddParent,
    AddChild,
    Rename,
    Move,
    Delete,
}

/// The Categories page (3.74 state, 3.81 entity): the category tree, the
/// detail pane for the selected category (rename, subcategory, move,
/// delete), and the category's rule list — the rule rows themselves are
/// `Shell`'s, rendered through the weak handle until 3.80 moves the rule
/// list to its own page (see the module docs).
pub(super) struct CategoriesPage {
    db: DbWorker,
    /// 5.3 review fix: shared with the shell — while an export or restore
    /// is in flight every write here is refused.
    writes_blocked: Rc<Cell<bool>>,
    /// The shared data model (3.78); `apply` reads the tree, the counts
    /// and the rules off its snapshot, never a query of its own.
    catalog: Entity<Catalog>,
    /// 3.81 transitional: the rule rows, the add-rule row and the shared
    /// palette live in `Shell`; the rule *state* is the Rules page's (3.80),
    /// read and written through this handle.
    shell: WeakEntity<Shell>,
    /// 3.80: the Rules page entity. The Categories page renders the shared
    /// rule rows and the per-category add-rule editor against this page's
    /// state (the same editor the Rules page opens), so the two stay in
    /// step whichever page the click arrives on.
    rules_page: Entity<RulesPage>,
    // `pub(super)` like the other extracted pages: the Escape tests focus
    // the page root the way `go_page` does (see `ui/tests/categories.rs`).
    pub(super) focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// 3.84: the category write the DB is still running, if any. While
    /// one is in flight its button renders loading and disabled, and a
    /// double-submit can't start a second write (`begin_cat_write`).
    pub(super) cat_pending: Option<PendingCatWrite>,
    /// 3.54: the shared grouped items last fed to `delete_target`, and
    /// the flag that a newer list arrived between `apply` and `render` —
    /// the queue page's lazy-picker pattern (see `RulesPage`): a hidden
    /// page does not pay for `set_items` until it is shown again.
    cat_choices_held: Option<Rc<Vec<SelectGroup<SubChoice>>>>,
    cat_pickers_need_items: bool,
    /// 3.54 test hook: how many picker `set_items` calls this page made.
    #[cfg(test)]
    pub(super) cat_picker_sets: std::cell::Cell<usize>,
    /// The `Catalog` subscription (3.79); lives as long as the page.
    _catalog: Subscription,
    /// 3.96: Enter in the new-subcategory input adds the subcategory
    /// (ui-conventions *Keyboard*: "Enter submits the focused form"), so
    /// several names go in a row from the open form.
    _new_child_enter: Subscription,
    // -- Snapshot mirrors (3.81). `uses` (3.49) is the per-rule count the
    // rule list's Uses column shows — the same `rule_use_stats` mirror the
    // Rules page reads, taken off the shared snapshot rather than the
    // Rules page's own copy, which only refreshes when that page applies.
    pub(super) tree: Vec<CategoryNode>,
    pub(super) counts: HashMap<i64, i64>,
    pub(super) rules: Vec<Rule>,
    pub(super) uses: HashMap<i64, usize>,
    // -- State (3.74).
    /// The category the detail pane shows; the shell keeps a mirror of
    /// this for the rule list (see the module docs).
    pub(super) selected_category: Option<i64>,
    pub(super) new_parent_name: Entity<InputState>,
    pub(super) new_parent_kind: Entity<SelectState<Names>>,
    pub(super) new_child_name: Entity<InputState>,
    /// 3.23: the New category form under the page header is open. The
    /// header button opens it, Cancel and a successful add close it, and
    /// `go_page` clears it like the page's other transient UI.
    pub(super) new_category_open: bool,
    /// 3.49: the rule list sorts by Uses (descending) while this is on;
    /// off is the default pattern order. The Rules page keeps its own copy.
    pub(super) cat_rules_sort_by_uses: bool,
    pub(super) renaming: bool,
    pub(super) rename_input: Entity<InputState>,
    pub(super) move_target: Entity<SelectState<Names>>,
    /// 3.59: the Delete form's reassign picker — the sub-categories
    /// grouped by parent and searchable, sharing `Catalog`'s items (fed
    /// by `Shell::data_changed`, where the old flat list was).
    pub(super) delete_target: Entity<SelectState<SubChoices>>,
    pub(super) confirm_delete_category: Option<i64>,
    /// 3.51: the detail panel's open inline form, one at a time; the
    /// title-row button of the open form shows it as selected.
    pub(super) detail_form: Option<DetailForm>,
    pub(super) cat_error: Option<String>,
    /// 3.21: one scroll handle per virtualized list, alive as long as the
    /// page (see the task's note on why per-render handles snap back).
    pub(super) tree_scroll: VirtualListScrollHandle,
    pub(super) rule_scroll: VirtualListScrollHandle,
    /// The counters the scaffold owns and the rules tests read (3.81);
    /// `share_counters` points these at the shell's copies.
    #[cfg(test)]
    tree_rows_built: Rc<std::cell::Cell<usize>>,
    #[cfg(test)]
    tree_range: Rc<std::cell::Cell<(usize, usize)>>,
    #[cfg(test)]
    rule_rows_built: Rc<std::cell::Cell<usize>>,
    #[cfg(test)]
    rule_range: Rc<std::cell::Cell<(usize, usize)>>,
}

impl EventEmitter<PageEvent> for CategoriesPage {}

impl PageEntity for CategoriesPage {
    /// The tree, the counts and the rule list: categories and rules
    /// obviously, and transactions too — the per-category counts move
    /// with every categorization.
    const DEPENDS: Domains = Domains::CATEGORIES_RULES_TX;

    fn visible(&self) -> bool {
        self.visible
    }

    fn dirty(&self) -> bool {
        self.dirty
    }

    fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    fn set_shown(&mut self, visible: bool) {
        self.visible = visible;
    }

    fn set_focus_pending(&mut self, pending: bool) {
        self.focus_pending = pending;
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let (tree, counts, rules, uses, choices) = {
            let cat = self.catalog.read(cx);
            (
                cat.snap.tree.clone(),
                cat.snap.counts.clone(),
                cat.snap.rules.clone(),
                cat.snap.uses.clone(),
                cat.cat_choices(),
            )
        };
        self.tree = tree;
        self.counts = counts;
        self.rules = rules;
        self.uses = uses;
        // 3.54: a new shared picker list lands in `render`, where the
        // window for `set_items` exists — a hidden page only flags it
        // (the queue page's 3.19/3.83 pattern).
        if self
            .cat_choices_held
            .as_ref()
            .is_none_or(|held| !Rc::ptr_eq(held, &choices))
        {
            self.cat_choices_held = Some(choices);
            self.cat_pickers_need_items = true;
        }
    }
}

impl CategoriesPage {
    pub(super) fn new(
        db: DbWorker,
        writes_blocked: Rc<Cell<bool>>,
        catalog: Entity<Catalog>,
        rules_page: Entity<RulesPage>,
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let _catalog = watch_catalog(&catalog, cx);
        let new_parent_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("New category name"));
        let new_parent_kind = cx.new(|cx| {
            SelectState::new(
                KIND_LABELS.iter().map(|(l, _)| (*l).into()).collect(),
                Some(gpui_kit::component::IndexPath::new(0)),
                window,
                cx,
            )
        });
        let new_child_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("New subcategory name"));
        // 3.96: Enter submits the open Add subcategory form (ui-conventions
        // *Keyboard*). The keystroke can only reach this input while the
        // form is open; while the write is in flight the Enter does what the
        // disabled Add button does — nothing (3.84).
        let _new_child_enter = cx.subscribe_in(
            &new_child_name,
            window,
            |this, _input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter {
                    secondary: false, ..
                } if this.detail_form == Some(DetailForm::AddSubcategory)
                    && this.cat_pending != Some(PendingCatWrite::AddChild) =>
                {
                    this.add_child(window, cx)
                }
                _ => {}
            },
        );
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New name"));
        let move_target =
            cx.new(|cx| SelectState::new(Names::new(), None, window, cx).searchable(true));
        let delete_target = cx
            .new(|cx| SelectState::new(SubChoices::new(vec![]), None, window, cx).searchable(true));
        Self {
            db,
            writes_blocked,
            catalog,
            rules_page,
            shell,
            focus: cx.focus_handle(),
            // `Shell::new` marks the initial page visible right after
            // construction; every page starts hidden.
            visible: false,
            dirty: true,
            focus_pending: false,
            _catalog,
            _new_child_enter,
            tree: Vec::new(),
            counts: HashMap::new(),
            rules: Vec::new(),
            uses: HashMap::new(),
            selected_category: None,
            new_parent_name,
            new_parent_kind,
            new_child_name,
            new_category_open: false,
            cat_rules_sort_by_uses: false,
            renaming: false,
            rename_input,
            move_target,
            delete_target,
            confirm_delete_category: None,
            cat_pending: None,
            cat_choices_held: None,
            cat_pickers_need_items: false,
            #[cfg(test)]
            cat_picker_sets: std::cell::Cell::new(0),
            detail_form: None,
            cat_error: None,
            tree_scroll: VirtualListScrollHandle::new(),
            rule_scroll: VirtualListScrollHandle::new(),
            #[cfg(test)]
            tree_rows_built: Rc::new(std::cell::Cell::new(0)),
            #[cfg(test)]
            tree_range: Rc::new(std::cell::Cell::new((0, 0))),
            #[cfg(test)]
            rule_rows_built: Rc::new(std::cell::Cell::new(0)),
            #[cfg(test)]
            rule_range: Rc::new(std::cell::Cell::new((0, 0))),
        }
    }

    /// Point the page's counters at the scaffold's copies, so the rules
    /// tests that read `Shell::categories_page` still see what the page's
    /// lists drew (3.81).
    #[cfg(test)]
    pub(super) fn share_counters(&mut self, scaffold: &CategoriesState) {
        self.tree_rows_built = scaffold.tree_rows_built.clone();
        self.tree_range = scaffold.tree_range.clone();
        self.rule_rows_built = scaffold.rule_rows_built.clone();
        self.rule_range = scaffold.rule_range.clone();
    }

    /// The selected node as (node, parent), parents having `None`.
    fn selected_node(&self) -> Option<(&CategoryNode, Option<&CategoryNode>)> {
        let id = self.selected_category?;
        for p in &self.tree {
            if p.id == id {
                return Some((p, None));
            }
            if let Some(c) = p.children.iter().find(|c| c.id == id) {
                return Some((c, Some(p)));
            }
        }
        None
    }

    /// Visible transaction count: a parent owns none itself, the sum of
    /// its children stands for it.
    fn node_count(&self, node: &CategoryNode) -> i64 {
        if node.children.is_empty() {
            self.counts.get(&node.id).copied().unwrap_or(0)
        } else {
            node.children
                .iter()
                .map(|c| self.counts.get(&c.id).copied().unwrap_or(0))
                .sum()
        }
    }

    /// 3.23: the rules the Categories page lists — the ones targeting the
    /// selected category, which for a parent means its subcategories' too
    /// (rules always target a child, but count the parent's own id in case
    /// one ever lands there). Nothing without a selection. 3.49: default
    /// order is pattern ascending; the Uses heading switches it to Uses
    /// descending with pattern as the tie-break, like the Rules page.
    fn visible_rules(&self) -> Vec<Rule> {
        let Some(id) = self.selected_category else {
            return Vec::new();
        };
        let mut ids: HashSet<i64> = [id].into_iter().collect();
        if let Some(parent) = self.tree.iter().find(|p| p.id == id) {
            ids.extend(parent.children.iter().map(|c| c.id));
        }
        let uses = &self.uses;
        let by_uses = self.cat_rules_sort_by_uses;
        let mut rules: Vec<Rule> = self
            .rules
            .iter()
            .filter(|r| ids.contains(&r.category_id))
            .cloned()
            .collect();
        rules.sort_by(|a, b| {
            if by_uses {
                let ua = uses.get(&a.id).copied().unwrap_or(0);
                let ub = uses.get(&b.id).copied().unwrap_or(0);
                ub.cmp(&ua).then_with(|| a.pattern.cmp(&b.pattern))
            } else {
                a.pattern.cmp(&b.pattern)
            }
        });
        rules
    }

    /// Point the detail pane at a category: the page's half (3.81), also
    /// run by `Shell::select_category` so both copies of the selection
    /// move together.
    fn note_selected(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // The selection resolves against the tree, so act on a current
        // mirror even when this arrives while the page is not shown.
        self.apply_if_dirty(cx);
        let changed = self.selected_category != Some(id);
        self.selected_category = Some(id);
        self.renaming = false;
        self.confirm_delete_category = None;
        // 3.51: an inline form belongs to the category it was opened on.
        self.detail_form = None;
        self.cat_error = None;
        // 3.23: the rule list now shows only this category's rules, so its
        // content is replaced with the selection and the scroll offset
        // resets (the reset point 3.21 documented).
        if changed {
            self.rule_scroll.set_offset(Point::default());
        }
        self.rebuild_move_target(window, cx);
        cx.notify();
    }

    /// A tree row was clicked: move the page's selection and the shell's
    /// rule-list half (see `select_category_host`).
    fn select_category(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.note_selected(id, window, cx);
        if let Some(shell) = self.shell.upgrade() {
            shell.update(cx, |shell, cx| shell.select_category_host(id, window, cx));
        }
    }

    /// Same-kind parents for the Move picker, the child's own parent out.
    fn move_parent_choices(&self) -> Vec<(i64, String)> {
        let Some((node, parent)) = self.selected_node() else {
            return Vec::new();
        };
        if parent.is_none() {
            return Vec::new();
        }
        self.tree
            .iter()
            .filter(|p| p.kind == node.kind && parent.is_none_or(|par| par.id != p.id))
            .map(|p| (p.id, p.name.clone()))
            .collect()
    }

    fn rebuild_move_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Names = self
            .move_parent_choices()
            .into_iter()
            .map(|(_, n)| n.into())
            .collect();
        self.move_target
            .update(cx, |s, cx| s.set_items(names, window, cx));
    }

    /// 3.54: re-set the Delete form's reassign picker when `apply` flagged
    /// a newer grouped-items `Rc` (the Rules page's method: `set_items`
    /// needs a window, so `render` and the delete action pick the change
    /// up; a hidden page does neither until it must).
    fn refresh_cat_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cat_pickers_need_items {
            return;
        }
        self.cat_pickers_need_items = false;
        if let Some(items) = self.cat_choices_held.clone() {
            self.delete_target.update(cx, |s, cx| {
                s.set_items(SearchableVec::new((*items).clone()), window, cx)
            });
            #[cfg(test)]
            self.cat_picker_sets.set(self.cat_picker_sets.get() + 1);
        }
    }

    /// 3.54: the mirror's view of `delete_category` — drop the node (a
    /// parent takes its whole subtree), move the freed transaction counts
    /// to the reassign target and re-point rules that targeted a dead id,
    /// exactly as the DB write does. Runs on the confirming click; a
    /// failed write restores the snapshot the caller kept.
    fn remove_node_optimistically(&mut self, id: i64, reassign_to: i64) {
        let mut dead: Vec<i64> = Vec::new();
        let mut moved: i64 = 0;
        if let Some(ix) = self.tree.iter().position(|p| p.id == id) {
            let parent = self.tree.remove(ix);
            dead.push(parent.id);
            moved += self.counts.get(&parent.id).copied().unwrap_or(0);
            for c in &parent.children {
                dead.push(c.id);
                moved += self.counts.get(&c.id).copied().unwrap_or(0);
            }
        } else {
            for p in &mut self.tree {
                if let Some(cix) = p.children.iter().position(|c| c.id == id) {
                    let c = p.children.remove(cix);
                    dead.push(c.id);
                    moved += self.counts.get(&c.id).copied().unwrap_or(0);
                    break;
                }
            }
        }
        if dead.is_empty() {
            return;
        }
        for d in &dead {
            self.counts.remove(d);
        }
        if !dead.contains(&reassign_to) {
            *self.counts.entry(reassign_to).or_insert(0) += moved;
        }
        for r in &mut self.rules {
            if dead.contains(&r.category_id) {
                r.category_id = reassign_to;
            }
        }
    }

    /// 3.54: the mirror's view of `move_child` — lift the child out of
    /// its current parent and insert it into the new one in name order
    /// (`list_categories` sorts by name). Counts do not move: the same
    /// transactions stay on the same child id.
    fn reparent_optimistically(&mut self, child_id: i64, new_parent_id: i64) {
        if !self.tree.iter().any(|p| p.id == new_parent_id) {
            return;
        }
        let mut child = None;
        for p in &mut self.tree {
            if let Some(cix) = p.children.iter().position(|c| c.id == child_id) {
                child = Some(p.children.remove(cix));
                break;
            }
        }
        let Some(child) = child else { return };
        if let Some(parent) = self.tree.iter_mut().find(|p| p.id == new_parent_id) {
            let ix = parent
                .children
                .iter()
                .position(|c| c.name > child.name)
                .unwrap_or(parent.children.len());
            parent.children.insert(ix, child);
        }
    }

    /// `go_page` (3.23): no transient UI travels across a page switch.
    pub(super) fn close_transient(&mut self, cx: &mut Context<Self>) {
        self.renaming = false;
        self.new_category_open = false;
        self.confirm_delete_category = None;
        self.detail_form = None;
        cx.notify();
    }

    /// 5.3 / 3.81: after a restore replaced the database, the page drops
    /// its share of the transient state — ids it holds name rows the new
    /// file may not have.
    pub(super) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
        self.selected_category = None;
        self.renaming = false;
        self.new_category_open = false;
        self.confirm_delete_category = None;
        self.detail_form = None;
        self.cat_error = None;
        cx.notify();
    }
}
impl CategoriesPage {
    pub(super) fn add_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let name = self.new_parent_name.read(cx).value().trim().to_string();
        let kind = self
            .new_parent_kind
            .read(cx)
            .selected_index(cx)
            .map(|ix| KIND_LABELS[ix.row].1)
            .unwrap_or(CategoryKind::Expense);
        if name.is_empty() {
            self.cat_error = Some("Give the category a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        let submitted = name.clone();
        // 3.84: the Add button shows loading, disabled, until the DB is
        // done.
        self.cat_pending = Some(PendingCatWrite::AddParent);
        // 3.64: the selection this add may claim — once the user selects
        // another row, that click owns the selection from then on.
        let prior_selection = self.selected_category;
        // 3.64: detached — a later action must not cancel this write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_parent(c, &name, kind)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.cat_pending == Some(PendingCatWrite::AddParent) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(id) => {
                        // 3.64: the form still shows what was submitted —
                        // don't wipe a newer draft typed while the write ran.
                        let unchanged = this.new_parent_name.read(cx).value().trim() == submitted;
                        if unchanged {
                            this.new_parent_name
                                .update(cx, |i, cx| i.set_value("", window, cx));
                            // 3.23: adding closes the New category form.
                            this.new_category_open = false;
                        }
                        // 3.64: and the new row takes the selection only
                        // while nothing newer claimed it. Through
                        // `select_category`, so the selection's transient UI
                        // (the rename editor, the delete confirm, the rule
                        // editor) closes with the move instead of staying
                        // open re-attached to the new category.
                        if unchanged && this.selected_category == prior_selection {
                            this.select_category(id, window, cx);
                        }
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// Add a child under the selected parent (3.96: the parent keeps the
    /// selection — the new child never takes it).
    pub(super) fn add_child(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // The action resolves against the tree (3.81): catch the mirror up
        // first, as `apply_if_dirty` documents for paths that can fire
        // while the page is not shown.
        self.apply_if_dirty(cx);
        let Some((parent, None)) = self.selected_node() else {
            return;
        };
        let parent_id = parent.id;
        let name = self.new_child_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.cat_error = Some("Give the subcategory a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        let submitted = name.clone();
        // 3.84: the Add subcategory button shows loading, disabled,
        // until the DB is done.
        self.cat_pending = Some(PendingCatWrite::AddChild);
        // 3.64 / 3.96: the selection the open form belongs to — a success
        // must not disturb any selection made while the write ran.
        let prior_selection = self.selected_category;
        // 3.51: submitted from the Add subcategory form? Then a success
        // keeps the form open and focused, parent still selected, so
        // several subcategories can be added in a row (2026-11-15).
        let from_form = self.detail_form == Some(DetailForm::AddSubcategory);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_child(c, parent_id, &name)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.cat_pending == Some(PendingCatWrite::AddChild) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(_id) => {
                        // 3.96: the new child never takes the selection —
                        // `select_category(id)` used to run here and moved
                        // the detail panel onto the new row, so the user
                        // had to click back to the parent for the next
                        // name (2026-11-15). The parent stays selected, and
                        // the child appears in the tree and in the parent's
                        // rule-target picker with the refresh below.
                        // 3.64: don't wipe a newer draft (see `add_parent`).
                        let unchanged = this.new_child_name.read(cx).value().trim() == submitted;
                        if unchanged {
                            this.new_child_name
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        }
                        if from_form {
                            // 3.51: the form keeps the selection it was
                            // opened on; a newer selection is not stolen
                            // back and the closed form is not reopened.
                            if unchanged && this.selected_category == prior_selection {
                                this.detail_form = Some(DetailForm::AddSubcategory);
                                // Focus stays in the cleared input, once
                                // the frame has rebuilt it (the 3.48 rule
                                // editor's pattern).
                                let handle = this.new_child_name.focus_handle(cx);
                                window.defer(cx, move |window, cx| window.focus(&handle, cx));
                            }
                        }
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        let Some((node, _)) = self.selected_node() else {
            return;
        };
        let name = node.name.clone();
        self.renaming = true;
        // 3.51: the rename editor replaces the title row, so whatever
        // inline form was open closes with it.
        self.detail_form = None;
        self.rename_input
            .update(cx, |i, cx| i.set_value(name, window, cx));
        cx.notify();
    }

    pub(super) fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some(id) = self.selected_category else {
            return;
        };
        let name = self.rename_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.cat_error = Some("Give the category a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.84: the Save button shows loading, disabled, until the DB is
        // done.
        self.cat_pending = Some(PendingCatWrite::Rename);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| rename_category(c, id, &name)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.cat_pending == Some(PendingCatWrite::Rename) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(()) => {
                        // 3.64: the rename still belongs to this row's
                        // session — leave a newer selection's UI alone.
                        if this.selected_category == Some(id) {
                            this.renaming = false;
                        }
                        this.cat_error = None;
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn move_child_to_selected_parent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        let Some((node, Some(_))) = self.selected_node() else {
            return;
        };
        let child_id = node.id;
        let Some(ix) = self.move_target.read(cx).selected_index(cx) else {
            self.cat_error = Some("Choose the category to move it under.".into());
            cx.notify();
            return;
        };
        let Some((new_parent_id, _)) = self.move_parent_choices().get(ix.row).cloned() else {
            return;
        };
        let db = self.db.clone();
        // 3.84: the Move button shows loading, disabled, until the DB is
        // done.
        self.cat_pending = Some(PendingCatWrite::Move);
        // 3.54: the node re-parents in the tree at once; a failed write
        // restores the snapshot below and the refresh reconciles either way.
        let snapshot = self.tree.clone();
        self.reparent_optimistically(child_id, new_parent_id);
        // 3.51: a successful move closes the Move form it was submitted
        // from (a successful action closes its form).
        let from_form = self.detail_form == Some(DetailForm::Move);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_child(c, child_id, new_parent_id)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.cat_pending == Some(PendingCatWrite::Move) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(()) => {
                        if from_form && this.detail_form == Some(DetailForm::Move) {
                            this.detail_form = None;
                        }
                        this.cat_error = None;
                    }
                    Err(e) => {
                        // 3.54: roll the optimistic tree edit back.
                        this.tree = snapshot;
                        this.cat_error = Some(e);
                    }
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// First click arms the delete, the second runs it (ui-conventions,
    /// *Interaction*). 3.59: the reassign target is the Delete picker's
    /// item value — the grouped `SubChoice` carries the sub-category id,
    /// so the shell's positional reassign list is no longer needed here
    /// (the 3.81 forwarder constraint it worked around is moot).
    pub(super) fn arm_delete_category(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        // 3.54: the reassign pick resolves against current items (see the
        // method).
        self.refresh_cat_pickers(window, cx);
        let Some(id) = self.selected_category else {
            return;
        };
        if self.confirm_delete_category != Some(id) {
            self.confirm_delete_category = Some(id);
            cx.notify();
            return;
        }
        // 3.59: the grouped items carry the sub-category id as their
        // value, so the pick resolves directly.
        let Some(reassign_to) = self.delete_target.read(cx).selected_value().copied() else {
            self.cat_error = Some("Choose where its transactions should go.".into());
            cx.notify();
            return;
        };
        self.confirm_delete_category = None;
        // 3.51: the second click ran the delete, so its form goes too.
        self.detail_form = None;
        let db = self.db.clone();
        // 3.84: the Delete button shows loading, disabled, until the DB
        // is done.
        self.cat_pending = Some(PendingCatWrite::Delete);
        // 3.54: the node leaves the tree at once, its counts moving to the
        // reassign target; a failed write restores the snapshot below and
        // the later refresh reconciles either way.
        let snapshot = (self.tree.clone(), self.counts.clone(), self.rules.clone());
        self.remove_node_optimistically(id, reassign_to);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| delete_category(c, id, reassign_to)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.cat_pending == Some(PendingCatWrite::Delete) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(()) => {
                        // 3.64: the deleted row is gone; only drop the
                        // selection if it still points at it — and the
                        // shell's mirror with it (3.81).
                        if this.selected_category == Some(id) {
                            this.selected_category = None;
                            if let Some(shell) = this.shell.upgrade() {
                                shell.update(cx, |shell, _| {
                                    if shell.categories_page.selected_category == Some(id) {
                                        shell.categories_page.selected_category = None;
                                    }
                                });
                            }
                        }
                        this.cat_error = None;
                    }
                    Err(e) => {
                        // 3.54: roll the optimistic tree edit back.
                        let (tree, counts, rules) = snapshot;
                        this.tree = tree;
                        this.counts = counts;
                        this.rules = rules;
                        this.cat_error = Some(e);
                    }
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// 3.91: record one tree paint pass in the counters the tests read.
    /// H4: `DataList::on_pass` hands over the exact range — like the
    /// Queue page's `note_paint_pass`, minus the sequence heuristic that
    /// could not see an exact-page scroll (ids continuing with no break).
    /// Unlike the Queue page's window nothing here drives a prune: the
    /// counters are the tests' own.
    fn note_tree_paint_pass(&self, _range: std::ops::Range<usize>) {
        #[cfg(test)]
        {
            self.tree_rows_built.set(_range.len());
            self.tree_range.set((_range.start, _range.end));
        }
    }
}
/// 3.91: one line of the category tree as the shared list sees it — the
/// fields the cells callback needs, so it never borrows the tree itself.
struct TreeListRow {
    id: i64,
    name: SharedString,
    depth: u8,
    is_system: bool,
    count: i64,
}

impl Render for CategoriesPage {
    /// The Categories page (3.23's layout): header, the new-category form,
    /// then the tree panel and the detail panel side by side. The rule
    /// rows are the shell's renderers, reached through the handle until
    /// 3.80 moves the rule list here.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 3.79: focusing the root when the page is shown waits for the end
        // of the frame so its element exists first (the Rules page's search
        // focus does the same, `rules_focus_pending`).
        if self.focus_pending {
            self.focus_pending = false;
            let focus = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        // 3.54: a new shared category list lands through
        // `refresh_cat_pickers`, from `render` and from the delete action
        // — a hidden page defers the re-set until it next renders or acts.
        self.refresh_cat_pickers(window, cx);
        // 3.81 transitional: the rule rows, the add-rule row and the
        // palette live in `Shell`; without it there is nothing to draw.
        let Some(shell) = self.shell.upgrade() else {
            return div().into_any_element();
        };
        let (muted, border, danger) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.border, theme.danger)
        };
        // 3.59: the Delete picker's menu width: the longest "Category ›
        // Subcategory" label up to 320 px (items fed lazily above, 3.54).
        let delete_menu_w = sub_choices_menu_width(&self.catalog.read(cx).cat_choices());
        // 3.84: the button whose write the DB is running renders loading,
        // disabled, until the answer arrives (the Rules page's 3.40
        // pattern; `cat_pending` is this page's one write slot).
        let add_parent_pending = self.cat_pending == Some(PendingCatWrite::AddParent);
        let add_child_pending = self.cat_pending == Some(PendingCatWrite::AddChild);
        let rename_pending = self.cat_pending == Some(PendingCatWrite::Rename);
        let move_pending = self.cat_pending == Some(PendingCatWrite::Move);
        let delete_pending = self.cat_pending == Some(PendingCatWrite::Delete);
        // 3.91: the tree is the shared list (ui-conventions *Lists*): a
        // 32 px heading over virtualized 40 px rows. Its lines are the
        // tree flattened parents-then-children, one level of `pl` indent
        // per subcategory; clicking a row selects it (3.23), and the
        // swatches use the same palette the transaction views show
        // (ui-conventions, *Category colors*). 3.85: the palette-index
        // map is the Catalog's, built once per categories reload.
        let palette = category_palette(cx.theme());
        let colors = self.catalog.read(cx).cat_colors();
        let mut rows: Vec<TreeListRow> = Vec::new();
        for parent in &self.tree {
            rows.push(TreeListRow {
                id: parent.id,
                name: parent.name.clone().into(),
                depth: 0,
                is_system: parent.is_system,
                count: self.node_count(parent),
            });
            for child in &parent.children {
                rows.push(TreeListRow {
                    id: child.id,
                    name: child.name.clone().into(),
                    depth: 1,
                    is_system: child.is_system,
                    count: self.node_count(child),
                });
            }
        }
        let tree_rows: Rc<Vec<TreeListRow>> = Rc::new(rows);
        let tree_items: Vec<DataItem> = tree_rows
            .iter()
            .enumerate()
            .map(|(ix, row)| DataItem::Row {
                id: ix,
                selected: self.selected_category == Some(row.id),
                selector: format!("cat:tree-row:{}", row.id),
            })
            .collect();
        let click_rows = Rc::clone(&tree_rows);
        // 3.23: the tree lives in its own bordered panel beside the detail
        // panel (ui-conventions, *Pages*); `flex_none` keeps the fixed
        // 300 px column against the shared `panel`'s `flex_1`.
        let tree = panel(cx, false)
            .flex_none()
            .h_full()
            .w(px(300.))
            .debug_selector(|| "cat:tree-panel".into())
            .child(
                DataList::new(
                    cx.entity(),
                    vec![
                        DataCol::new("Category tree", Col::Flex, ColAlign::Left),
                        DataCol::new("Txns", Col::Fixed(48.), ColAlign::Right),
                    ],
                    tree_items,
                )
                .heading_selectors("cat")
                // 3.21: track the page-owned handle, not the fresh one
                // the list creates per render — that one resets the
                // offset to 0 every frame.
                .scroll(self.tree_scroll.clone())
                // H4: the counters track the exact pass range (the tests'
                // own — see [`Self::note_tree_paint_pass`]).
                .on_pass(move |this, range| this.note_tree_paint_pass(range))
                .cells(move |_this, item, _measuring, _window, _cx| {
                    let DataItem::Row { id, .. } = item else {
                        return Vec::new();
                    };
                    let row = &tree_rows[*id];
                    let color = colors
                        .get(&row.id)
                        .map(|ix| palette[ix % 12])
                        .unwrap_or(muted);
                    vec![
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .pl(px(16. * row.depth as f32))
                            .child(swatch(color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .when(row.depth == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                                    .child(row.name.clone()),
                            )
                            .when(row.is_system, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(muted)
                                        .child("system"),
                                )
                            })
                            .into_any_element()
                            .into(),
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(flow::count(row.count))
                            .into_any_element()
                            .into(),
                    ]
                })
                .row_click(move |this, ix, window, cx| {
                    if let Some(row) = click_rows.get(ix) {
                        this.select_category(row.id, window, cx);
                    }
                }),
            );
        // Rules of the selected category (3.23): the list shows only these
        // (`visible_rules`). The list is virtualized like the tree (3.12);
        // rows are fixed height (24 px), except the one row 3.24's editor
        // swapped in for, which reserves `RULE_EDITOR_H`
        // (`RULE_EDITOR_ERROR_H` while a validation error is open under
        // it, one ellipsized line — the two-line 3.48 `rule_editor_row`).
        // 3.80: the rule editor state lives on the Rules page entity, read
        // once per frame.
        let rp = self.rules_page.read(cx);
        let editing = rp.editing_rule;
        let rule_error = rp.rule_error.clone();
        let edit_error = rule_error.is_some();
        let rule_match_count = rp.rule_match_count;
        let rule_pattern = rp.rule_pattern.clone();
        let rule_regex = rp.rule_regex;
        let add_pending = rp.rule_action_pending(PendingRuleAction::Add);
        // 3.53: the parent-scoped add-rule picker is the scaffold's entity
        // (`sync_cat_rule_picker` feeds it); the page only renders it.
        let cat_rule_target = shell.read(cx).categories_page.cat_rule_target.clone();
        let visible = Rc::new(self.visible_rules());
        // 3.49: when the selection is a parent, each rule from one of its
        // subcategories carries that child's name as a tag inside its
        // Pattern column; a subcategory selection paints no tags (the
        // column already reads as that category's rules).
        let subcat_tags: HashMap<i64, SharedString> = self
            .selected_category
            .and_then(|id| self.tree.iter().find(|p| p.id == id))
            .map(|parent| {
                parent
                    .children
                    .iter()
                    .map(|c| (c.id, SharedString::from(c.name.as_str())))
                    .collect()
            })
            .unwrap_or_default();
        let rule_sizes = Rc::new(
            visible
                .iter()
                .map(|r| {
                    if Some(r.id) == editing {
                        // 3.48: the editor's reserved height is shared
                        // with the Rules page (see `RULE_EDITOR_H`).
                        size(
                            px(0.),
                            if edit_error {
                                px(RULE_EDITOR_ERROR_H)
                            } else {
                                px(RULE_EDITOR_H)
                            },
                        )
                    } else {
                        size(px(0.), px(24.))
                    }
                })
                .collect::<Vec<_>>(),
        );

        let preview = match rule_match_count.filter(|_| editing.is_none()) {
            Some(n) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "cat:add-rule-preview".into())
                .child(format!("matches {} transactions", flow::count(n)))
                .into_any_element(),
            None => div().into_any_element(),
        };

        // Detail panel: the selected category, or the prompt to pick one.
        let detail: AnyElement = match self.selected_node() {
            None => v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .min_h_0()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(border)
                .rounded_lg()
                .debug_selector(|| "cat:detail-empty".into())
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Select a category to see its rules and settings."),
                )
                .into_any_element(),
            Some((node, parent)) => {
                let count = self.node_count(node);
                let name = node.name.clone();
                let kind = node.kind;
                let is_system = node.is_system;
                let is_child = parent.is_some();
                // 3.53: the add-rule row's Category control follows the
                // selection: a subcategory names the target itself (no
                // picker); a parent picks among its own children by their
                // own names; a childless parent gets a hint, not a row.
                let childless_parent = !is_child && node.children.is_empty();
                let sub_picker = if is_child || childless_parent {
                    None
                } else {
                    // Trigger and menu fit the longest child name, up to
                    // 200 px / 320 px (see `subcategory_widths`).
                    let longest = node
                        .children
                        .iter()
                        .map(|c| c.name.chars().count())
                        .max()
                        .unwrap_or(0);
                    let (trigger_w, menu_w) = subcategory_widths(longest);
                    Some(
                        v_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .child(field_label(muted, "Subcategory"))
                            .child(subcategory_select(
                                &cat_rule_target,
                                "cat-add",
                                trigger_w,
                                menu_w,
                                cx,
                            ))
                            .into_any_element(),
                    )
                };
                let renaming = self.renaming;
                // 3.51: the Delete form's button reads plain "Delete"
                // until the two-click question arms it (no "…" — the
                // form itself is the confirmation step).
                let confirm_label = if self.confirm_delete_category == Some(node.id) {
                    "Click again to delete"
                } else {
                    "Delete"
                };
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .min_h_0()
                    .gap_3()
                    .p_4()
                    .border_1()
                    .border_color(border)
                    .rounded_lg()
                    .debug_selector(|| "cat:detail-panel".into())
                    // Title line (3.23): the name, its kind and transaction
                    // count, Rename at the right end; the rename editor
                    // (3.16) replaces this line in place.
                    .child(if renaming {
                        h_flex()
                            .gap_2()
                            .items_end()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label(muted, "New name"))
                                    .child(div().w(px(260.)).child(Input::new(&self.rename_input))),
                            )
                            .child(
                                Button::new("save-rename")
                                    .label("Save")
                                    .primary()
                                    .loading(rename_pending)
                                    .disabled(rename_pending)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.save_rename(window, cx)
                                        }),
                                    ),
                            )
                            .child(Button::new("cancel-rename").label("Cancel").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.renaming = false;
                                    cx.notify();
                                }),
                            ))
                            .into_any_element()
                    } else {
                        h_flex()
                            .gap_2()
                            .items_center()
                            .debug_selector(|| "cat:detail-title".into())
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .child(div().flex_shrink_0().text_sm().text_color(muted).child(
                                format!(
                                    "{} · {} transactions",
                                    kind_label(kind),
                                    flow::count(count)
                                ),
                            ))
                            // 3.51: Organize and Delete hide behind these
                            // title-row buttons — small, icon + label, no
                            // "…"; the open form's button reads selected.
                            // Rename keeps its place at the right end.
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .ml_auto()
                                    .flex_shrink_0()
                                    .when(!is_child, |group| {
                                        group.child(
                                            Button::new("title-add-sub")
                                                .small()
                                                .icon(IconName::FolderPlus)
                                                .label(ADD_SUBCATEGORY)
                                                .debug_selector(|| "cat:title-add-sub".into())
                                                .selected(
                                                    self.detail_form
                                                        == Some(DetailForm::AddSubcategory),
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.detail_form =
                                                        Some(DetailForm::AddSubcategory);
                                                    cx.notify();
                                                })),
                                        )
                                    })
                                    .when(is_child, |group| {
                                        group.child(
                                            Button::new("title-move")
                                                .small()
                                                .icon(IconName::FolderInput)
                                                .label("Move")
                                                .disabled(is_system)
                                                .debug_selector(|| "cat:title-move".into())
                                                .selected(self.detail_form == Some(DetailForm::Move))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.detail_form = Some(DetailForm::Move);
                                                    cx.notify();
                                                })),
                                        )
                                    })
                                    .when(!is_system, |group| {
                                        group.child(
                                            Button::new("title-delete")
                                                .small()
                                                .icon(IconName::Trash)
                                                .label("Delete")
                                                .debug_selector(|| "cat:title-delete".into())
                                                .selected(
                                                    self.detail_form == Some(DetailForm::Delete)
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.detail_form = Some(DetailForm::Delete);
                                                    cx.notify();
                                                })),
                                        )
                                    })
                                    // 3.43: Rename carries a `pencil` icon
                                    // and no "…"; the selector names the
                                    // icon so a test can pin both (the
                                    // path is interpolated so the 3.75
                                    // selector guard sees `cat:title-rename`).
                                    .child(
                                        Button::new("rename-category")
                                            .icon(IconName::Pencil)
                                            .label("Rename")
                                            .debug_selector(|| {
                                                format!("cat:title-rename:{}", "icons/pencil.svg")
                                            })
                                            .disabled(is_system)
                                            .flex_shrink_0()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.start_rename(window, cx)
                                            })),
                                    ),
                            )
                            .into_any_element()
                    })
                    .child(div().w_full().border_t_1().border_color(border))
                    // 3.51: Organize and Delete no longer sit open in the
                    // panel — their forms open right under the title row,
                    // one at a time (`detail_form`), opened by the buttons
                    // above. No section titles and no field labels: each
                    // control is named by its own label or placeholder,
                    // and Cancel closes. A successful Add subcategory
                    // keeps the form open and focused (see `add_child`);
                    // a successful Move or Delete closes it.
                    .when_some(self.detail_form, |detail, form| match form {
                        DetailForm::AddSubcategory => detail.child(
                            inline_form(cx)
                                .debug_selector(|| "cat:form-add-sub".into())
                                .child(
                                    div().w(px(220.)).child(
                                        Input::new(&self.new_child_name)
                                            .aria_label("New subcategory name"),
                                    ),
                                )
                                .child(
                                    Button::new("form-add-sub-cancel")
                                        .label("Cancel")
                                        .debug_selector(|| "cat:form-add-sub-cancel".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.detail_form = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("add-child")
                                        .label(ADD_SUBCATEGORY)
                                        .primary()
                                        .loading(add_child_pending)
                                        .disabled(add_child_pending)
                                        .debug_selector(move || {
                                            format!("label:{ADD_SUBCATEGORY}")
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.add_child(window, cx)
                                        })),
                                ),
                        ),
                        DetailForm::Move => detail.child(
                            inline_form(cx)
                                .debug_selector(|| "cat:form-move".into())
                                .child(
                                    div()
                                        .w(px(200.))
                                        .debug_selector(move || {
                                            format!("label:{MOVE_TO_CATEGORY_PH}")
                                        })
                                        .child(
                                            Select::new(&self.move_target)
                                                .menu_width(names_menu_width(
                                                    self.move_parent_choices()
                                                        .iter()
                                                        .map(|(_, n)| n.as_str()),
                                                ))
                                                .placeholder(MOVE_TO_CATEGORY_PH)
                                                .accessibility_label("Move to category"),
                                        ),
                                )
                                .child(
                                    Button::new("form-move-cancel")
                                        .label("Cancel")
                                        .debug_selector(|| "cat:form-move-cancel".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.detail_form = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("move-child")
                                        .label("Move")
                                        .primary()
                                        .loading(move_pending)
                                        .disabled(is_system || move_pending)
                                        .debug_selector(|| "cat:form-move-go".into())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.move_child_to_selected_parent(window, cx)
                                        })),
                                ),
                        ),
                        DetailForm::Delete => detail.child(
                            inline_form(cx)
                                .debug_selector(|| "cat:form-delete".into())
                                .child(
                                    div()
                                        .w(px(260.))
                                        .debug_selector(move || {
                                            format!("label:{MOVE_TXNS_TO_PH}")
                                        })
                                        .child(
                                            Select::new(&self.delete_target)
                                                .menu_width(delete_menu_w)
                                                .placeholder(MOVE_TXNS_TO_PH)
                                                .accessibility_label("Move its transactions to"),
                                        ),
                                )
                                .child(
                                    Button::new("form-delete-cancel")
                                        .label("Cancel")
                                        .debug_selector(|| "cat:form-delete-cancel".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            // Closing the form closes the
                                            // two-click question with it.
                                            this.detail_form = None;
                                            this.confirm_delete_category = None;
                                            cx.notify();
                                        })),
                                )
                                .child({
                                    // 3.51: the armed second click is the
                                    // destructive one — `danger`. 3.43: the
                                    // confirm button carries a `trash` icon;
                                    // armed it still reads "Click again to
                                    // delete" (no "…").
                                    let armed =
                                        self.confirm_delete_category == Some(node.id);
                                    let b = Button::new("delete-category")
                                        .icon(IconName::Trash)
                                        .label(confirm_label)
                                        .loading(delete_pending)
                                        .disabled(delete_pending)
                                        .debug_selector(|| "cat:form-delete-go".into());
                                    let b = if armed { b.danger() } else { b };
                                    b.on_click(cx.listener(move |this, _, window, cx| {
                                        this.arm_delete_category(window, cx)
                                    }))
                                })
                        ),
                    })
                    // Rules for this category (3.23): the list below holds
                    // only these. 3.53: the add-rule row carries no
                    // category picker when the selection names the target
                    // (a subcategory); a parent picks among its own
                    // children instead. 3.51: with Organize and Delete
                    // gone, Rules starts right under the title divider
                    // and takes the panel's remaining height.
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .gap_2()
                            .debug_selector(|| "cat:rules-section".into())
                            .child(region_title("Rules"))
                            // 3.53: a parent with no subcategories can have
                            // no rule at all (rules target a child), so the
                            // hint stands in for the add-rule row.
                            .when(childless_parent, |section| {
                                section.child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .debug_selector(|| "cat:add-rule-need-child".into())
                                        .child("Add a subcategory first"),
                                )
                            })
                            // Add-rule group (3.16): labelled fields; the
                            // pattern column is the flex one, so the row
                            // fits the panel's width. The pattern input, the
                            // regex switch and the pending flag are the
                            // shell's — the same add-rule flow the Rules
                            // page drives, through the 3.81 handle.
                            .when(!childless_parent, |section| {
                                section
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_end()
                                            .debug_selector(|| "cat:add-rule-row".into())
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap_1()
                                                    .child(field_label(muted, "Pattern"))
                                                    .child(
                                                        div().w_full().debug_selector(|| {
                                                            "cat:pattern-input:add".into()
                                                        }).child(Input::new(&rule_pattern)),
                                                    ),
                                            )
                                            .child(
                                                // 3.42: control-height slot, centred.
                                                control_slot(
                                                    "cat:regex-slot:add".into(),
                                                    "cat:switch:add".into(),
                                                    Switch::new("rule-regex")
                                                        .label("regex")
                                                        .checked(rule_regex)
                                                        // 3.80: the add row's controls live on
                                                        // the Rules page; the handler updates
                                                        // the entity directly.
                                                        .on_click({
                                                            let page = self.rules_page.clone();
                                                            move |checked: &bool, _, cx| {
                                                                page.update(cx, |p, cx| {
                                                                    p.rule_regex = *checked;
                                                                    p.preview_rule_match(cx);
                                                                    cx.notify();
                                                                });
                                                            }
                                                        }),
                                                ),
                                            )
                                            // 3.53: a parent selection adds
                                            // the picker over its own children
                                            // (`sub_picker`, built above); a
                                            // subcategory selection adds no
                                            // picker — the row targets it.
                                            .children(sub_picker)
                                            .child(
                                                Button::new("add-rule")
                                                    .label("Add rule")
                                                    .primary()
                                                    .flex_shrink_0()
                                                    // 3.42: the alignment test measures its centre.
                                                    .debug_selector(|| {
                                                        "cat:add-rule-btn:add".into()
                                                    })
                                                    // 3.40: loading + disabled while the add runs.
                                                    .loading(add_pending)
                                                    .disabled(add_pending)
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        if let Some(shell) = this.shell.upgrade() {
                                                            shell.update(cx, |shell, cx| {
                                                                shell.add_rule_to_selected(
                                                                    window, cx,
                                                                );
                                                            });
                                                        }
                                                    })),
                                            ),
                                    )
                                    .child(preview)
                            })
                            .children(
                                rule_error
                                    .clone()
                                    // 3.24: while a row's editor is open it
                                    // shows the error under itself.
                                    .filter(|_| editing.is_none())
                                    .map(|e| div().text_sm().text_color(danger).child(e)),
                            )
                            .when(visible.is_empty(), |section| {
                                section.child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .debug_selector(|| "cat:rules-empty".into())
                                        .child("No rules for this category yet"),
                                )
                            })
                            .when(!visible.is_empty(), |section| {
                                // The headings mirror `rule_row`'s columns
                                // (the queue's pattern, 3.16). Actions sits
                                // on the same line as the other labels,
                                // right-aligned over the buttons: invisible
                                // copies of the row's Edit and Delete give
                                // the column the cluster's exact width, and
                                // their box is clipped to 0 height so the
                                // heading stays a single line.
                                // 3.49: Pattern and Uses are the sort
                                // headings — clicking Uses orders the list
                                // by Uses descending, clicking Pattern
                                // sorts back, like the Rules page's pair.
                                // They stay single-line labels (the 3.23
                                // fit test keeps the whole heading under
                                // one 24 px row, so not the Rules page's
                                // 32 px ghost buttons): the ↓ marks the
                                // active sort and the cursor invites the
                                // click.
                                let heading = h_flex()
                                    .gap_2()
                                    .items_end()
                                    .debug_selector(|| "cat:rule-heading-row".into())
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            div()
                                                .id("cat-rules-sort-pattern")
                                                .cursor_pointer()
                                                .text_xs()
                                                .text_color(muted)
                                                .debug_selector(|| {
                                                    "cat:rule-heading-pattern".into()
                                                })
                                                .child(if self.cat_rules_sort_by_uses {
                                                    "Pattern"
                                                } else {
                                                    "Pattern ↓"
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.cat_rules_sort_by_uses = false;
                                                    cx.notify();
                                                })),
                                        ),
                                    )
                                    .child(
                                        div().w(px(RULE_TYPE_W)).flex_shrink_0().child(
                                            field_label(muted, "Type")
                                                .debug_selector(|| "cat:rule-heading-type".into()),
                                        ),
                                    )
                                    .child(
                                        div().w(px(RULE_USES_W)).flex_shrink_0().child(
                                            div()
                                                .id("cat-rules-sort-uses")
                                                .cursor_pointer()
                                                .text_right()
                                                .text_xs()
                                                .text_color(muted)
                                                .debug_selector(|| "cat:rule-heading-uses".into())
                                                .child(if self.cat_rules_sort_by_uses {
                                                    "Uses ↓"
                                                } else {
                                                    "Uses"
                                                })
                                                .tooltip(|window, cx| {
                                                    Tooltip::element(move |_, _| {
                                                        div()
                                                            .debug_selector(|| {
                                                                "cat:rule-heading-uses-tip".into()
                                                            })
                                                            .child(
                                                                "Transactions this rule categorizes",
                                                            )
                                                    })
                                                    .build(window, cx)
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.cat_rules_sort_by_uses = true;
                                                    cx.notify();
                                                })),
                                        ),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_shrink_0()
                                            .child(
                                                field_label(muted, "Actions")
                                                    .text_right()
                                                    .debug_selector(|| {
                                                        "cat:rule-heading-actions".into()
                                                    }),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .h_0()
                                                    .overflow_hidden()
                                                    .child(
                                                        Button::new("rule-heading-edit-spacer")
                                                            .icon(IconName::Pencil)
                                                            .label("Edit")
                                                            .ghost()
                                                            .text_xs()
                                                            .opacity(0.0),
                                                    )
                                                    .child(
                                                        // 3.43: matches the row's
                                                        // Delete button (trash
                                                        // icon, no "…") so the
                                                        // Actions heading stays
                                                        // over the buttons.
                                                        Button::new("rule-heading-delete-spacer")
                                                            .icon(IconName::Trash)
                                                            .label("Delete")
                                                            .ghost()
                                                            .text_xs()
                                                            .opacity(0.0),
                                                    ),
                                            ),
                                    );
                                // 3.81: the rows are `Shell`'s renderers —
                                // the same `rule_row` / `rule_editor_row`
                                // the Rules page draws — through the
                                // transitional handle, so one rule is one
                                // row on both pages.
                                let rule_shell = shell.clone();
                                section.child(heading).child(
                                    v_virtual_list(
                                        cx.entity(),
                                        "rule-list",
                                        rule_sizes,
                                        move |this, range, _window, cx| {
                                            let rows: Vec<AnyElement> = visible
                                                .iter()
                                                .skip(range.start)
                                                .take(range.len())
                                                .map(|r| {
                                                    // 3.49: the Uses count
                                                    // and the parent-view
                                                    // subcategory tag come
                                                    // from the page's own
                                                    // snapshot mirrors.
                                                    let uses =
                                                        this.uses.get(&r.id).copied().unwrap_or(0);
                                                    let subcat =
                                                        subcat_tags.get(&r.category_id).cloned();
                                                    rule_shell.update(cx, |shell, cx| {
                                                        if shell
                                                            .rules_page
                                                            .read(cx)
                                                            .editing_rule
                                                            == Some(r.id)
                                                        {
                                                            shell.rule_editor_row(
                                                                r, muted, danger, cx,
                                                            )
                                                        } else {
                                                            shell.rule_row(
                                                                r, muted, uses, subcat, cx,
                                                            )
                                                        }
                                                    })
                                                })
                                                .collect();
                                            #[cfg(test)]
                                            this.rule_rows_built.set(rows.len());
                                            #[cfg(test)]
                                            this.rule_range.set((range.start, range.end));
                                            rows
                                        },
                                    )
                                    // 3.21: page-owned handle (see the tree).
                                    .track_scroll(&self.rule_scroll)
                                    .gap_1()
                                    .flex_1()
                                    .min_h_0(),
                                )
                                // 3.53: the editor changes pattern and kind
                                // only — moving a rule to another category
                                // is the Rules page Move bar's job (3.28).
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .debug_selector(|| "cat:rules-move-hint".into())
                                        .child(
                                            "To move rules to another category, use the Rules page.",
                                        ),
                                )
                            }),
                    )
                    .into_any_element()
            }
        };
        // The header count paints `count_text` and its selector mirrors it
        // (3.33): the scan test asserts the exact words and cannot be
        // satisfied by a retired one. 3.23: it counts categories and
        // subcategories apart.
        let n_subcategories: usize = self.tree.iter().map(|p| p.children.len()).sum();
        let count_text = format!(
            "{} categories · {} subcategories",
            self.tree.len(),
            n_subcategories
        );
        let count_sel = count_text.clone();

        v_flex()
            // 3.81: the page's own key context, so Escape is *this* page's
            // Cancel (`ui::init` binds it per context); the root takes
            // focus when the page is shown.
            .track_focus(&self.focus)
            .key_context("Categories")
            .on_action(cx.listener(|this, _: &Cancel, _, cx| {
                // Escape closes the innermost open editor or form (3.81,
                // *Interaction*): the rule editor first (it sits inside
                // the list), then the rename editor, then the detail
                // panel's inline form (3.51 — a two-click delete question
                // disarms with the form), then the new-category form,
                // then the armed category delete, then an armed rule
                // delete on a row; with nothing open Escape means nothing
                // here and propagates (3.82 gave every page its own key
                // context, so there is no Root binding behind it — Escape
                // does nothing while focus sits on Shell).
                if this.rules_page.read(cx).editing_rule.is_some() {
                    this.rules_page.update(cx, |p, cx| p.cancel_edit_rule(cx));
                    return;
                }
                if this.renaming {
                    this.renaming = false;
                    cx.notify();
                    return;
                }
                if this.detail_form.take().is_some() {
                    this.confirm_delete_category = None;
                    cx.notify();
                    return;
                }
                if this.new_category_open {
                    this.new_category_open = false;
                    cx.notify();
                    return;
                }
                if this.confirm_delete_category.take().is_some() {
                    cx.notify();
                    return;
                }
                let disarmed = this.rules_page.update(cx, |p, cx| {
                    if p.armed_delete_rule.take().is_some() {
                        cx.notify();
                        true
                    } else {
                        false
                    }
                });
                if disarmed {
                    return;
                }
                cx.propagate();
            }))
            .flex_1()
            .min_h_0()
            .gap_3()
            // Page header (3.23): title, count summary, and the primary
            // action that opens the new-category form in place.
            .child(
                page_header(
                    cx,
                    "Categories",
                    count_text,
                    vec![
                        Button::new("new-category")
                            .icon(IconName::Plus)
                            .label(NEW_CATEGORY)
                            .debug_selector(move || format!("label:{NEW_CATEGORY}"))
                            .primary()
                            .ml_auto()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.new_category_open = true;
                                cx.notify();
                            }))
                            .into_any_element(),
                    ],
                )
                // The count summary's selector moves onto the whole header
                // row (the Queue page's precedent, 3.91).
                .debug_selector(move || format!("label:{count_sel}")),
            )
            .children(
                self.cat_error
                    .clone()
                    .map(|e| div().text_sm().text_color(danger).child(e)),
            )
            // New-category form (3.23): hidden until the header button
            // opens it; Cancel and a successful add close it.
            .when(self.new_category_open, |page| {
                page.child(
                    inline_form(cx)
                        .debug_selector(|| "cat:new-form".into())
                        .child(
                            v_flex()
                                .gap_1()
                                .child(field_label(muted, "New category name"))
                                .child(div().w(px(220.)).child(Input::new(&self.new_parent_name))),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(field_label(muted, "Category type"))
                                .child(div().w(px(140.)).child(Select::new(&self.new_parent_kind))),
                        )
                        .child(
                            Button::new("cancel-new-category")
                                .label("Cancel")
                                .debug_selector(|| "cat:new-form-cancel".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.new_category_open = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("add-parent")
                                .label(ADD_CATEGORY)
                                .debug_selector(move || format!("label:{ADD_CATEGORY}"))
                                .primary()
                                .loading(add_parent_pending)
                                .disabled(add_parent_pending)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_parent(window, cx)),
                                ),
                        ),
                )
            })
            // Body: the tree panel and the detail panel side by side.
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(tree)
                    .child(detail),
            )
            .into_any_element()
    }
}

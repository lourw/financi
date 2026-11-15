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
const SELECT_CHROME_W: f32 = 40.;
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
/// form in place under the header (ui-conventions, *Pages*).
pub(super) const NEW_CATEGORY: &str = "New category…";
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
        if confirm.is_some_and(|id| !known(id)) {
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
        let names: Names = self
            .child_choices
            .iter()
            .map(|(_, n)| SharedString::from(n.clone()))
            .collect();
        self.rule_target
            .update(cx, |s, cx| s.set_items(names.clone(), window, cx));
        // 3.81: the page owns the delete picker; the shell feeds it.
        self.categories_view.update(cx, |page, cx| {
            page.delete_target
                .update(cx, |s, cx| s.set_items(names.clone(), window, cx));
        });
        // 3.28: the Move bar's picker offers the same child categories.
        self.rules_page
            .move_rules_target
            .update(cx, |s, cx| s.set_items(names, window, cx));
        // 3.28: a refresh that dropped a rule drops its checkbox with it;
        // a deleted category drops its collapsed-group marker.
        self.rules_page
            .selected_rules
            .retain(|id| self.rules.iter().any(|r| r.id == *id));
        self.rules_page
            .collapsed_rule_groups
            .retain(|id| known(*id));
        // 3.24: a refresh that dropped the rule (its category was deleted,
        // say) drops the editor open on it and its armed delete with it,
        // like the `rule_offer` check in `refresh`.
        if let Some(id) = self.editing_rule
            && !self.rules.iter().any(|r| r.id == id)
        {
            self.close_rule_editor();
        }
        if let Some(id) = self.armed_delete_rule
            && !self.rules.iter().any(|r| r.id == id)
        {
            self.armed_delete_rule = None;
        }
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
        self.category_selection_settled = self.refresh_started + 1;
        self.categories_page.selected_category = Some(id);
        // 3.24: selecting another category closes the rule editor (with its
        // preview count) and disarms any armed rule delete — the same
        // "transient UI belongs to what it was opened on" rule as the page
        // switch.
        self.close_rule_editor();
        self.armed_delete_rule = None;
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
        // Read the reassign list before entering the page: the page must
        // not read the shell while this update is in flight (3.81).
        let choices = self.child_choices.clone();
        self.categories_view.update(cx, move |page, cx| {
            page.arm_delete_category(&choices, window, cx)
        });
    }
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
    /// palette live in `Shell` until 3.80 moves the rule list here.
    shell: WeakEntity<Shell>,
    // `pub(super)` like the other extracted pages: the Escape tests focus
    // the page root the way `go_page` does (see `ui/tests/categories.rs`).
    pub(super) focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// The `Catalog` subscription (3.79); lives as long as the page.
    _catalog: Subscription,
    // -- Snapshot mirrors (3.81).
    tree: Vec<CategoryNode>,
    counts: HashMap<i64, i64>,
    rules: Vec<Rule>,
    // -- State (3.74).
    /// The category the detail pane shows; the shell keeps a mirror of
    /// this for the rule list (see the module docs).
    pub(super) selected_category: Option<i64>,
    pub(super) new_parent_name: Entity<InputState>,
    pub(super) new_parent_kind: Entity<SelectState<Names>>,
    pub(super) new_child_name: Entity<InputState>,
    /// 3.23: the New category… form under the page header is open. The
    /// header button opens it, Cancel and a successful add close it, and
    /// `go_page` clears it like the page's other transient UI.
    pub(super) new_category_open: bool,
    pub(super) renaming: bool,
    pub(super) rename_input: Entity<InputState>,
    pub(super) move_target: Entity<SelectState<Names>>,
    pub(super) delete_target: Entity<SelectState<Names>>,
    pub(super) confirm_delete_category: Option<i64>,
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
        let cat = self.catalog.read(cx);
        self.tree = cat.snap.tree.clone();
        self.counts = cat.snap.counts.clone();
        self.rules = cat.snap.rules.clone();
    }
}

impl CategoriesPage {
    pub(super) fn new(
        db: DbWorker,
        writes_blocked: Rc<Cell<bool>>,
        catalog: Entity<Catalog>,
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
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New name"));
        let move_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        let delete_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        Self {
            db,
            writes_blocked,
            catalog,
            shell,
            focus: cx.focus_handle(),
            // `Shell::new` marks the initial page visible right after
            // construction; every page starts hidden.
            visible: false,
            dirty: true,
            focus_pending: false,
            _catalog,
            tree: Vec::new(),
            counts: HashMap::new(),
            rules: Vec::new(),
            selected_category: None,
            new_parent_name,
            new_parent_kind,
            new_child_name,
            new_category_open: false,
            renaming: false,
            rename_input,
            move_target,
            delete_target,
            confirm_delete_category: None,
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
    /// one ever lands there). Nothing without a selection.
    fn visible_rules(&self) -> Vec<Rule> {
        let Some(id) = self.selected_category else {
            return Vec::new();
        };
        let mut ids: HashSet<i64> = [id].into_iter().collect();
        if let Some(parent) = self.tree.iter().find(|p| p.id == id) {
            ids.extend(parent.children.iter().map(|c| c.id));
        }
        self.rules
            .iter()
            .filter(|r| ids.contains(&r.category_id))
            .cloned()
            .collect()
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

    /// `go_page` (3.23): no transient UI travels across a page switch.
    pub(super) fn close_transient(&mut self, cx: &mut Context<Self>) {
        self.renaming = false;
        self.new_category_open = false;
        self.confirm_delete_category = None;
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
        // 3.64: the selection this add may claim — once the user selects
        // another row, that click owns the selection from then on.
        let prior_selection = self.selected_category;
        // 3.64: detached — a later action must not cancel this write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_parent(c, &name, kind)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(id) => {
                        // 3.64: the form still shows what was submitted —
                        // don't wipe a newer draft typed while the write ran.
                        let unchanged = this.new_parent_name.read(cx).value().trim() == submitted;
                        if unchanged {
                            this.new_parent_name
                                .update(cx, |i, cx| i.set_value("", window, cx));
                            // 3.23: adding closes the New category… form.
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

    /// Add a child under the selected parent.
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
        // 3.64: the selection this add may claim (see `add_parent`).
        let prior_selection = self.selected_category;
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_child(c, parent_id, &name)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(id) => {
                        // 3.64: don't wipe a newer draft (see `add_parent`).
                        let unchanged = this.new_child_name.read(cx).value().trim() == submitted;
                        if unchanged {
                            this.new_child_name
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        }
                        // 3.64: nor steal a newer selection, nor leave an
                        // open rename session on it (see `add_parent`).
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

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        let Some((node, _)) = self.selected_node() else {
            return;
        };
        let name = node.name.clone();
        self.renaming = true;
        self.rename_input
            .update(cx, |i, cx| i.set_value(name, window, cx));
        cx.notify();
    }

    fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| rename_category(c, id, &name)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
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
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_child(c, child_id, new_parent_id)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(()) => this.cat_error = None,
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// First click arms the delete, the second runs it (ui-conventions,
    /// *Interaction*). `child_choices` is the shell's reassign list, read
    /// by the caller (render, or the `Shell` test forwarder) so this never
    /// reads the shell through the handle — a forwarder runs inside
    /// `Shell::update`, where that read would panic (3.81).
    pub(super) fn arm_delete_category(
        &mut self,
        child_choices: &[(i64, String)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        let Some(id) = self.selected_category else {
            return;
        };
        if self.confirm_delete_category != Some(id) {
            self.confirm_delete_category = Some(id);
            cx.notify();
            return;
        }
        let Some(ix) = self.delete_target.read(cx).selected_index(cx) else {
            self.cat_error = Some("Choose where its transactions should go.".into());
            cx.notify();
            return;
        };
        let Some((reassign_to, _)) = child_choices.get(ix.row).cloned() else {
            return;
        };
        self.confirm_delete_category = None;
        let db = self.db.clone();
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| delete_category(c, id, reassign_to)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
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
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// One clickable row of the category tree (3.23): a color swatch
    /// (`category_colors`, the same color the transaction views show),
    /// the name — semibold for a top-level category — truncating with an
    /// ellipsis, and the transaction count right-aligned. The selected row
    /// takes `list_active`, an unselected one lights up `list_hover` under
    /// the pointer (gpui-component's `list_item` pattern).
    fn tree_row(
        &self,
        node: &CategoryNode,
        depth: u8,
        selected: bool,
        palette: (Hsla, Hsla, Hsla),
        colors: &HashMap<i64, Hsla>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (muted, list_active, list_hover) = palette;
        let id = node.id;
        let count = self.node_count(node);
        let radius = cx.theme().radius;
        let color = colors.get(&id).copied().unwrap_or(muted);
        div()
            .id(SharedString::from(format!("cat-row-{id}")))
            // Fill the list's width, or the row stays at its natural width:
            // the counts would not line up at the right edge, the selection
            // background would cover only the text, and a long name would
            // never truncate (the queue rows, 3.16).
            .w_full()
            .h_8()
            .px_2()
            .pl(px(8. + 16. * depth as f32))
            .flex()
            .items_center()
            .gap_2()
            .rounded(radius)
            .cursor_pointer()
            .debug_selector(move || format!("cat:tree-row:{id}"))
            .when(selected, |d| d.bg(list_active))
            // A selected row keeps its `list_active` under the pointer.
            .hover(move |s| if selected { s } else { s.bg(list_hover) })
            .child(
                div()
                    .w(px(10.))
                    .h(px(10.))
                    .flex_shrink_0()
                    .rounded(px(2.))
                    .bg(color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .when(depth == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(node.name.clone()),
            )
            .when(node.is_system, |d| {
                d.child(div().text_xs().text_color(muted).child("system"))
            })
            .child(
                div()
                    .w(px(48.))
                    .flex_shrink_0()
                    .text_right()
                    .text_xs()
                    .text_color(muted)
                    .child(flow::count(count)),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select_category(id, window, cx);
            }))
            .into_any_element()
    }
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
        // 3.81 transitional: the rule rows, the add-rule row and the
        // palette live in `Shell`; without it there is nothing to draw.
        let Some(shell) = self.shell.upgrade() else {
            return div().into_any_element();
        };
        let (muted, border, list_active, list_hover, danger) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.border,
                theme.list_active,
                theme.list_hover,
                theme.danger,
            )
        };
        let palette = (muted, list_active, list_hover);
        // 3.23: the tree's swatches use the same palette and assignment the
        // transaction views show (ui-conventions, *Category colors*).
        let colors = Rc::new(shell.read(cx).category_colors(cx.theme()));

        // 3.12: the tree is virtualized (gpui-kit `v_virtual_list`). The
        // closure runs during layout with the visible range, so a tree of
        // thousands builds only the rows that can be on screen. Rows are
        // fixed height (`tree_row`'s `h_8` = 32 px); the panel's `gap_1`
        // is added to the stride by the list.
        let n_tree_rows: usize = self.tree.iter().map(|p| 1 + p.children.len()).sum();
        let tree_sizes = Rc::new(vec![size(px(0.), px(32.)); n_tree_rows]);
        // 3.23: the tree lives in its own bordered panel beside the detail
        // panel (ui-conventions, *Pages*).
        // `h_flex` centers children on the cross axis (unlike `v_flex`), so
        // each column needs `h_full` for the virtual list inside it to
        // resolve a real height; without it the lists collapse to 0 px and
        // the page opens blank (3.18).
        let tree_colors = Rc::clone(&colors);
        let tree = v_flex()
            .w(px(300.))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .p_2()
            .gap_1()
            .border_1()
            .border_color(border)
            .rounded_lg()
            .debug_selector(|| "cat:tree-panel".into())
            .child(region_title("Category tree"))
            .child(
                v_virtual_list(
                    cx.entity(),
                    "category-tree",
                    tree_sizes,
                    move |this, range, _window, cx| {
                        let mut rows = Vec::new();
                        let mut ix = 0usize;
                        for parent in &this.tree {
                            if ix >= range.end {
                                break;
                            }
                            if ix >= range.start {
                                let selected = this.selected_category == Some(parent.id);
                                rows.push(this.tree_row(
                                    parent,
                                    0,
                                    selected,
                                    palette,
                                    &tree_colors,
                                    cx,
                                ));
                            }
                            ix += 1;
                            for child in &parent.children {
                                if ix >= range.end {
                                    break;
                                }
                                if ix >= range.start {
                                    let selected = this.selected_category == Some(child.id);
                                    rows.push(this.tree_row(
                                        child,
                                        1,
                                        selected,
                                        palette,
                                        &tree_colors,
                                        cx,
                                    ));
                                }
                                ix += 1;
                            }
                        }
                        #[cfg(test)]
                        this.tree_rows_built.set(rows.len());
                        #[cfg(test)]
                        this.tree_range.set((range.start, range.end));
                        rows
                    },
                )
                // 3.21: track the page-owned handle, not the fresh one
                // `v_virtual_list` creates per render — that one resets the
                // offset to 0 every frame.
                .track_scroll(&self.tree_scroll)
                .gap_1()
                .flex_1()
                .min_h_0(),
            );

        // Rules of the selected category (3.23): the list shows only these
        // (`visible_rules`). The list is virtualized like the tree (3.12);
        // rows are fixed height (24 px), except the one row 3.24's editor
        // swapped in for, which reserves 68 px (labelled fields) — 92 px
        // while a validation error is open under it, one ellipsized line
        // (`rule_editor_row`). The editor state is the shell's (the same
        // editor the Rules page opens), read once per frame.
        let editing = shell.read(cx).editing_rule;
        let rule_error = shell.read(cx).rule_error.clone();
        let edit_error = rule_error.is_some();
        let rule_match_count = shell.read(cx).rule_match_count;
        let rule_pattern = shell.read(cx).rule_pattern.clone();
        let rule_regex = shell.read(cx).rule_regex;
        let add_pending = shell.read(cx).rule_action_pending(PendingRuleAction::Add);
        // 3.53: the parent-scoped add-rule picker is the scaffold's entity
        // (`sync_cat_rule_picker` feeds it); the page only renders it.
        let cat_rule_target = shell.read(cx).categories_page.cat_rule_target.clone();
        // The reassign list for the Delete section (3.81): the shell's,
        // read once per frame like the other shared bits above.
        let child_choices = shell.read(cx).child_choices.clone();
        let visible = Rc::new(self.visible_rules());
        let rule_sizes = Rc::new(
            visible
                .iter()
                .map(|r| {
                    if Some(r.id) == editing {
                        size(px(0.), if edit_error { px(108.) } else { px(80.) })
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
                let confirm_label = if self.confirm_delete_category == Some(node.id) {
                    "Click again to delete"
                } else {
                    "Delete…"
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
                    // count, Rename… at the right end; the rename editor
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
                            .child(Button::new("save-rename").label("Save").primary().on_click(
                                cx.listener(|this, _, window, cx| this.save_rename(window, cx)),
                            ))
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
                            .child(
                                Button::new("rename-category")
                                    .label("Rename…")
                                    .disabled(is_system)
                                    .ml_auto()
                                    .flex_shrink_0()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_rename(window, cx)
                                    })),
                            )
                            .into_any_element()
                    })
                    .child(div().w_full().border_t_1().border_color(border))
                    // Organize (3.16's labelled groups): a category gains
                    // a subcategory; a subcategory moves under another.
                    .child(
                        v_flex()
                            .gap_1()
                            .child(region_title("Organize"))
                            .when(!is_child, |section| {
                                section.child(
                                    h_flex()
                                        .gap_2()
                                        .items_end()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(muted, "New subcategory name"))
                                                .child(
                                                    div()
                                                        .w(px(220.))
                                                        .child(Input::new(&self.new_child_name)),
                                                ),
                                        )
                                        .child(
                                            Button::new("add-child")
                                                .label(ADD_SUBCATEGORY)
                                                .debug_selector(move || {
                                                    format!("label:{ADD_SUBCATEGORY}")
                                                })
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.add_child(window, cx)
                                                })),
                                        ),
                                )
                            })
                            .when(is_child, |section| {
                                section.child(
                                    h_flex()
                                        .gap_2()
                                        .items_end()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(muted, "Move to category"))
                                                .child(
                                                    div()
                                                        .w(px(200.))
                                                        .debug_selector(move || {
                                                            format!("label:{MOVE_TO_CATEGORY_PH}")
                                                        })
                                                        .child(
                                                            Select::new(&self.move_target)
                                                                .placeholder(MOVE_TO_CATEGORY_PH),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            Button::new("move-child")
                                                .label("Move")
                                                .disabled(is_system)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.move_child_to_selected_parent(window, cx)
                                                })),
                                        ),
                                )
                            }),
                    )
                    // Rules for this category (3.23): the list below holds
                    // only these. 3.53: the add-rule row carries no
                    // category picker when the selection names the target
                    // (a subcategory); a parent picks among its own
                    // children instead.
                    .child(div().w_full().border_t_1().border_color(border))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .gap_2()
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
                                                        .on_click(cx.listener(
                                                            |this, checked: &bool, _, cx| {
                                                                if let Some(shell) =
                                                                    this.shell.upgrade()
                                                                {
                                                                    shell.update(cx, |shell, cx| {
                                                                        shell.rule_regex = *checked;
                                                                        shell.preview_rule_match(cx);
                                                                        cx.notify();
                                                                    });
                                                                }
                                                            },
                                                        )),
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
                                // copies of the row's Edit and Delete… give
                                // the column the cluster's exact width, and
                                // their box is clipped to 0 height so the
                                // heading stays a single line.
                                let heading = h_flex()
                                    .gap_2()
                                    .items_end()
                                    .debug_selector(|| "cat:rule-heading-row".into())
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            field_label(muted, "Pattern")
                                                .debug_selector(|| "cat:rule-heading-pattern".into()),
                                        ),
                                    )
                                    .child(
                                        div().w(px(RULE_TYPE_W)).flex_shrink_0().child(
                                            field_label(muted, "Type")
                                                .debug_selector(|| "cat:rule-heading-type".into()),
                                        ),
                                    )
                                    .child(
                                        div().w(px(RULE_CATEGORY_W)).flex_shrink_0().child(
                                            field_label(muted, "Category")
                                                .debug_selector(|| "cat:rule-heading-category".into()),
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
                                                            .label("Edit")
                                                            .ghost()
                                                            .text_xs()
                                                            .opacity(0.0),
                                                    )
                                                    .child(
                                                        Button::new("rule-heading-delete-spacer")
                                                            .label("Delete…")
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
                                            // `this` is only read for the
                                            // test counters below.
                                            let _ = &this;
                                            let rows: Vec<AnyElement> = visible
                                                .iter()
                                                .skip(range.start)
                                                .take(range.len())
                                                .map(|r| {
                                                    rule_shell.update(cx, |shell, cx| {
                                                        if shell.editing_rule == Some(r.id) {
                                                            shell.rule_editor_row(
                                                                r, muted, danger, cx,
                                                            )
                                                        } else {
                                                            shell.rule_row(r, muted, cx)
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
                    // Delete (3.16's labelled group): the transactions move
                    // first; system categories can't be deleted, so their
                    // detail has no Delete section at all.
                    .when(!is_system, |detail| {
                        let choices = child_choices.clone();
                        detail
                            .child(div().w_full().border_t_1().border_color(border))
                            .child(
                                v_flex().gap_1().child(region_title("Delete")).child(
                                    h_flex()
                                        .gap_2()
                                        .items_end()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(
                                                    muted,
                                                    "Move its transactions to",
                                                ))
                                                .child(
                                                    div()
                                                        .w(px(260.))
                                                        .debug_selector(move || {
                                                            format!("label:{MOVE_TXNS_TO_PH}")
                                                        })
                                                        .child(
                                                            Select::new(&self.delete_target)
                                                                .placeholder(MOVE_TXNS_TO_PH),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            Button::new("delete-category")
                                                .label(confirm_label)
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    this.arm_delete_category(&choices, window, cx)
                                                })),
                                        ),
                                ),
                            )
                    })
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
                // the list), then the rename editor, then the
                // new-category form, then the armed category delete, then
                // an armed rule delete on a row; with nothing open Escape
                // means nothing here and propagates (the dashboard's
                // context-less binding stays behind it).
                if let Some(shell) = this.shell.upgrade()
                    && shell.read(cx).editing_rule.is_some()
                {
                    shell.update(cx, |shell, cx| shell.cancel_edit_rule(cx));
                    return;
                }
                if this.renaming {
                    this.renaming = false;
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
                if let Some(shell) = this.shell.upgrade() {
                    let disarmed = shell.update(cx, |shell, cx| {
                        if shell.armed_delete_rule.take().is_some() {
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    });
                    if disarmed {
                        return;
                    }
                }
                cx.propagate();
            }))
            .flex_1()
            .min_h_0()
            .gap_3()
            // Page header (3.23): title, count summary, and the primary
            // action that opens the new-category form in place.
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Categories"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .debug_selector(move || format!("label:{count_sel}"))
                            .child(count_text),
                    )
                    .child(
                        Button::new("new-category")
                            .label(NEW_CATEGORY)
                            .debug_selector(move || format!("label:{NEW_CATEGORY}"))
                            .primary()
                            .ml_auto()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.new_category_open = true;
                                cx.notify();
                            })),
                    ),
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
                    h_flex()
                        .gap_2()
                        .items_end()
                        .p_2()
                        .border_1()
                        .border_color(border)
                        .rounded_lg()
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
                                .child(field_label(muted, "Kind"))
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

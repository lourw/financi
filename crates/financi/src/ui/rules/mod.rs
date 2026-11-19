//! The Rules page (3.62), a page entity since 3.80.
//!
//! The page owns its search, filters, collapse and sort state, the checked
//! rows, and the add-rule row with its in-place editor (3.24, 3.28). The
//! editor is shared with the Categories page — 3.53 kept one editor, not
//! two — so `rule_row` and `rule_editor_row` are free functions here that
//! render from this entity's state, and `Shell` keeps thin wrapper methods
//! for the Categories rule list. While the Rules page is hidden the
//! Categories page still paints that state, so `Shell` observes this
//! entity: a page notify repaints both.
//!
//! S1 split the file by seam: `editor` (the add/edit handlers and the live
//! preview), `rows` (the shared row, editor row and preview block), `list`
//! (the grouped list, Move bar and matches panel) and `render` (the page
//! view). This module keeps the state, the lifecycle and the `Shell`
//! bridge.

use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;

/// 3.28: the Rules page's header action; it opens the add-rule row in
/// place under the header (ui-conventions, *Pages*). 3.43: no trailing
/// "…" — the button carries a `plus` icon instead.
pub(super) const NEW_RULE: &str = "New rule";
/// 3.28: the Move bar's target picker (moving checked rules between
/// categories).
const MOVE_RULES_TO_PH: &str = "Move rules to…";

/// 3.23: the Categories rule list's fixed columns — the row and its column
/// headings share these widths so the headings sit over their columns
/// (same rule as the queue's `QUEUE_*_W`). Pattern takes the remaining
/// width (`flex_1`) and truncates with a tooltip.
pub(super) const RULE_TYPE_W: f32 = 80.;
pub(super) const RULE_CATEGORY_W: f32 = 200.;
/// 3.53: the parent-scoped add-rule picker's open-menu ceiling — wide
/// enough for a long subcategory name where the trigger has to ellipsize
/// (a 40-character name fits, per the menu test).
pub(super) const RULE_CATEGORY_MENU_W: f32 = 320.;
/// 3.28: the Rules page list's fixed columns beside the flexed Pattern: a
/// checkbox column, and Uses (right-aligned). Same sharing rule as
/// [`RULE_TYPE_W`]: row and column headings use these. 3.49: the
/// Categories page's rule list swapped its Category column for the same
/// Uses column, so the width is shared with it.
const RULE_SEL_W: f32 = 24.;
pub(super) const RULE_USES_W: f32 = 80.;
/// 3.58: the Total column — a signed money sum at the widths the
/// transactions table uses (ui-conventions *Rules*).
pub(super) const RULE_TOTAL_W: f32 = 110.;
/// 3.58: the actions column: two `xsmall` ghost icon buttons (Edit,
/// Delete) with the list's 4 px gap. The width is reserved always, so
/// revealing them on hover shifts nothing.
pub(super) const RULE_ACTIONS_W: f32 = 64.;
/// 3.58: the Move bar inside the panel, replacing the heading row.
pub(super) const RULE_MOVE_BAR_H: f32 = 32.;

/// 3.48: the height the editor row reserves in the virtualized lists —
/// the Rules page's `item_sizes` and the Categories page's `rule_sizes`
/// share these, so the two-line [`inline_form`] editor (labelled Pattern
/// line + 32 px controls line, preview count under it) never paints over
/// the row below it on either page. Measured at 1200 × 720: 148 px with
/// the preview, and 174.5 px with an error line open (the preview stays
/// under a refused save), so the slots reserve 148 / 176.
/// `the_rule_editor_gives_the_pattern_room` and
/// `the_editor_error_stays_inside_its_row` pin them down.
pub(super) const RULE_EDITOR_H: f32 = 148.;
pub(super) const RULE_EDITOR_ERROR_H: f32 = 176.;
/// 3.103: how many preview rows the match preview shows under the add-rule
/// form and the editor (the rest are implied by the summary count).
pub(super) const PREVIEW_ROWS: usize = 8;
/// 3.103: one preview row's height — Date, Merchant and Amount at the
/// list's comfortable line height.
const PREVIEW_ROW_H: f32 = 32.;
/// 3.103: the preview's summary and empty/error lines, matching the
/// editor's one-error-line allowance.
const PREVIEW_LINE_H: f32 = 24.;
/// 3.103: the line when a pattern matches nothing. H5: the line and its
/// `…:preview-empty:<text>` selector both read this const, so a test can
/// pin the exact words the way 3.52 pins `rules:match-tag:won by …`.
const PREVIEW_EMPTY: &str = "No transactions match";

/// 3.28: one entry of the Rules page virtual list — a group header row or a
/// rule row. Group headers are items too, so collapsing a group just drops
/// its rows and the list stays virtualized over everything (D5).
/// 3.85: fingerprint of every input of the grouped list — the applied
/// (debounced) search text, Type picker row, test mode, Unused, the sort
/// column and direction (3.58), the sorted collapsed group ids, and the
/// catalog generation bumped by `apply`.
type ItemsFingerprint = (
    String,
    usize,
    bool,
    bool,
    Option<(&'static str, SortState)>,
    Vec<i64>,
    u64,
);

enum RulesItem {
    Group {
        /// Child-category id; also the collapse key.
        id: i64,
        /// "Parent › Child", from `rule_target_label`.
        label: String,
        /// Rules shown in this group (after the page filters).
        count: usize,
        collapsed: bool,
    },
    Rule {
        id: i64,
    },
}

/// 3.40: the rule-editor button currently waiting on the DB (Save on a
/// rule edit, Add rule, or a row's Delete). While one is pending that
/// button renders `.loading(true)` and disabled, so a slow save can't be
/// double-submitted; the task clears it when it finishes, on success or
/// error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PendingRuleAction {
    Add,
    Save(i64),
    Delete(i64),
}

/// The Rules page (3.74 state, 3.80 entity): the search, filters, checked
/// rows and list state, plus the add-rule row and its in-place editor —
/// the same editor the Categories page renders (see the module docs). The
/// rule list and Uses counts are mirrors of the `Catalog` snapshot;
/// `apply` refreshes them and prunes the page's per-rule state to the
/// rules that still exist.
pub(super) struct RulesPage {
    db: DbWorker,
    /// The shared data model (3.78); `apply` reads the rule list, the Uses
    /// counts and the category tree off its snapshot, never a query of
    /// its own.
    catalog: Entity<Catalog>,
    /// 5.3 review fix: shared with the shell — while an export or restore
    /// is in flight every write here is refused (the row ids may name
    /// different rules after it).
    writes_blocked: Rc<Cell<bool>>,
    focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// The `Catalog` subscription (3.80); lives as long as the page.
    _catalog: Subscription,
    // -- Page state (3.74).
    /// 3.28: the Rules page search box; live filter over the list (pattern
    /// and target category name, case-insensitive), focused when the page
    /// opens (D5). In "Test a description" mode it takes the description.
    pub(super) rules_search: Entity<InputState>,
    /// 3.28: the "Test a description" switch (D5): the list shows only the
    /// rules matching the text in the search box, winner first.
    pub(super) rules_test_mode: bool,
    /// 3.28: Type picker over the list (row 0 = All, 1 = Substring, 2 = Regex;
    /// nothing selected reads as All).
    pub(super) rules_type_select: Entity<SelectState<Names>>,
    /// 3.28: the "Unused" checkbox: only show rules deciding zero
    /// transactions.
    pub(super) rules_unused_only: bool,
    /// 3.28: child-category ids whose group header row is collapsed.
    pub(super) collapsed_rule_groups: HashSet<i64>,
    /// 3.28: sort rows within a group. 3.58: the column and direction
    /// the `DataList` heading drives — `None` is the default, Pattern
    /// ascending; `Some(("Uses", _))` sorts by Uses and `Some(("Total",
    /// _))` by the Total's absolute amount (largest flow first when
    /// descending).
    pub(super) rules_sort: Option<(&'static str, SortState)>,
    /// 3.28: rules whose row checkbox is ticked; the Move bar acts on these.
    pub(super) selected_rules: HashSet<i64>,
    /// 3.28: the Move bar's target picker — the sub-categories grouped
    /// by parent and searchable (3.59); the pick carries the id value.
    pub(super) move_rules_target: Entity<SelectState<SubChoices>>,
    /// 3.28: the Move bar's error line (a move the DB refused).
    pub(super) rules_move_error: Option<SharedString>,
    /// 3.28: whether the add-rule row is open on the Rules page.
    pub(super) new_rule_open: bool,
    /// 3.29: the rule the dashboard's *Show rule* item just landed on; its
    /// row carries the `list_active` background until the user searches or
    /// leaves the page (transient, like the page's other one-shot state).
    pub(super) rules_reveal: Option<i64>,
    /// 3.28: the Rules page list's scroll handle (headers and rows together).
    pub(super) rules_scroll: VirtualListScrollHandle,
    /// 3.52: the rule whose matches show in the panel under the list. A
    /// row click sets it (the row takes the `list_active` background);
    /// clicking the same row again, Escape, selecting another category,
    /// leaving the page or deleting the rule clears it; editing keeps it.
    pub(super) focused_rule: Option<i64>,
    /// 3.52: the focused rule's matches, loaded by the keyed read job
    /// (empty while the load runs or with no focus).
    pub(super) rule_matches: Rc<Vec<RuleMatchRow>>,
    /// 3.52: the in-flight `rule_matches:<id>` job; a result for a rule
    /// the focus has moved on from is dropped when it arrives.
    rule_matches_task: Option<Task<()>>,
    /// 3.52: a load is in flight (the panel shows `…` instead of the
    /// empty message while it is).
    pub(super) rule_matches_loading: bool,
    /// 3.52: the matches panel's own scroll handle (the list and the
    /// matches scroll independently).
    matches_scroll: VirtualListScrollHandle,
    /// 3.58: the search text the list filters by — applied 150 ms after
    /// typing stops (or on Enter), so a burst of keystrokes rebuilds the
    /// list once. The input's own value may run ahead of it.
    rules_query: String,
    /// 3.58: the pending search-debounce timer (see [`RulesPage::rules_query`]).
    rules_search_task: Option<Task<()>>,
    /// 3.85: the grouped, filtered list, cached between renders. Every
    /// input is fingerprinted (the search text and Type picker read
    /// cheaply; the bools and the collapsed set clone cheaply; the
    /// catalog mirrors bump [`RulesPage::rules_gen`] when they reload),
    /// so a frame that changes nothing reuses the same `Rc`.
    rules_items_cache: Option<Rc<Vec<RulesItem>>>,
    rules_items_fp: Option<ItemsFingerprint>,
    /// 3.58: each group's summed Uses and Total (cents), built with the
    /// items and keyed by group (child-category) id — the group headers'
    /// trailing cells read it without re-walking the rules.
    rules_group_sums: Rc<HashMap<i64, (usize, i64)>>,
    /// 3.85: bumped by [`RulesPage::apply`] — the fingerprint component
    /// covering the catalog mirrors (rules, Uses, category labels).
    rules_gen: u64,
    /// 3.47: the id the next provisional rule row carries while its add
    /// round-trip is in flight (negative, so it can never collide with a
    /// real `rules.id`).
    next_temp_rule_id: i64,
    /// 3.54: the shared grouped items last fed to `rule_target` and
    /// `move_rules_target` (3.59; `Rc::ptr_eq` is the change test), and
    /// the flag that a newer list arrived between `apply` and `render` —
    /// the queue page's lazy-picker pattern (3.19/3.83): a hidden page
    /// does not pay for `set_items` until it is shown again.
    cat_choices_held: Option<Rc<Vec<SelectGroup<SubChoice>>>>,
    cat_pickers_need_items: bool,
    /// 3.54 test hook: how many picker `set_items` calls this page made —
    /// proves a hidden page is not re-fed during a refresh.
    #[cfg(test)]
    pub(super) cat_picker_sets: std::cell::Cell<usize>,
    /// 3.28: how many items the Rules page list built last frame (headers
    /// and rows together) and the range it drew, for the long-list tests.
    #[cfg(test)]
    pub(super) rules_page_rows_built: std::cell::Cell<usize>,
    #[cfg(test)]
    pub(super) rules_page_range: std::cell::Cell<(usize, usize)>,
    /// 3.85: how many times the grouped item list was actually built.
    #[cfg(test)]
    pub(super) rules_items_built: std::cell::Cell<usize>,
    // -- `Catalog` snapshot mirrors (3.80).
    /// The shared rule list; the Categories page's list is a view over
    /// the same rows (`Shell` mirrors it too for its other pages).
    pub(super) rules: Vec<Rule>,
    /// 3.28: how many unlocked rows each rule's match decides (the Uses
    /// column); rules deciding nothing are absent.
    pub(super) rule_uses: HashMap<i64, usize>,
    /// 3.58: the signed sum, in cents, of the rows each rule decides
    /// (the Total column); rules deciding nothing are absent.
    pub(super) rule_totals: HashMap<i64, i64>,
    // -- The add-rule row and its in-place editor (3.24, 3.28): rendered
    // by both the Categories page and the Rules page.
    pub(super) rule_pattern: Entity<InputState>,
    pub(super) rule_regex: bool,
    /// 3.59: the add-rule target picker — grouped and searchable like
    /// the Move bar's.
    pub(super) rule_target: Entity<SelectState<SubChoices>>,
    /// The add row's and the editor's shared match preview (the editor owns
    /// it while open; see [`RulesPage::preview_rule_match`]). 3.103: the
    /// rows behind the current draft, not just a count.
    pub(super) pattern_preview: Option<Rc<PatternPreview>>,
    /// The in-flight preview: the debounce timer while keystrokes are still
    /// arriving, then the keyed database read. Clearing it — a keystroke, a
    /// regex toggle, a close — cancels the earlier run.
    pattern_preview_task: Option<Task<()>>,
    /// Why the preview refused to run: a regex pattern that doesn't
    /// compile, worded by the same check `new_rule` applies.
    pub(super) pattern_preview_error: Option<SharedString>,
    /// Keeps the preview list's scroll position across re-renders (3.103).
    preview_scroll: VirtualListScrollHandle,
    pub(super) rule_error: Option<String>,
    /// 3.24: the rule whose row currently shows the in-place editor. One at
    /// a time; `select_category` and `go_page` close it. `start_edit_rule`
    /// loads the rule into the editor's own controls, so the editor is the
    /// add-rule row's shape with the row swapped out.
    pub(super) editing_rule: Option<i64>,
    /// 3.24: the rule whose Delete button is armed ("Click again to
    /// delete"). Armed per row; `select_category` and `go_page` disarm it
    /// with the page's other transient confirmations.
    pub(super) armed_delete_rule: Option<i64>,
    /// 3.40: which rule button is waiting on the DB (see
    /// [`PendingRuleAction`]); rendered loading + disabled while set.
    pub(super) pending_rule_action: Option<PendingRuleAction>,
    /// 3.24: the editor's own controls, separate entities from the add-rule
    /// row's so the two never share an `InputState` on screen at once.
    /// While the editor is open the shared `pattern_preview`
    /// belongs to it (the add-rule row hides it, like `rule_error`)
    /// and excludes that rule (2.5 re-run semantics); every close path
    /// clears it, and [`RulesPage::apply`] closes the editor when its rule
    /// is gone.
    pub(super) rule_edit_pattern: Entity<InputState>,
    pub(super) rule_edit_regex: bool,
    /// The pickers and inputs above subscribe here.
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for RulesPage {}

impl PageEntity for RulesPage {
    /// The list renders the rules and the category labels; a category
    /// rename moves a group header, and the writes that touch either
    /// re-run the rules, so the transactions too.
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
        let (rules, uses, totals, tree, choices) = {
            let cat = self.catalog.read(cx);
            (
                cat.snap.rules.clone(),
                cat.snap.uses.clone(),
                cat.snap.rule_totals.clone(),
                cat.snap.tree.clone(),
                cat.cat_choices(),
            )
        };
        self.rules = rules;
        self.rule_uses = uses;
        self.rule_totals = totals;
        // 3.85: the item cache's catalog inputs just moved.
        self.rules_gen += 1;
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
        // 3.28: a refresh that dropped a rule drops its checkbox with it;
        // a deleted category drops its collapsed-group marker.
        self.selected_rules
            .retain(|id| self.rules.iter().any(|r| r.id == *id));
        self.collapsed_rule_groups.retain(|id| {
            tree.iter()
                .any(|p| p.id == *id || p.children.iter().any(|c| c.id == *id))
        });
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
        // 3.52: a refresh that dropped the focused rule (a delete, say)
        // closes its matches panel; while a rule stays focused, every
        // refresh reloads the matches — a Save, a categorisation, any
        // write that moved the data.
        if let Some(id) = self.focused_rule {
            if self.rules.iter().any(|r| r.id == id) {
                self.load_rule_matches(cx);
            } else {
                self.clear_rule_focus();
            }
        }
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.set_shown(visible);
        if visible {
            self.apply_if_dirty(cx);
            self.set_focus_pending(true);
        } else {
            self.set_dirty(true);
            // 3.11: no transient UI travels across a page switch — the
            // Rules-only state (the add row, the ticked rows, the move
            // error and the *Show rule* highlight) clears on leaving; the
            // shared editor and armed delete close in `go_page`, which
            // runs on every switch.
            self.new_rule_open = false;
            self.selected_rules.clear();
            self.rules_move_error = None;
            self.rules_reveal = None;
            // 3.52: the focused rule and its matches panel are transient
            // too — leaving the page closes them.
            self.clear_rule_focus();
        }
    }
}

impl RulesPage {
    pub(super) fn new(
        db: DbWorker,
        writes_blocked: Rc<Cell<bool>>,
        catalog: &Entity<Catalog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 3.28: the Rules page search box filters the list, and in "Test a
        // description" mode it is the description the rules are tried
        // against (D5). 3.58: filtering is debounced by 150 ms — each
        // keystroke replaces the pending timer, so a burst of typing
        // rebuilds the list once after the last key; Enter applies at once.
        // Typing replaces the list, so the scroll offset goes back to the
        // top when the query is applied.
        let rules_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search rules or categories"));
        let rules_search_sub = cx.subscribe_in(
            &rules_search,
            window,
            |this, input, ev: &InputEvent, window, cx| {
                match ev {
                    InputEvent::Change => {
                        // 3.29: the reveal highlight is one-shot; anything
                        // typed after it is the user's own search.
                        this.rules_reveal = None;
                        let value = input.read(cx).value().to_string();
                        this.rules_search_task =
                            Some(cx.spawn_in(window, async move |this, cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_millis(150))
                                    .await;
                                let _ = this.update_in(cx, move |this, _window, cx| {
                                    this.rules_search_task = None;
                                    this.apply_rules_query(&value, cx);
                                });
                            }));
                    }
                    InputEvent::PressEnter { .. } => {
                        // Enter means "now": cancel the pending debounce.
                        this.rules_search_task = None;
                        let value = input.read(cx).value().to_string();
                        this.apply_rules_query(&value, cx);
                    }
                    _ => {}
                }
            },
        );
        // 3.28: the Type and Move pickers. The Type picker filters the list
        // (nothing selected reads as All types); the Move picker holds the
        // child categories, refreshed by `sync_category_choices`.
        let rules_type_select = cx.new(|cx| {
            SelectState::new(
                vec!["All types".into(), "Substring".into(), "Regex".into()],
                None,
                window,
                cx,
            )
        });
        let rules_type_sub = cx.subscribe_in(
            &rules_type_select,
            window,
            |this, _select, ev: &SelectEvent<Names>, _window, cx| {
                if matches!(ev, SelectEvent::Confirm(_)) {
                    this.rules_scroll.set_offset(Point::default());
                    cx.notify();
                }
            },
        );
        // 3.58: the Move picker is searchable (a long category list);
        // 3.59: its items are the shared grouped sub-categories.
        let move_rules_target = cx
            .new(|cx| SelectState::new(SubChoices::new(vec![]), None, window, cx).searchable(true));
        // The add-rule row and its editor (3.24, 3.28): rendered by both
        // the Categories page and the Rules page.
        let rule_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        let rule_edit_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        // The live "matches N transactions" preview runs on every pattern
        // keystroke (ui-conventions, Categories page).
        let preview_sub = cx.subscribe(&rule_pattern, |this, _, ev: &InputEvent, cx| {
            // 3.24: while a rule editor is open, the preview belongs to it.
            if matches!(ev, InputEvent::Change) && this.editing_rule.is_none() {
                this.preview_rule_match(cx);
            }
        });
        // 3.24: the rule editor's Pattern input drives the same preview and
        // saves on Enter (Escape cancels through the page's `Cancel`).
        let rule_edit_sub = cx.subscribe_in(
            &rule_edit_pattern,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change if this.editing_rule.is_some() => this.preview_rule_match(cx),
                InputEvent::PressEnter {
                    secondary: false, ..
                } if this.editing_rule.is_some() => this.save_rule_edit(window, cx),
                _ => {}
            },
        );
        let _catalog = watch_catalog(catalog, cx);
        Self {
            db,
            catalog: catalog.clone(),
            writes_blocked,
            focus: cx.focus_handle(),
            // `Shell::new` marks the initial page visible right after
            // construction; every page starts hidden.
            visible: false,
            dirty: true,
            focus_pending: false,
            _catalog,
            rules_search,
            rules_test_mode: false,
            rules_type_select,
            rules_unused_only: false,
            collapsed_rule_groups: HashSet::default(),
            rules_sort: None,
            selected_rules: HashSet::default(),
            move_rules_target,
            rules_move_error: None,
            new_rule_open: false,
            rules_reveal: None,
            rules_scroll: VirtualListScrollHandle::new(),
            focused_rule: None,
            rule_matches: Rc::new(Vec::new()),
            rule_matches_task: None,
            rule_matches_loading: false,
            matches_scroll: VirtualListScrollHandle::new(),
            rules_query: String::new(),
            rules_search_task: None,
            rules_items_cache: None,
            rules_items_fp: None,
            rules_group_sums: Rc::new(HashMap::new()),
            rules_gen: 0,
            next_temp_rule_id: -1,
            cat_choices_held: None,
            cat_pickers_need_items: false,
            #[cfg(test)]
            cat_picker_sets: std::cell::Cell::new(0),
            #[cfg(test)]
            rules_page_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            rules_page_range: std::cell::Cell::new((0, 0)),
            #[cfg(test)]
            rules_items_built: std::cell::Cell::new(0),
            rules: Vec::new(),
            rule_uses: HashMap::default(),
            rule_totals: HashMap::default(),
            rule_pattern,
            rule_regex: false,
            rule_target: cx.new(|cx| {
                SelectState::new(SubChoices::new(vec![]), None, window, cx).searchable(true)
            }),
            pattern_preview: None,
            pattern_preview_task: None,
            pattern_preview_error: None,
            preview_scroll: VirtualListScrollHandle::new(),
            rule_error: None,
            editing_rule: None,
            armed_delete_rule: None,
            pending_rule_action: None,
            rule_edit_pattern,
            rule_edit_regex: false,
            _subs: vec![rules_search_sub, rules_type_sub, preview_sub, rule_edit_sub],
        }
    }
}

mod editor;
mod list;
mod render;
mod rows;

// Re-exports keeping the paths the rest of `ui` uses unchanged by the S1
// split: `pattern_preview_block` is public to `ui` as before, and the row
// builders back the `Shell` wrappers at the bottom of this file.
pub(in crate::ui) use self::rows::pattern_preview_block;
use self::rows::{rule_editor_row, rule_row};

impl Shell {
    /// 3.80: thin wrappers over the free row builders — the Categories
    /// page paints the shared rule rows from the Rules page's state (the
    /// `rules` module docs). 3.49: `uses` and `subcat` come from the
    /// Categories page, which mirrors the catalog itself.
    pub(super) fn rule_row(
        &self,
        rule: &Rule,
        muted: Hsla,
        uses: usize,
        subcat: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = &self.rules_page;
        rule_row(rule, muted, uses, subcat, entity.read(cx), entity)
    }

    pub(super) fn rule_editor_row(
        &self,
        rule: &Rule,
        muted: Hsla,
        danger: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = &self.rules_page;
        rule_editor_row(rule, muted, danger, entity.read(cx), entity, cx)
    }

    /// 3.53: Add rule on the Categories page. A selected subcategory is
    /// the target — nothing to pick. A parent needs the row's picker:
    /// rules must target a child (`ensure_assignable`), so no pick is an
    /// error. No selection (nothing selected) can't render the row at all.
    pub(super) fn add_rule_to_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let category_id = match self.selected_node() {
            Some((_, None)) => {
                let Some(ix) = self
                    .categories_page
                    .cat_rule_target
                    .read(cx)
                    .selected_index(cx)
                else {
                    self.rules_page.update(cx, |p, cx| {
                        p.rule_error = Some("Choose the subcategory the rule assigns.".into());
                        cx.notify();
                    });
                    return;
                };
                let Some(cid) = self.categories_page.cat_rule_child_ids.get(ix.row).copied() else {
                    return;
                };
                cid
            }
            Some((node, Some(_))) => node.id,
            None => return,
        };
        self.rules_page.update(cx, |p, cx| {
            p.commit_new_rule(category_id, window, cx);
        });
    }
}

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
    /// The add row's and the editor's shared "matches N transactions"
    /// preview (the editor owns it while open; see
    /// [`RulesPage::preview_rule_match`]).
    pub(super) rule_match_count: Option<i64>,
    rule_task: Option<Task<()>>,
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
    /// While the editor is open the shared `rule_match_count`
    /// preview belongs to it (the add-rule row hides it, like `rule_error`)
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
        let (rules, uses, totals, tree) = {
            let cat = self.catalog.read(cx);
            (
                cat.snap.rules.clone(),
                cat.snap.uses.clone(),
                cat.snap.rule_totals.clone(),
                cat.snap.tree.clone(),
            )
        };
        self.rules = rules;
        self.rule_uses = uses;
        self.rule_totals = totals;
        // 3.85: the item cache's catalog inputs just moved.
        self.rules_gen += 1;
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
            rule_match_count: None,
            rule_task: None,
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

impl RulesPage {
    /// Count rows the current pattern would categorize, live as the user
    /// types (ui-conventions, Categories page).
    pub(super) fn preview_rule_match(&mut self, cx: &mut Context<Self>) {
        // 3.24: while a rule is being edited, the preview reads the
        // editor's controls and excludes that rule, so the count shows what
        // the edit would newly categorize (`count_rule_matches`' contract).
        let editing = self.editing_rule;
        let pattern = match editing {
            Some(_) => self.rule_edit_pattern.read(cx).value().trim().to_string(),
            None => self.rule_pattern.read(cx).value().trim().to_string(),
        };
        if pattern.is_empty() {
            self.rule_task = None;
            self.rule_match_count = None;
            cx.notify();
            return;
        }
        let is_regex = match editing {
            Some(_) => self.rule_edit_regex,
            None => self.rule_regex,
        };
        let db = self.db.clone();
        self.rule_task = Some(cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| {
                count_rule_matches(c, &pattern, is_regex, editing).map_err(|e| e.to_string())
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(count) = result {
                    this.rule_match_count = Some(count);
                    cx.notify();
                }
            });
        }));
    }

    /// 3.40: is `action` the rule button currently waiting on the DB? The
    /// matching button renders `.loading(true)` and disabled while it is,
    /// so a slow save can't be double-submitted.
    pub(super) fn rule_action_pending(&self, action: PendingRuleAction) -> bool {
        self.pending_rule_action == Some(action)
    }

    /// Feedback goes to the shell's toast stack (3.79: a page never owns
    /// window chrome).
    fn toast(&mut self, message: &str, error: bool, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast {
            message: message.to_string(),
            error,
        });
    }

    /// 3.64: show a rule write's failure where it can be read. `rule_error`
    /// is the one line under the add row *and* under the open editor, so it
    /// belongs to whichever of the two is showing: with an editor open the
    /// failure goes out as a toast instead of painting under another rule's
    /// fields (and the add row hides its own line then anyway).
    fn report_rule_error(&mut self, error: String, cx: &mut Context<Self>) {
        match self.editing_rule {
            None => self.rule_error = Some(error),
            Some(_) => self.toast(&error, true, cx),
        }
    }

    pub(super) fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 3.59: the grouped items carry the sub-category id as their
        // value, so the pick resolves directly.
        let Some(category_id) = self.rule_target.read(cx).selected_value().copied() else {
            self.rule_error = Some("Choose the category the rule assigns.".into());
            cx.notify();
            return;
        };
        self.commit_new_rule(category_id, window, cx);
    }

    /// The shared tail of both add-rule paths: validate the pattern, run
    /// [`financi_db::rules::add_rule`], clear the row and refresh (3.40:
    /// the button shows loading, disabled, while the DB is busy — a save
    /// that re-runs the rules can take seconds).
    fn commit_new_rule(&mut self, category_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let pattern = self.rule_pattern.read(cx).value().trim().to_string();
        let is_regex = self.rule_regex;
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.40: the Add button shows loading, disabled, until the DB is
        // done (a save that re-runs the rules can take seconds).
        self.pending_rule_action = Some(PendingRuleAction::Add);
        let submitted = pattern.clone();
        // 3.47: the new row appears in the list at once with a temporary
        // negative id; the Ok arm swaps in the real id, the Err arm drops
        // the row again (the later refresh reconciles either way).
        let temp_id = self.next_temp_rule_id;
        self.next_temp_rule_id -= 1;
        self.rules.push(Rule {
            id: temp_id,
            pattern: pattern.clone(),
            is_regex,
            category_id,
        });
        self.rules_gen += 1;
        // 3.64: detached — a later action must not cancel this write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| add_rule(c, &pattern, is_regex, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // 3.64: only clear the pending marker if it still marks
                // *this* add; a newer action owns the slot now.
                let own = this.pending_rule_action == Some(PendingRuleAction::Add);
                if own {
                    this.pending_rule_action = None;
                }
                match result {
                    Ok(new_id) => {
                        // 3.47: the provisional row carries its real id.
                        if let Some(rule) = this.rules.iter_mut().find(|r| r.id == temp_id) {
                            rule.id = new_id;
                            this.rules_gen += 1;
                        }
                        // The input still shows what was submitted — don't
                        // wipe a pattern typed after this one landed.
                        if this.rule_pattern.read(cx).value().trim() == submitted {
                            this.rule_pattern
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        }
                        // 3.64: this add clears the shared error line only
                        // while it still owns it — a stale add must not wipe
                        // a genuine error another action just reported.
                        if own && this.editing_rule.is_none() {
                            this.rule_error = None;
                        }
                    }
                    // 3.47: a failed add takes the provisional row back
                    // out, whether or not this add still owns the badge.
                    // 3.64: and only this add reports the error: a failure
                    // that lands after the user moved on (another action
                    // owns the badge, or an editor is open) must not paint
                    // under someone else's fields — `report_rule_error`
                    // toasts it while one is open, and a stale add's
                    // failure is the newer action's to report.
                    Err(e) => {
                        if let Some(ix) = this.rules.iter().position(|r| r.id == temp_id) {
                            this.rules.remove(ix);
                            this.rules_gen += 1;
                        }
                        if own {
                            this.report_rule_error(e, cx);
                        }
                    }
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 3.24: close the rule editor and drop everything it owns — the
    /// error line and the live preview count, including its in-flight task
    /// (dropping a `Task` cancels it, so a late result can't repopulate
    /// the count after the editor is gone). Returns whether an editor was
    /// open. Every editor-close path goes through here.
    pub(super) fn close_rule_editor(&mut self) -> bool {
        if self.editing_rule.take().is_some() {
            self.rule_error = None;
            self.rule_task = None;
            self.rule_match_count = None;
            true
        } else {
            false
        }
    }

    /// 3.24: close the rule editor without saving. The editor's controls
    /// keep their values (invisible while closed); `start_edit_rule`
    /// re-fills them from the next rule.
    pub(super) fn cancel_edit_rule(&mut self, cx: &mut Context<Self>) {
        if self.close_rule_editor() {
            cx.notify();
        }
    }

    /// 3.24: swap a rule's row for its editor, pre-filled from the rule —
    /// same shape as the add-rule row (Pattern, regex switch, Category
    /// picker, Save, Cancel). One rule is edited at a time, so opening a
    /// second row's editor closes the first and disarms any pending delete.
    pub(super) fn start_edit_rule(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.rules.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        self.editing_rule = Some(id);
        self.armed_delete_rule = None;
        self.rule_error = None;
        self.rule_edit_regex = rule.is_regex;
        self.rule_edit_pattern.update(cx, |i, cx| {
            i.set_value(rule.pattern.clone(), window, cx);
        });
        // 3.53: no category picker — Edit changes pattern and kind only.
        self.preview_rule_match(cx);
        cx.notify();
        // 3.48: focus lands in the pattern input with the caret at the
        // end (`set_value` places a single-line caret at the end of the
        // text); the focus waits for the end of the frame so the editor's
        // `Input` element exists first, like the page's search focus.
        let handle = self.rule_edit_pattern.focus_handle(cx);
        window.defer(cx, move |window, cx| window.focus(&handle, cx));
    }

    /// 3.24: save the editor through [`update_rule`], which re-runs every
    /// rule over the unlocked rows (2.5). A validation error (empty
    /// pattern, bad regex) lands in `rule_error` — shown in `danger` under
    /// the editor — and the editor stays open. On success every view
    /// refreshes, as after `add_rule`.
    pub(super) fn save_rule_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some(id) = self.editing_rule else {
            return;
        };
        let pattern = self.rule_edit_pattern.read(cx).value().trim().to_string();
        let is_regex = self.rule_edit_regex;
        // 3.53: the editor has no picker, so the rule keeps its category;
        // `update_rule` is still handed the rule's own `category_id`.
        let Some(category_id) = self
            .rules
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.category_id)
        else {
            return;
        };
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.40: the Save button shows loading, disabled, until the DB is
        // done; on error the editor stays open and pending clears too.
        self.pending_rule_action = Some(PendingRuleAction::Save(id));
        // 3.47: the row shows the new pattern at once; the Err arm puts
        // the saved copy back (the refresh reconciles on success).
        let previous = self
            .rules
            .iter()
            .position(|r| r.id == id)
            .map(|ix| self.rules[ix].clone());
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == id) {
            rule.pattern = pattern.clone();
            rule.is_regex = is_regex;
            self.rules_gen += 1;
        }
        // 3.64: detached — see `commit_new_rule`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                update_rule(c, id, &pattern, is_regex, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.pending_rule_action == Some(PendingRuleAction::Save(id)) {
                    this.pending_rule_action = None;
                }
                match result {
                    Ok(()) => {
                        // The editor may have moved to another rule (or
                        // closed) while this save ran: only the editor on
                        // *this* rule gets its result.
                        if this.editing_rule == Some(id) {
                            this.close_rule_editor();
                        }
                    }
                    Err(e) => {
                        // 3.47: roll the optimistic edit back.
                        if let Some(prev) = previous
                            && let Some(rule) = this.rules.iter_mut().find(|r| r.id == id)
                        {
                            *rule = prev;
                            this.rules_gen += 1;
                        }
                        if this.editing_rule == Some(id) {
                            this.rule_error = Some(e);
                        }
                    }
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 3.24: Delete with the two-click confirm (ui-conventions,
    /// *Interaction*): the first click only arms the button ("Click again
    /// to delete"), the second deletes through [`delete_rule`] — whose
    /// re-run leaves the freed rows to the remaining rules or
    /// Uncategorized — and refreshes every view.
    pub(super) fn arm_delete_rule(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        if self.armed_delete_rule == Some(id) {
            let db = self.db.clone();
            // 3.40: the delete runs (and the button shows loading,
            // disabled) while the rules re-run over the unlocked rows.
            self.pending_rule_action = Some(PendingRuleAction::Delete(id));
            // 3.47: the row leaves the list at once; a failed delete puts
            // it back at its position and reports the error (3.24's
            // "rule removed underneath" case).
            let removed = self
                .rules
                .iter()
                .position(|r| r.id == id)
                .map(|ix| (ix, self.rules.remove(ix)));
            if removed.is_some() {
                self.rules_gen += 1;
            }
            // 3.64: detached — see `commit_new_rule`.
            cx.spawn_in(window, async move |this, cx| {
                let result = on_db(&db, move |c| delete_rule(c, id)).await;
                let _ = this.update_in(cx, |this, _window, cx| {
                    if this.pending_rule_action == Some(PendingRuleAction::Delete(id)) {
                        this.pending_rule_action = None;
                    }
                    match result {
                        Ok(()) => {
                            if this.armed_delete_rule == Some(id) {
                                this.armed_delete_rule = None;
                            }
                            if this.editing_rule == Some(id) {
                                this.close_rule_editor();
                            }
                        }
                        // 3.64: the failure of *this* delete never paints
                        // under an editor that belongs to another rule —
                        // `report_rule_error` toasts it while one is open.
                        Err(e) => {
                            // 3.47: restore the row unless a refresh has
                            // already brought it back (it can't be gone
                            // twice — but neither may it be listed twice).
                            if let Some((ix, rule)) = removed
                                && !this.rules.iter().any(|r| r.id == rule.id)
                            {
                                let ix = ix.min(this.rules.len());
                                this.rules.insert(ix, rule);
                                this.rules_gen += 1;
                            }
                            this.report_rule_error(e, cx)
                        }
                    }
                    cx.emit(PageEvent::Changed(Domains::RULES_TX));
                    cx.notify();
                });
            })
            .detach();
        } else {
            self.armed_delete_rule = Some(id);
        }
        cx.notify();
    }
}

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
fn rule_row(
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

/// 3.24: the in-place editor that replaces one rule's row, pre-filled
/// from the rule, with the same live "matches N transactions" count. A
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
/// add-rule rows keep theirs). The preview count and error line stay
/// under both lines; the row reserves [`RULE_EDITOR_H`]
/// ([`RULE_EDITOR_ERROR_H`] with an error open).
///
/// 3.53: no Category picker — the editor changes pattern and kind only,
/// and the rule keeps its category (moving is the Rules page Move bar's
/// job). 3.80: a free function over the page entity, like [`rule_row`].
fn rule_editor_row(
    rule: &Rule,
    muted: Hsla,
    danger: Hsla,
    page: &RulesPage,
    entity: &Entity<RulesPage>,
    cx: &App,
) -> AnyElement {
    let preview = match page.rule_match_count {
        Some(n) => div()
            .text_sm()
            .text_color(muted)
            .debug_selector(|| "rules:edit-preview".into())
            .child(format!("matches {} transactions", flow::count(n)))
            .into_any_element(),
        None => div().into_any_element(),
    };
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
                                                    page.preview_rule_match(cx);
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
    fn apply_rules_query(&mut self, value: &str, cx: &mut Context<Self>) {
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
    fn rules_items_mut(&mut self, cx: &App) -> Rc<Vec<RulesItem>> {
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

    fn toggle_rule_group(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.collapsed_rule_groups.remove(&id) {
            self.collapsed_rule_groups.insert(id);
        }
        cx.notify();
    }

    pub(super) fn set_rule_selected(&mut self, id: i64, on: bool, cx: &mut Context<Self>) {
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
    pub(super) fn move_selected_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
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
            let result = on_db(&db, move |c| move_rules(c, &ids, category_id)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(()) => {
                        // Only the moved rules leave the selection: rules
                        // checked while this move ran keep their checkbox.
                        this.selected_rules.retain(|id| !moved.contains(id));
                        this.rules_move_error = None;
                    }
                    Err(e) => this.rules_move_error = Some(e.into()),
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
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
    pub(super) fn show_rule(&mut self, rule_id: i64, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn clear_rule_focus(&mut self) {
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
    pub(super) fn toggle_rule_focus(&mut self, rule_id: i64, cx: &mut Context<Self>) {
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
    fn load_rule_matches(&mut self, cx: &mut Context<Self>) {
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
    fn matches_panel(&self, cx: &Context<Self>) -> AnyElement {
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
                .cells(move |_this, item, _window, _cx| {
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
        // 3.59: the menu width both category pickers share: the longest
        // "Category › Subcategory" label up to 320 px. The items
        // themselves are fed by `Shell::data_changed` (a window is
        // needed, and hidden pages never render).
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
                        if edit_error {
                            RULE_EDITOR_ERROR_H
                        } else {
                            RULE_EDITOR_H
                        }
                    } else {
                        LIST_ROW_HEIGHT
                    }
                }
                DataItem::Group { .. } => LIST_GROUP_HEIGHT,
            })
            .cells(move |this, item, _window, cx| match item {
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
            .rule_match_count
            .filter(|_| self.editing_rule.is_none())
        {
            Some(n) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "rules:add-preview".into())
                .child(format!("matches {} transactions", flow::count(n)))
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
                                                    this.preview_rule_match(cx);
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

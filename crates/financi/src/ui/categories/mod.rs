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
//!
//! S1 split the file by seam: `pickers` (the add-rule picker's item type
//! and builders, the Select sizing and the kind labels), `shell` (the
//! `Shell`-side category methods), `mutations` (the optimistic writes and
//! the tree row type) and `render` (the page view). This module keeps the
//! shared scaffold, the page state and the entity lifecycle.

use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;
// 3.51: `Button::selected` — the open form's title-row button reads as
// selected. `ui/mod.rs` re-exports the other component traits; this one
// is imported here (the module-level import list belongs to the shell).
use gpui_kit::base::Selectable as _;
/// The state the category flow still shares with the Rules page while
/// the rule list lives there (3.81): the selected-category mirror the
/// add-rule flow reads, the parent-scoped add-rule picker, and the two
/// virtual lists' test counters (the page writes them through the shared
/// `Rc`s it takes in `share_counters`). This scaffold goes away when the
/// Rules page becomes its own entity (3.80).
pub(in crate::ui) struct CategoriesState {
    /// The selected category, mirrored from the page (see the module
    /// docs): `rules.rs` reads it next to the tree's own copy.
    pub(in crate::ui) selected_category: Option<i64>,
    /// 3.53: the Categories page's add-rule picker, shown only when the
    /// selection is a parent: its own child categories by their own names
    /// (no "Parent › Child"), searchable, the wide menu and the
    /// full-name tooltip. `cat_rule_child_ids` maps the picker's rows to
    /// category ids (the items carry names only).
    pub(in crate::ui) cat_rule_target: Entity<SelectState<RuleChoices>>,
    pub(in crate::ui) cat_rule_child_ids: Vec<i64>,
    /// 3.21: how many tree rows the virtualized list built for the last
    /// layout pass — the visible slice, not the whole tree. Observability
    /// for the long-list tests. Shared with the page (3.81) so the rules
    /// tests keep reading the counters here.
    #[cfg(test)]
    pub(in crate::ui) tree_rows_built: Rc<std::cell::Cell<usize>>,
    /// 3.21: the range of rows the tree built last frame, so the scroll
    /// tests can see the window move.
    #[cfg(test)]
    pub(in crate::ui) tree_range: Rc<std::cell::Cell<(usize, usize)>>,
    #[cfg(test)]
    pub(in crate::ui) rule_rows_built: Rc<std::cell::Cell<usize>>,
    /// 3.21: the range of rows the rule list built last frame.
    #[cfg(test)]
    pub(in crate::ui) rule_range: Rc<std::cell::Cell<(usize, usize)>>,
}

impl CategoriesState {
    pub(in crate::ui) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
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
/// Which of the detail panel's inline forms (3.51) is open, one at a
/// time. Opening one closes another; Cancel, Escape, a successful action,
/// a new selection or leaving the page closes the open one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum DetailForm {
    AddSubcategory,
    Move,
    Delete,
}

/// Which category write the DB is still running (3.84), one at a time.
/// The matching button renders `.loading(true)` and disabled while it is,
/// so a slow write can't be double-submitted — the `PendingRuleAction`
/// pattern from 3.40, applied to this page's five writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum PendingCatWrite {
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
pub(in crate::ui) struct CategoriesPage {
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
    pub(in crate::ui) focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// 3.84: the category write the DB is still running, if any. While
    /// one is in flight its button renders loading and disabled, and a
    /// double-submit can't start a second write (`begin_cat_write`).
    pub(in crate::ui) cat_pending: Option<PendingCatWrite>,
    /// 3.54: the shared grouped items last fed to `delete_target`, and
    /// the flag that a newer list arrived between `apply` and `render` —
    /// the queue page's lazy-picker pattern (see `RulesPage`): a hidden
    /// page does not pay for `set_items` until it is shown again.
    cat_choices_held: Option<Rc<Vec<SelectGroup<SubChoice>>>>,
    cat_pickers_need_items: bool,
    /// 3.54 test hook: how many picker `set_items` calls this page made.
    #[cfg(test)]
    pub(in crate::ui) cat_picker_sets: std::cell::Cell<usize>,
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
    pub(in crate::ui) tree: Vec<CategoryNode>,
    pub(in crate::ui) counts: HashMap<i64, i64>,
    pub(in crate::ui) rules: Vec<Rule>,
    pub(in crate::ui) uses: HashMap<i64, usize>,
    // -- State (3.74).
    /// The category the detail pane shows; the shell keeps a mirror of
    /// this for the rule list (see the module docs).
    pub(in crate::ui) selected_category: Option<i64>,
    pub(in crate::ui) new_parent_name: Entity<InputState>,
    pub(in crate::ui) new_parent_kind: Entity<SelectState<Names>>,
    pub(in crate::ui) new_child_name: Entity<InputState>,
    /// 3.23: the New category form under the page header is open. The
    /// header button opens it, Cancel and a successful add close it, and
    /// `go_page` clears it like the page's other transient UI.
    pub(in crate::ui) new_category_open: bool,
    /// 3.49: the rule list sorts by Uses (descending) while this is on;
    /// off is the default pattern order. The Rules page keeps its own copy.
    pub(in crate::ui) cat_rules_sort_by_uses: bool,
    pub(in crate::ui) renaming: bool,
    pub(in crate::ui) rename_input: Entity<InputState>,
    pub(in crate::ui) move_target: Entity<SelectState<Names>>,
    /// 3.59: the Delete form's reassign picker — the sub-categories
    /// grouped by parent and searchable, sharing `Catalog`'s items (fed
    /// by `Shell::data_changed`, where the old flat list was).
    pub(in crate::ui) delete_target: Entity<SelectState<SubChoices>>,
    pub(in crate::ui) confirm_delete_category: Option<i64>,
    /// 3.51: the detail panel's open inline form, one at a time; the
    /// title-row button of the open form shows it as selected.
    pub(in crate::ui) detail_form: Option<DetailForm>,
    pub(in crate::ui) cat_error: Option<String>,
    /// 3.21: one scroll handle per virtualized list, alive as long as the
    /// page (see the task's note on why per-render handles snap back).
    pub(in crate::ui) tree_scroll: VirtualListScrollHandle,
    pub(in crate::ui) rule_scroll: VirtualListScrollHandle,
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
    pub(in crate::ui) fn new(
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
    pub(in crate::ui) fn share_counters(&mut self, scaffold: &CategoriesState) {
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
    pub(in crate::ui) fn close_transient(&mut self, cx: &mut Context<Self>) {
        self.renaming = false;
        self.new_category_open = false;
        self.confirm_delete_category = None;
        self.detail_form = None;
        cx.notify();
    }

    /// 5.3 / 3.81: after a restore replaced the database, the page drops
    /// its share of the transient state — ids it holds name rows the new
    /// file may not have.
    pub(in crate::ui) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
        self.selected_category = None;
        self.renaming = false;
        self.new_category_open = false;
        self.confirm_delete_category = None;
        self.detail_form = None;
        self.cat_error = None;
        cx.notify();
    }
}

mod mutations;
mod pickers;
mod render;
mod shell;

pub(in crate::ui) use self::pickers::*;

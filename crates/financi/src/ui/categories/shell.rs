//! The `Shell` side of the category flow: applying refresh results,
//! syncing the add-rule picker's parent-scoped items, the shared
//! selected-category mirror, and the new-category undo scaffolding the
//! header row drives.

use super::*;
impl Shell {
    /// Apply this page's part of a refresh result (3.74): the shared tree
    /// and counts. `Shell` keeps the mirror the Rules page and the
    /// category-color palette read; the Categories page applies its own
    /// copy from the same snapshot (3.81).
    pub(in crate::ui) fn apply_categories(
        &mut self,
        tree: Vec<CategoryNode>,
        counts: HashMap<i64, i64>,
    ) {
        self.categories = tree;
        self.cat_counts = counts;
    }
}

impl Shell {
    /// Recompute the derived choice lists after a refresh and drop stale
    /// selection state.
    pub(in crate::ui) fn sync_category_choices(
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
    pub(in crate::ui) fn selected_node(&self) -> Option<(&CategoryNode, Option<&CategoryNode>)> {
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
    pub(super) fn select_category_host(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(in crate::ui) fn select_category(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_category_host(id, window, cx);
        self.categories_view
            .update(cx, |page, cx| page.note_selected(id, window, cx));
    }

    #[cfg(test)]
    pub(in crate::ui) fn add_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.categories_view
            .update(cx, |page, cx| page.add_parent(window, cx));
    }

    #[cfg(test)]
    pub(in crate::ui) fn add_child(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.categories_view
            .update(cx, |page, cx| page.add_child(window, cx));
    }

    #[cfg(test)]
    pub(in crate::ui) fn move_child_to_selected_parent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.categories_view.update(cx, |page, cx| {
            page.move_child_to_selected_parent(window, cx)
        });
    }

    #[cfg(test)]
    pub(in crate::ui) fn arm_delete_category(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 3.59: the reassign target resolves from the picker's item value,
        // so the page needs no mirror of the shell's list here.
        self.categories_view
            .update(cx, move |page, cx| page.arm_delete_category(window, cx));
    }
}

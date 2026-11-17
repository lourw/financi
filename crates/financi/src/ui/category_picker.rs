//! 3.59: The shared searchable category pickers.
//!
//! Every `Select` that picks a sub-category — the To categorize row
//! pickers, the Rules page Move bar and add-rule row, the Categories
//! page Delete form — uses these items: one [`SelectGroup`] per parent
//! category, its sub-categories as [`SubChoice`] rows, wrapped in a
//! [`SearchableVec`] so the menu opens with a focused search box
//! (gpui-component's own machinery; nothing hand-rolled). `Catalog`
//! builds the grouped list once per category change and every picker
//! shares a clone of it.
//!
//! Why the *item* matches the query, not just the group title: a
//! `SearchableVec` of groups keeps a group whose title matches even when
//! no row matches — with an empty row list (`vec.rs`, `matched_rows`),
//! so typing "food" would show a bare "Food" header. `SubChoice::matches`
//! checks the parent name too, so every child of Food matches as well
//! and the sub-categories appear under their header, which is what the
//! task asks for ("typing a category name lists its subcategories").

use super::*;
pub(super) use gpui_kit::component::select::SelectGroup;

/// One sub-category row of a category picker. The value is the
/// sub-category's id, so a pick resolves without a positional lookup
/// into any side list; the parent name rides along for the menu row and
/// the search.
#[derive(Clone, Debug)]
pub(super) struct SubChoice {
    pub(super) category: SharedString,
    pub(super) name: SharedString,
    pub(super) id: i64,
}

impl SearchableListItem for SubChoice {
    type Value = i64;

    fn title(&self) -> SharedString {
        format!("{} › {}", self.category, self.name).into()
    }

    fn value(&self) -> &i64 {
        &self.id
    }

    /// Case-insensitive substring match on the sub-category name *or*
    /// the parent's name — "food" matches every child of Food (3.59).
    fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.name.to_lowercase().contains(&q) || self.category.to_lowercase().contains(&q)
    }

    /// The menu row: the muted parent, then the sub-category name, which
    /// ellipsizes when the menu is at its cap (the parent part stays —
    /// same-named sub-categories are told apart by it). The debug
    /// selector carries the full "Category › Name" pair, so two
    /// same-named sub-categories have distinct selectors.
    fn render(&self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let probe = format!("{} › {}", self.category, self.name);
        h_flex()
            .id("sub-choice")
            .debug_selector(move || format!("cat:sub-choice:{probe}"))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} › ", self.category)),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.name.clone()),
            )
    }
}

/// The pickers' delegate: grouped sub-categories, searchable. Each
/// `SelectState` owns its own clone (the filter state lives in it); the
/// shared cache in `Catalog` is the plain `Vec` these are built from.
pub(super) type SubChoices = SearchableVec<SelectGroup<SubChoice>>;

/// 3.59: the picker items for a category tree — one group per parent
/// that has children (an empty group would be a header with no pickable
/// row), sub-categories in tree order like the flat list kept them.
pub(super) fn sub_choice_groups(tree: &[CategoryNode]) -> Vec<SelectGroup<SubChoice>> {
    tree.iter()
        .filter(|p| !p.children.is_empty())
        .map(|p| {
            SelectGroup::new(p.name.clone()).items(p.children.iter().map(|c| SubChoice {
                category: p.name.clone().into(),
                name: c.name.clone().into(),
                id: c.id,
            }))
        })
        .collect()
}

/// 3.59: floor for the grouped pickers' menus — the placeholder
/// ("Category…") must fit whatever the category names are; same spirit
/// as the 3.53 `SUBCATEGORY_MIN_W`.
pub(super) const SUB_PICKER_MENU_MIN_W: f32 = 130.;

/// 3.59: the grouped pickers' menu width — the longest
/// "Category › Subcategory" label at `text_sm` plus the Select chrome,
/// capped at [`RULE_CATEGORY_MENU_W`] (320 px); longer labels ellipsize
/// in the row. Estimated, never measured (like `subcategory_widths`).
pub(super) fn sub_choices_menu_width(groups: &[SelectGroup<SubChoice>]) -> Pixels {
    let longest = groups
        .iter()
        .flat_map(|g| g.items.iter())
        .map(|c| (c.category.chars().count() + c.name.chars().count() + 3) as f32)
        .fold(0., f32::max);
    menu_width_for(longest)
}

/// 3.59: the same width for a flat list of plain names — the Move
/// form's parent-category picker.
pub(super) fn names_menu_width<'a>(names: impl Iterator<Item = &'a str>) -> Pixels {
    let longest = names.map(|n| n.chars().count() as f32).fold(0., f32::max);
    menu_width_for(longest)
}

fn menu_width_for(longest_chars: f32) -> Pixels {
    let est = longest_chars * TEXT_SM_CHAR_PX + SELECT_CHROME_W;
    px(est.clamp(SUB_PICKER_MENU_MIN_W, RULE_CATEGORY_MENU_W))
}

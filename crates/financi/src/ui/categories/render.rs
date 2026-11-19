//! The Categories page's view tree: header, new-category form, tree
//! panel and detail panel with its rule list.

use super::mutations::TreeListRow;
use super::*;
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
                // 3.26: the list selector also names its scrollbar
                // (`cat:tree-list-vscrollbar`) for the overflow tests.
                .selector("cat:tree-list")
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
        // 3.103: the editor row grows by its preview's height.
        let preview_extra = rp.editor_preview_extra_h();
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
                        // with the Rules page (see `RULE_EDITOR_H`);
                        // 3.103 adds the preview block's height.
                        let h = if edit_error {
                            RULE_EDITOR_ERROR_H
                        } else {
                            RULE_EDITOR_H
                        };
                        size(px(0.), px(h + preview_extra))
                    } else {
                        size(px(0.), px(24.))
                    }
                })
                .collect::<Vec<_>>(),
        );

        // 3.103: the add form's match summary, in the form's second line at
        // the right; while a row's editor is open the preview belongs to it
        // (like `rule_error`), so the add form shows nothing.
        let add_summary = rp
            .pattern_preview
            .clone()
            .filter(|_| editing.is_none())
            .map(|preview| {
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_sm()
                    .text_color(muted)
                    .debug_selector(|| "cat:add-rule-preview".into())
                    .child(format!(
                        "Matches {} transactions · {} would move here",
                        flow::count(preview.total),
                        flow::count(preview.decides)
                    ))
            });
        // 3.103: the live match preview under the form — the rows the editor
        // shows too, or "No transactions match", or the bad-regex words in
        // place of the rows. Empty pattern or a first read in flight:
        // nothing.
        let add_preview = if editing.is_none() {
            pattern_preview_block(rp, &self.rules_page, "cat", None, cx)
        } else {
            div().into_any_element()
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
                        div()
                            .flex_shrink_0()
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
                    // count, then the action group at the right end (3.104:
                    // Rename, Add subcategory, Move, Delete per
                    // ui-conventions.md); the rename editor (3.16) replaces
                    // this line in place.
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
                            // 3.104: every button in the group is `small`,
                            // ordered as ui-conventions.md lists it:
                            // Rename, Add subcategory, Move, Delete.
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .ml_auto()
                                    .flex_shrink_0()
                                    // 3.43: Rename carries a `pencil` icon
                                    // and no "…"; the selector names the
                                    // icon so a test can pin both (the
                                    // path is interpolated so the 3.75
                                    // selector guard sees `cat:title-rename`).
                                    .child(
                                        Button::new("rename-category")
                                            .small()
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
                                    )
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
                                    }),
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
                            // Add-rule form (3.103): the 3.48 editor's
                            // shape — a two-line bordered `inline_form`.
                            // Line 1 is the "Pattern" label over the full
                            // width input; line 2 is one `items_center`
                            // row of the controls with no labels: the
                            // regex switch at the left, then the
                            // subcategory picker when the selection is a
                            // parent (3.53's rules unchanged: a
                            // subcategory selection adds no picker — the
                            // form targets it), and the match summary and
                            // Add rule pushed right. `items_center` lines
                            // the controls up on its own — no 3.42
                            // `control_slot`. The live preview sits under
                            // the form. The pattern input, the regex
                            // switch and the pending flag are the Rules
                            // page's — the same add flow the Rules page
                            // drives (3.80/3.81).
                            .when(!childless_parent, |section| {
                                section
                                    .child(
                                        inline_form(cx)
                                            .debug_selector(|| "cat:add-rule-row".into())
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .gap_2()
                                                    // Line 1: the label
                                                    // over the input, at
                                                    // the form's full
                                                    // width.
                                                    .child(
                                                        v_flex()
                                                            .gap_1()
                                                            .child(field_label(muted, "Pattern"))
                                                            .child(
                                                                div().w_full().debug_selector(
                                                                    || {
                                                                        "cat:pattern-input:add"
                                                                            .into()
                                                                    },
                                                                ).child(Input::new(
                                                                    &rule_pattern,
                                                                )),
                                                            ),
                                                    )
                                                    // Line 2: the
                                                    // controls, no
                                                    // labels.
                                                    .child(
                                                        h_flex()
                                                            .w_full()
                                                            .gap_2()
                                                            .items_center()
                                                            .child(
                                                                div()
                                                                    .flex_shrink_0()
                                                                    // 3.42's probe selector:
                                                                    // the alignment test
                                                                    // measures the switch
                                                                    // itself, not the row.
                                                                    .debug_selector(|| {
                                                                        "cat:switch:add".into()
                                                                    })
                                                                    .child(
                                                                        Switch::new("rule-regex")
                                                                            .label("regex")
                                                                            .checked(rule_regex)
                                                                            // 3.103: a toggle
                                                                            // re-runs the
                                                                            // preview at once.
                                                                            .on_click({
                                                                                let page =
                                                                                    self
                                                                                        .rules_page
                                                                                        .clone();
                                                                                move |
                                                                                    checked: &bool,
                                                                                    _,
                                                                                    cx| {
                                                                                    page.update(
                                                                                        cx,
                                                                                        |p, cx| {
                                                                                            p.rule_regex =
                                                                                                *checked;
                                                                                            p.run_pattern_preview(cx);
                                                                                            cx.notify();
                                                                                        },
                                                                                    );
                                                                                }
                                                                            }),
                                                                    ),
                                                            )
                                                            .children(sub_picker)
                                                            .child(
                                                                h_flex()
                                                                    .ml_auto()
                                                                    .min_w_0()
                                                                    .gap_2()
                                                                    .items_center()
                                                                    .children(add_summary)
                                                                    .child(
                                                                        Button::new("add-rule")
                                                                            .label("Add rule")
                                                                            .primary()
                                                                            .flex_shrink_0()
                                                                            // 3.42: the alignment test measures its centre.
                                                                            .debug_selector(|| {
                                                                                "cat:add-rule-btn:add"
                                                                                    .into()
                                                                            })
                                                                            // 3.40: loading + disabled while the add runs.
                                                                            .loading(add_pending)
                                                                            .disabled(add_pending)
                                                                            .on_click(cx.listener(
                                                                                |this,
                                                                                _,
                                                                                window,
                                                                                cx| {
                                                                                    if let Some(
                                                                                        shell,
                                                                                    ) =
                                                                                        this.shell
                                                                                            .upgrade()
                                                                                    {
                                                                                        shell.update(
                                                                                            cx,
                                                                                            |shell,
                                                                                             cx| {
                                                                                                shell.add_rule_to_selected(
                                                                                                    window, cx,
                                                                                                );
                                                                                            },
                                                                                        );
                                                                                    }
                                                                                },
                                                                            )),
                                                                    ),
                                                            ),
                                                    ),
                                            ),
                                    )
                                    .child(add_preview)
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
                                    // 3.26: the same gutter as the rows, so
                                    // the headings stay over their columns.
                                    .pr(px(SCROLLBAR_GUTTER))
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
                                                        // H5: the tip's exact
                                                        // words ride in the
                                                        // selector (3.52's
                                                        // matches-tag pin),
                                                        // built per tooltip like
                                                        // 3.52's own
                                                        // `rules:match-tooltip:<text>`.
                                                        let sel = format!(
                                                            "cat:rule-heading-uses-tip:\
                                                             {RULE_USES_TIP}"
                                                        );
                                                        div()
                                                            .debug_selector(move || sel.clone())
                                                            .child(RULE_USES_TIP)
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
                                    // 3.26: the rule list's scrollbar — a
                                    // relative wrapper with the bar as an
                                    // absolutely positioned sibling over
                                    // the right edge, and a gutter on the
                                    // rows (same shape as `DataList`).
                                    div()
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .flex()
                                        .flex_col()
                                        .child(
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
                                    // 3.26: rows stop short of the bar.
                                    .pr(px(SCROLLBAR_GUTTER))
                                    .flex_1()
                                    .min_h_0(),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .right_0()
                                        .top_0()
                                        .bottom_0()
                                        .w(px(SCROLLBAR_GUTTER))
                                        .id("cat:rule-list-vscrollbar")
                                        .debug_selector(|| "cat:rule-list-vscrollbar".into())
                                        .child(
                                            Scrollbar::vertical(&self.rule_scroll)
                                                .viewport_from_layout(),
                                        ),
                                ),
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

//! Shared view helpers: small widgets, labels, toasts and the width
//! constants pages agree on (split from `ui.rs` by 3.62), plus the 3.89
//! design-system components (`docs/ui-conventions.md` → *Components*).

use super::*;
use financi_db::{DbError, on_db_typed};
use gpui_kit::component::tag::Tag;

/// Every checkbox in the app is built through here (3.14) so they are all
/// constructed alike: the check mark itself arrives with the icon assets
/// `crate::icon_assets` registers on the application, and the debug
/// selector (`checkbox:<id>`) lets headless tests find a box and click it.
/// The selector records itself only in debug builds with `test-support`.
pub(super) fn checkbox(id: impl Into<SharedString>) -> Checkbox {
    let id = id.into();
    let selector = format!("checkbox:{id}");
    Checkbox::new(id).debug_selector(move || selector.clone())
}

/// 3.65: a filter checkbox whose state is Shell state (the filter row and
/// the Options popover). Same widget as [`checkbox`], but the debug
/// selector records the checked state — `checkbox:<id>:checked` when the
/// box is checked — so a headless test can see the box paint *checked* in
/// the one frame right after the click, while the refresh it triggered is
/// still in flight.
pub(super) fn filter_checkbox(id: impl Into<SharedString>, checked: bool) -> Checkbox {
    let id = id.into();
    let selector = if checked {
        format!("checkbox:{id}:checked")
    } else {
        format!("checkbox:{id}")
    };
    Checkbox::new(id)
        .checked(checked)
        .debug_selector(move || selector.clone())
}

/// Field label for a control (3.16, ui-conventions *Numbers, dates and
/// labels*): every input, dropdown and date picker carries a visible label
/// — never a placeholder alone. `text_xs` `muted_foreground` (secondary
/// text), placed above or beside the control. The debug selector
/// (`label:<text>`) lets headless tests assert the label is painted.
pub(super) fn field_label(muted: Hsla, text: &'static str) -> Div {
    div()
        .text_xs()
        .text_color(muted)
        .debug_selector(move || format!("label:{text}"))
        .child(text)
}

/// 3.34: the To-categorize panel's column headings — like `field_label`
/// but `text_sm` (a table heading row, not a form field), keeping the same
/// `label:<text>` debug selector so headless tests still find them.
pub(super) fn heading_label(muted: Hsla, text: &'static str) -> Div {
    div()
        .text_sm()
        .text_color(muted)
        .debug_selector(move || format!("label:{text}"))
        .child(text)
}

/// 3.42 (ui-conventions *Forms*): an unlabelled control in a labelled
/// `items_end` row — the regex switch beside Pattern and Category — goes
/// in a slot the height of one control (32 px), centred, so its centre
/// matches the controls beside it while the row's bottoms stay aligned.
/// The debug selectors record the slot (`<selector>`) and, in a
/// shrink-wrap probe div (the pattern `RuleChoice::render` uses), the
/// control itself (`<probe>`) — the Switch widget carries no selector,
/// so without the probe a test could only ever measure the slot.
pub(super) fn control_slot(selector: String, probe: String, control: impl IntoElement) -> Div {
    div()
        .h(px(32.))
        .flex()
        .items_center()
        .flex_shrink_0()
        .debug_selector(move || selector.clone())
        .child(
            div()
                .flex_shrink_0()
                .debug_selector(move || probe.clone())
                .child(control),
        )
}

/// Region title (ui-conventions *Typography*: `text_sm`, `font_semibold`).
/// Names a group of related controls or a data panel (3.16: the filter bar,
/// the chart, the monthly summary, the category tree). Same debug selector
/// as [`field_label`].
pub(super) fn region_title(text: &'static str) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .debug_selector(move || format!("label:{text}"))
        .child(text)
}

/// Hover tooltip naming a dashboard filter control that has no visible
/// label (3.35, ui-conventions *Interaction*). The tooltip body carries the
/// `filters:tooltip:<name>` selector so headless tests can see it appear.
pub(super) fn filter_tip(
    id: &'static str,
    name: &'static str,
    control: impl IntoElement,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .tooltip(move |window, cx| {
            Tooltip::element(move |_, _| {
                div()
                    .debug_selector(move || format!("filters:tooltip:{name}"))
                    .child(name)
            })
            .build(window, cx)
        })
        .child(control)
}

// -- Design-system components (3.89) -------------------------------------
//
// One shared implementation of every repeated shape (`docs/ui-conventions.md`
// → *Components*): panel, page header, filter strip, list, group header,
// category swatch and the income/expense tag. Page modules call these
// instead of restyling each page by hand; none of them holds page state —
// a plain builder function, or for [`DataList`] a `RenderOnce` element the
// page owns and feeds.

/// Height of a [`DataList`] heading row (ui-conventions *Lists*).
pub(super) const LIST_HEADING_HEIGHT: f32 = 32.;
/// Height of a [`DataList`] collapsible group header.
pub(super) const LIST_GROUP_HEIGHT: f32 = 36.;
/// Height of a [`DataList`] data row.
pub(super) const LIST_ROW_HEIGHT: f32 = 40.;

/// How a [`DataList`] column takes its width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Col {
    /// A fixed width in px.
    Fixed(f32),
    /// Whatever width is left; a list has at most one flex column.
    Flex,
}

/// How a [`DataList`] column aligns its contents — the heading and the
/// rows share the definition, so they always line up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ColAlign {
    /// Text runs from the left edge (labels, dates).
    Left,
    /// Amounts and counts run to the right edge.
    Right,
    /// Toggles and actions centre in their column. (No page uses it
    /// yet; the later milestones that adopt the shared list will.)
    #[allow(dead_code)]
    Center,
}

/// One column of a [`DataList`]: heading text, width and alignment.
#[derive(Clone, Debug)]
pub(super) struct DataCol {
    /// Heading text; also what [`DataList::sorted_by`] names a column by.
    pub title: &'static str,
    /// Width: fixed px, or the one flex column.
    pub width: Col,
    /// Alignment, shared by heading and rows.
    pub align: ColAlign,
}

impl DataCol {
    pub fn new(title: &'static str, width: Col, align: ColAlign) -> Self {
        Self {
            title,
            width,
            align,
        }
    }
}

/// One cell of a painted line (see [`DataList::cells`]): an element, plus
/// the full text a *text* cell carries so the list itself — not the page —
/// attaches the ellipsis tooltip (ui-conventions *Components*: a `Flex`
/// text column "ellipsizes with a tooltip holding the full text").
pub(super) struct ListCell {
    el: AnyElement,
    tooltip: Option<SharedString>,
}

impl ListCell {
    /// A text cell: ellipsized to fit its column, with the list wrapping
    /// it in a tooltip holding the full text. Text in a fixed column can
    /// clip too, so every text cell carries its tooltip, not just the
    /// `Flex` column's.
    pub fn text(text: impl Into<SharedString>) -> Self {
        let text = text.into();
        Self {
            el: div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(text.clone())
                .into_any_element(),
            tooltip: Some(text),
        }
    }
}

impl From<AnyElement> for ListCell {
    /// A non-text cell (a date, an amount, a button): painted as-is.
    fn from(el: AnyElement) -> Self {
        Self { el, tooltip: None }
    }
}

/// Wrap one painted [`ListCell`] in its column's width and alignment, and
/// a text cell additionally in a tooltip holding the full text (through
/// gpui's own tooltip, like [`filter_tip`]). `line` is the line's test
/// selector, which makes the tooltip trigger's element id unique, and `j`
/// the column index.
fn list_cell(col: &DataCol, cell: ListCell, line: &str, j: usize) -> AnyElement {
    let wrap = col_cell(col, div().flex_shrink_0());
    let Some(text) = cell.tooltip else {
        return wrap.child(cell.el).into_any_element();
    };
    let sel = format!("financi:tooltip:{text}");
    wrap.id(SharedString::from(format!("financi-cell-{line}-c{j}")))
        .tooltip(move |window, cx| {
            let text = text.clone();
            let sel = sel.clone();
            Tooltip::element(move |_, _| {
                let text = text.clone();
                let sel = sel.clone();
                div().debug_selector(move || sel).child(text)
            })
            .build(window, cx)
        })
        .child(cell.el)
        .into_any_element()
}

/// Wrap `el` in its column's width and alignment.
fn col_cell(col: &DataCol, el: Div) -> Div {
    let el = match col.width {
        Col::Fixed(w) => el.w(px(w)).flex_shrink_0(),
        Col::Flex => el.flex_1().min_w_0(),
    };
    let el = el.overflow_hidden();
    match col.align {
        ColAlign::Left => el,
        ColAlign::Right => el.items_end().justify_end().text_right(),
        ColAlign::Center => el.items_center().justify_center(),
    }
}

/// A line of a [`DataList`]: a data row, or a group header starting a
/// collapsible group of rows.
#[derive(Debug)]
pub(super) enum DataItem {
    /// A data row.
    Row {
        /// The row's index into the page's own row data.
        id: usize,
        /// Whether the row draws with `list_active`.
        selected: bool,
        /// Test selector for the row element, e.g. `transfers:candidate:7:9`.
        selector: String,
    },
    /// A collapsible group header: the chevron toggles `collapsed`, and
    /// the trailing cells the `cells` callback returns for it line up
    /// with the list's last columns.
    Group {
        /// Stable id the [`DataList::toggle`] callback receives.
        id: i64,
        /// Whether the group is collapsed (its rows hidden).
        collapsed: bool,
        /// Group name, drawn semibold and ellipsized.
        name: SharedString,
        /// Optional muted text after the name, e.g. `3 candidates`.
        count: Option<SharedString>,
        /// Test selector for the header element.
        selector: String,
    },
}

/// Paints one line's cells (see [`DataList::cells`]).
type CellsFn<V> = Rc<dyn Fn(&V, &DataItem, &mut Window, &mut Context<V>) -> Vec<ListCell>>;
/// Paints one line's actions (see [`DataList::actions`]).
type ActionsFn<V> = Rc<dyn Fn(&V, &DataItem, &mut Window, &mut Context<V>) -> Vec<AnyElement>>;
/// A group header's chevron callback (see [`DataList::toggle`]).
type ToggleFn<V> = Rc<dyn Fn(&mut V, i64, &mut Window, &mut Context<V>)>;
/// A heading click's sort callback (see [`DataList::sort`]).
type SortFn<V> = Rc<dyn Fn(&mut V, &'static str, SortState, &mut Window, &mut Context<V>)>;

/// What direction a [`DataList`] heading sorts by (3.89).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SortState {
    /// Ascending.
    Ascending,
    /// Descending.
    Descending,
}

/// The shared list (ui-conventions *Lists*): a 32 px heading row over
/// virtualised 40 px `text_sm` rows with dividers, `list_hover` on hover
/// and a hover-revealed actions column, plus 36 px collapsible group
/// headers on `muted`. Every line pads `px_3`, and text cells built with
/// [`ListCell::text`] carry the spec's full-text tooltip, attached by the
/// list itself so pages never wire tooltips up. The page holds no row
/// styling of its own: it flattens its data into [`DataItem`]s and paints
/// each line's cells through [`DataList::cells`]. Host view `V` is the
/// page's state entity — the same pattern `v_virtual_list` uses.
#[derive(IntoElement)]
pub(super) struct DataList<V: Render> {
    host: Entity<V>,
    columns: Rc<[DataCol]>,
    items: Rc<Vec<DataItem>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    cells: CellsFn<V>,
    actions_width: Option<f32>,
    actions: Option<ActionsFn<V>>,
    on_toggle: Option<ToggleFn<V>>,
    sort: Option<SortFn<V>>,
    sorted_by: Option<(&'static str, SortState)>,
    heading_selectors: Option<SharedString>,
    selector: Option<SharedString>,
    scroll: VirtualListScrollHandle,
}

impl<V: Render> DataList<V> {
    /// A new list over `items` (rows and, between them, group headers)
    /// with the given columns. `host` is the page's own state entity.
    pub fn new(host: Entity<V>, columns: Vec<DataCol>, items: Vec<DataItem>) -> Self {
        let items = Rc::new(items);
        let sizes = Rc::new(
            items
                .iter()
                .map(|item| {
                    let h = match item {
                        DataItem::Row { .. } => LIST_ROW_HEIGHT,
                        DataItem::Group { .. } => LIST_GROUP_HEIGHT,
                    };
                    size(px(0.), px(h))
                })
                .collect(),
        );
        Self {
            host,
            columns: Rc::from(columns),
            items,
            sizes,
            cells: Rc::new(|_, _, _, _| Vec::new()),
            actions_width: None,
            actions: None,
            on_toggle: None,
            sort: None,
            sorted_by: None,
            heading_selectors: None,
            selector: None,
            scroll: VirtualListScrollHandle::new(),
        }
    }

    /// Paint every line through `f`. For a [`DataItem::Row`] return one
    /// cell per column, in column order; for a [`DataItem::Group`] return
    /// the trailing cells only (a sum, say), which line up with the
    /// list's last `n` columns. Each cell is wrapped in its column's
    /// width and alignment; a [`ListCell::text`] cell additionally gets
    /// the tooltip holding its full text.
    pub fn cells(
        mut self,
        f: impl Fn(&V, &DataItem, &mut Window, &mut Context<V>) -> Vec<ListCell> + 'static,
    ) -> Self {
        self.cells = Rc::new(f);
        self
    }

    /// Reserve `width` px at the line's right edge for actions and paint
    /// them through `f` (rows and group headers alike). They are
    /// invisible until the pointer is over the line, and reserving their
    /// space up front means they never shift the columns.
    pub fn actions(
        mut self,
        width: f32,
        f: impl Fn(&V, &DataItem, &mut Window, &mut Context<V>) -> Vec<AnyElement> + 'static,
    ) -> Self {
        self.actions_width = Some(width);
        self.actions = Some(Rc::new(f));
        self
    }

    /// Call `f` with the group id when a group header's chevron is
    /// clicked. The click is bound to the chevron alone, so action
    /// buttons in the header never trigger it.
    pub fn toggle(
        mut self,
        f: impl Fn(&mut V, i64, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }

    /// Make the headings clickable: clicking the heading for column
    /// `title` calls `f` with the direction to sort by next. (No page
    /// sorts through the shared list yet; 3.58 wires the history.)
    #[allow(dead_code)]
    pub fn sort(
        mut self,
        f: impl Fn(&mut V, &'static str, SortState, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.sort = Some(Rc::new(f));
        self
    }

    /// Mark the column the list is sorted by; its heading shows an
    /// arrow and a repeat click flips the direction.
    #[allow(dead_code)]
    pub fn sorted_by(mut self, title: &'static str, state: SortState) -> Self {
        self.sorted_by = Some((title, state));
        self
    }

    /// Prefix the heading cells' test selectors with `prefix`, giving
    /// `{prefix}:heading:{column}` per column (e.g.
    /// `transfers:heading:Date`).
    pub fn heading_selectors(mut self, prefix: &str) -> Self {
        self.heading_selectors = Some(SharedString::from(prefix));
        self
    }

    /// Test selector for the list element itself.
    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    /// Keep the scroll position across re-renders with the page's own
    /// handle (one [`VirtualListScrollHandle`] per list, in page state).
    pub fn scroll(mut self, handle: VirtualListScrollHandle) -> Self {
        self.scroll = handle;
        self
    }

    fn heading_cell(&self, col: &DataCol) -> AnyElement {
        let sorted = self.sorted_by.filter(|(t, _)| *t == col.title);
        let inner = h_flex()
            .gap_1()
            .items_center()
            .child(div().child(col.title));
        let inner = match (&self.sort, sorted) {
            (None, _) => inner.into_any_element(),
            (Some(_), state) => {
                let next = match state {
                    Some((_, SortState::Ascending)) => SortState::Descending,
                    _ => SortState::Ascending,
                };
                let f = self.sort.clone().unwrap();
                let host = self.host.clone();
                let title = col.title;
                inner
                    .id(SharedString::from(format!(
                        "financi-heading-{}",
                        col.title.replace(' ', "-")
                    )))
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .on_click(move |_, window, cx| {
                        host.update(cx, |v, cx| f(v, title, next, window, cx));
                    })
                    .child(
                        div().flex_none().child(
                            Icon::new(match state {
                                Some((_, SortState::Descending)) => IconName::SortDescending,
                                _ => IconName::SortAscending,
                            })
                            .xsmall(),
                        ),
                    )
                    .into_any_element()
            }
        };
        let cell = col_cell(col, div()).child(inner);
        match &self.heading_selectors {
            Some(prefix) => {
                let sel = format!("{prefix}:heading:{}", col.title);
                cell.debug_selector(move || sel.clone()).into_any_element()
            }
            None => cell.into_any_element(),
        }
    }
}

impl<V: Render> RenderOnce for DataList<V> {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (border, muted_bg, muted_fg, list_hover, list_active) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.muted,
                theme.muted_foreground,
                theme.list_hover,
                theme.list_active,
            )
        };
        let heading = h_flex()
            .w_full()
            .h(px(LIST_HEADING_HEIGHT))
            .px_3()
            .gap_4()
            .items_end()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(muted_fg)
            .border_b_1()
            .border_color(border)
            .children(self.columns.iter().map(|col| self.heading_cell(col)))
            .when_some(self.actions_width, |el, w| {
                el.child(div().w(px(w)).flex_shrink_0())
            });
        let heading = match &self.heading_selectors {
            Some(prefix) => {
                let sel = format!("{prefix}:heading");
                heading.debug_selector(move || sel.clone())
            }
            None => heading,
        };

        let columns = self.columns.clone();
        let items = self.items.clone();
        let cells = self.cells.clone();
        let actions = self.actions.clone();
        let actions_width = self.actions_width;
        let on_toggle = self.on_toggle.clone();
        let total = items.len();
        let list = v_virtual_list(
            self.host.clone(),
            "financi-data-list",
            self.sizes.clone(),
            move |view, range, window, cx| {
                let start = range.start;
                let mut lines = Vec::with_capacity(range.len());
                for (i, item) in items[range].iter().enumerate() {
                    let is_last = start + i + 1 == total;
                    let painted = cells(view, item, window, cx);
                    let line = match item {
                        DataItem::Row {
                            selected, selector, ..
                        } => {
                            let group = format!("financi-list-row-{selector}");
                            let selected = *selected;
                            let mut row = div()
                                .w_full()
                                .h(px(LIST_ROW_HEIGHT))
                                .px_3()
                                .gap_4()
                                .flex()
                                .items_center()
                                .text_sm()
                                .when(!is_last, |r| r.border_b_1().border_color(border))
                                .hover(move |s| if selected { s } else { s.bg(list_hover) })
                                .when(selected, |s| s.bg(list_active))
                                .group(group.as_str())
                                .debug_selector({
                                    let s = selector.clone();
                                    move || s.clone()
                                })
                                .children(painted.into_iter().enumerate().map(|(j, cell)| {
                                    list_cell(&columns[j.min(columns.len() - 1)], cell, selector, j)
                                }));
                            if let (Some(f), Some(w)) = (&actions, actions_width) {
                                let painted = f(view, item, window, cx);
                                row = row.child(
                                    div()
                                        .w(px(w))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_end()
                                        .gap_1()
                                        .invisible()
                                        .group_hover(group.as_str(), |s| s.visible())
                                        .children(painted),
                                );
                            }
                            row.into_any_element()
                        }
                        DataItem::Group {
                            id,
                            collapsed,
                            name,
                            count,
                            selector,
                        } => {
                            let group = format!("financi-list-group-{selector}");
                            let mut head = div()
                                .w_full()
                                .h(px(LIST_GROUP_HEIGHT))
                                .px_3()
                                .gap_4()
                                .flex()
                                .items_center()
                                .text_sm()
                                .bg(muted_bg)
                                .when(!is_last, |r| r.border_b_1().border_color(border))
                                .group(group.as_str())
                                .debug_selector({
                                    let s = selector.clone();
                                    move || s.clone()
                                });
                            let toggle_sel = format!("{selector}:toggle");
                            let chevron = div()
                                .id(SharedString::from(format!("financi-group-toggle-{id}")))
                                .flex_none()
                                .cursor_pointer()
                                .debug_selector(move || toggle_sel.clone())
                                .child(Icon::new(if *collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                }));
                            head = head.child(match &on_toggle {
                                None => chevron.into_any_element(),
                                Some(f) => {
                                    let f = f.clone();
                                    let id = *id;
                                    chevron
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            f(this, id, window, cx)
                                        }))
                                        .into_any_element()
                                }
                            });
                            head = head
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(name.clone()),
                                )
                                .when_some(count.clone(), |el, c| {
                                    el.child(div().flex_none().text_color(muted_fg).child(c))
                                });
                            // Trailing cells reuse the last columns'
                            // wrappers, so a group's sums line up with
                            // the rows below by construction.
                            let n = columns.len();
                            let k = painted.len().min(n);
                            if k < n {
                                head = head.child(div().flex_1().min_w_0());
                            }
                            for (j, cell) in painted.into_iter().enumerate() {
                                head =
                                    head.child(list_cell(&columns[n - k + j], cell, selector, j));
                            }
                            if let (Some(f), Some(w)) = (&actions, actions_width) {
                                let painted = f(view, item, window, cx);
                                head = head.child(
                                    div()
                                        .w(px(w))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_end()
                                        .gap_1()
                                        .invisible()
                                        .group_hover(group.as_str(), |s| s.visible())
                                        .children(painted),
                                );
                            }
                            head.into_any_element()
                        }
                    };
                    lines.push(line);
                }
                lines
            },
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        let root = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(heading)
            .child(list);
        match self.selector {
            Some(s) => root
                .debug_selector(move || s.to_string())
                .into_any_element(),
            None => root.into_any_element(),
        }
    }
}

/// A panel (`docs/ui-conventions.md` → *Panels*): bordered, rounded,
/// never filled, filling the space the page gives it. `padded` adds the
/// standard 16 px of inner padding (`p_4`).
pub(super) fn panel(cx: &App, padded: bool) -> Div {
    let (border, radius) = {
        let theme = cx.theme();
        (theme.border, theme.radius)
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .rounded(radius)
        .border_1()
        .border_color(border)
        .overflow_hidden()
        .when(padded, |el| el.p_4())
}

/// The page header (ui-conventions *Pages*): `text_lg` semibold title,
/// a `text_sm muted` summary beside it, and the page's actions pushed to
/// the right edge.
pub(super) fn page_header(
    cx: &App,
    title: impl Into<SharedString>,
    summary: impl Into<SharedString>,
    actions: Vec<AnyElement>,
) -> Div {
    let muted = cx.theme().muted_foreground;
    let title: SharedString = title.into();
    let summary: SharedString = summary.into();
    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().text_sm().text_color(muted).child(summary))
        .child(div().flex_1())
        .children(actions)
}

/// The filter strip (ui-conventions *Filtering*): one 32 px row of
/// controls with the standard 8 px gap. (Built ahead of the pages that
/// move onto it; 3.51 and 3.52 adopt it.)
#[allow(dead_code)]
pub(super) fn filter_row() -> Div {
    h_flex().w_full().h(px(32.)).gap_2().items_center()
}

/// An inline add/edit form (ui-conventions *Forms*): a bordered row of
/// controls with the standard padding, so a form reads as one unit
/// without a modal. (Built ahead of the pages that move onto it.)
#[allow(dead_code)]
pub(super) fn inline_form(cx: &App) -> Div {
    let (border, radius) = {
        let theme = cx.theme();
        (theme.border, theme.radius)
    };
    h_flex()
        .w_full()
        .gap_2()
        .items_end()
        .p_3()
        .rounded(radius)
        .border_1()
        .border_color(border)
}

/// A category color swatch (ui-conventions *Category colors*): a 10 px
/// square with a 2 px radius beside the category name. (Built ahead of
/// the pages that move onto it; 3.51 adopts it.)
#[allow(dead_code)]
pub(super) fn swatch(color: Hsla) -> Div {
    div().size(px(10.)).flex_none().rounded(px(2.)).bg(color)
}

/// The income/expense tag (ui-conventions *Components*): an outline Tag,
/// the same widget wherever a flow type is labelled.
pub(super) fn type_tag(label: impl Into<SharedString>) -> Tag {
    Tag::new().outline().child(label.into())
}

/// A row action: a ghost xsmall icon button with a tooltip naming it
/// (ui-conventions *Interaction* — icon-only actions always explain
/// themselves on hover). (Built ahead of the pages that move onto it.)
#[allow(dead_code)]
pub(super) fn row_action_button(
    id: impl Into<SharedString>,
    icon: IconName,
    tooltip: impl Into<SharedString>,
) -> Button {
    let id: SharedString = id.into();
    Button::new(id).ghost().xsmall().icon(icon).tooltip(tooltip)
}

// -- Responsive chart labels (3.15) -------------------------------------
//
// `BarChart` paints each band label centred under its band at gpui-kit's
// 10px `TEXT_SIZE` and offers no truncation, so the app prepares the
// text: names are ellipsized to a character budget, and when even short
// labels cannot share a band they are thinned out with
// `BarChart::band_tick_count`. The budgets come from the window's live
// width (tracked through `observe_window_bounds`), so labels grow as the
// window grows and never overlap or spill past the panel at the minimum
// size (1200 × 720) or above.
//
// Layout widths shared by the dashboard and the budget maths: the fixed
// sidebar, the legend column inside the chart panel, the panel's share
// of the dashboard's middle row, the 16px padding of pages and panels
// (`p_4`) and the 12px gap between regions (`gap_3`).
pub(super) const SIDEBAR_W: f32 = 210.;
pub(super) const PAGE_PAD: f32 = 16.;
/// Generous width of one 10px system-font character, rounded up so the
/// budget errs towards labels that are too short rather than overlapping.
pub(super) const CHART_CHAR_PX: f32 = 6.5;
/// A generous width of one 14 px (`text_sm`) system-font character, the
/// same rounding-up spirit as `CHART_CHAR_PX` (which covers the chart's
/// 10 px font, scaled by 14/10): the budget errs towards columns that
/// are too wide rather than labels that clip.
pub(super) const TEXT_SM_CHAR_PX: f32 = CHART_CHAR_PX * 14. / 10.;
/// Horizontal padding inside an xsmall `Button`: `px_1` per side.
pub(super) const BUTTON_PAD_X: f32 = 8.;
/// The in-row Undo button's icon plus the gap between icon and label:
/// xsmall renders its icon at `size_3` (12 px) with a `gap_1` (4 px)
/// before the label (3.32).
pub(super) const UNDO_ICON_W: f32 = 16.;
/// Horizontal padding inside a table cell (8 px per side); gpui-kit
/// draws the cell inside the column width, so it comes off the space
/// the label has.
pub(super) const CELL_PAD_X: f32 = 16.;
/// The table's 1 px border on each side; the page width includes it.
pub(super) const TABLE_BORDER_X: f32 = 2.;

/// The category palette (ui-conventions, *Category colors*): the six base
/// colors then their light variants, so red and green — which also mean
/// danger and money in — come last.
pub(super) fn category_palette(theme: &gpui_kit::component::theme::Theme) -> [Hsla; 12] {
    [
        theme.blue,
        theme.cyan,
        theme.magenta,
        theme.yellow,
        theme.green,
        theme.red,
        theme.blue_light,
        theme.cyan_light,
        theme.magenta_light,
        theme.yellow_light,
        theme.green_light,
        theme.red_light,
    ]
}

/// 3.27: how long a success/info toast stays up before dismissing itself.
/// Errors have no timeout; they stay until the user closes them
/// (`docs/ui-conventions.md`, *Feedback*).
const TOAST_TIMEOUT: Duration = Duration::from_secs(4);
/// How often the toast ticker advances the manager's lifecycle clock while
/// any toast is mounted. Coarser than a frame on purpose: the stack's
/// motion is spring-driven, and the clock only gates enter/timeout/exit.
const TOAST_TICK: Duration = Duration::from_millis(100);
/// Width of one toast card; wide enough for the longest message the app
/// writes on one or two lines.
const TOAST_W: f32 = 360.;

/// 3.27: one toast in the manager — its text and whether it is an error
/// (error text uses `danger` and the toast persists until dismissed).
pub(super) struct ToastMessage {
    pub(super) message: SharedString,
    pub(super) is_error: bool,
}

impl Shell {
    /// Success/info feedback: a toast in the bottom-right corner that
    /// dismisses itself after [`TOAST_TIMEOUT`]. Every former
    /// `self.status = Some((msg, false))` call site goes through here.
    pub(super) fn notify_ok(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast(message.into(), false, cx);
    }

    /// Failure feedback: a toast that stays until the user closes it.
    pub(super) fn notify_err(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast(message.into(), true, cx);
    }

    fn push_toast(&mut self, message: SharedString, is_error: bool, cx: &mut Context<Self>) {
        let id = self.toast_next_id;
        self.toast_next_id += 1;
        self.toasts.push(
            id,
            ToastMessage { message, is_error },
            ToastOptions {
                timeout: if is_error { None } else { Some(TOAST_TIMEOUT) },
            },
            self.toast_clock,
        );
        self.tick_toasts(cx);
        cx.notify();
    }

    /// The user closed a toast — gpui-base toasts dismiss on click.
    fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.toasts.dismiss(&id, self.toast_clock) {
            self.tick_toasts(cx);
            cx.notify();
        }
    }

    /// Drive the toast lifecycle while anything is mounted.
    ///
    /// `ToastManager` is a pure function of the clock it is handed, so the
    /// ticker advances a *virtual* clock by the timer's interval rather than
    /// calling `Instant::now()`: the test executor's `advance_clock` moves
    /// its timers without moving real time, and the auto-dismiss tests need
    /// both to agree. The clock only matters relatively — between toasts it
    /// stands still, and every entry's deadlines are deltas from its push.
    ///
    /// A hovered or focused stack pauses the auto-hide timers (gpui-base's
    /// semantics), so a toast the user is reaching for never vanishes under
    /// the pointer.
    pub(super) fn tick_toasts(&mut self, cx: &mut Context<Self>) {
        if self.toast_task.is_some() || self.toasts.is_empty() {
            return;
        }
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TOAST_TICK).await;
                let running = this.update(cx, |this, cx| {
                    this.toast_clock += TOAST_TICK;
                    let advanced = this
                        .toasts
                        .advance(this.toast_clock, this.toast_stack.is_expanded());
                    let running = !this.toasts.is_empty();
                    if !running {
                        this.toast_task = None;
                    } else if advanced.changed {
                        cx.notify();
                    }
                    running
                });
                if !running.unwrap_or(false) {
                    return;
                }
            }
        }));
    }

    /// The toast overlay (`docs/ui-conventions.md`, *Feedback*): anchored
    /// bottom-right, stacked, newest in front. It is absolutely positioned,
    /// so it overlays the window and takes no layout space — the page area
    /// starts at the top of the content padding whether or not a toast
    /// shows. Card styling follows the panel rule (border, radius) on the
    /// `popover` surface, `text_sm`; error text is `danger`.
    pub(super) fn render_toasts(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.toasts.is_empty() {
            return div().into_any_element();
        }
        let (popover, foreground, border, danger, radius) = {
            let theme = cx.theme();
            (
                theme.popover,
                theme.foreground,
                theme.border,
                theme.danger,
                theme.radius,
            )
        };
        let mut stack = ToastStack::new("toasts", self.toast_stack.clone())
            .placement(Anchor::BottomRight)
            .absolute()
            .bottom_4()
            .right_4()
            .w(px(TOAST_W));
        for (id, toast, status) in self.toasts.iter() {
            let id = *id;
            stack = stack.item(
                id,
                Toast::new(format!("toast-{id}"))
                    .transition_status(status)
                    .w_full()
                    .child(
                        div()
                            .id(("toast-dismiss", id))
                            .w_full()
                            .rounded(radius)
                            .border_1()
                            .border_color(border)
                            .bg(popover)
                            .text_color(if toast.is_error { danger } else { foreground })
                            .px_3()
                            .py_2()
                            .debug_selector({
                                let message = toast.message.clone();
                                move || format!("toast:{message}")
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dismiss_toast(id, cx);
                            }))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_start()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .child(toast.message.clone()),
                                    )
                                    .child(div().flex_none().child("×")),
                            ),
                    ),
            );
        }
        stack.into_any_element()
    }

    // -- rendering ---------------------------------------------------------
}

/// Run `f` on the database thread, flattening worker and query errors.
///
/// 3.92: dispatches through the typed path ([`on_db_typed`]), so a job
/// that *panics* resolves [`DbError::JobPanicked`] — the user sees "the
/// database job panicked; the worker survived" instead of the old
/// worker-gone text, which a dropped result sender produced on the
/// untyped path. `WorkerGone` keeps meaning the worker thread actually
/// went away, and a job that fails normally passes its own message
/// through unchanged.
pub(super) async fn on_db<R, E, F>(db: &DbWorker, f: F) -> Result<R, String>
where
    R: Send + 'static,
    E: std::fmt::Display + Send + 'static,
    F: FnOnce(&mut Connection) -> Result<R, E> + Send + 'static,
{
    // The wrapper never returns `Err` on its own: the only errors
    // `on_db_typed` can produce here are `JobPanicked` and `WorkerGone`.
    let wrapped = move |c: &mut Connection| Ok::<_, DbError>(f(c).map_err(|e| e.to_string()));
    match on_db_typed(db, wrapped).await {
        Ok(result) => result,
        Err(e) => Err(e.to_string()),
    }
}

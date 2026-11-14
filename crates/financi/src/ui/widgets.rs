//! Shared view helpers: small widgets, labels, toasts and the width
//! constants pages agree on (split from `ui.rs` by 3.62).

use super::*;
use financi_db::{DbError, on_db_typed};

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

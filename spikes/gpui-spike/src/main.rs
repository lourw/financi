//! M0 GPUI spike. Throwaway: proves each UI piece financi needs exists and
//! works with the pinned gpui-kit / gpui-pre versions.
//!
//! Checks:
//! 1. external file drop onto the window (import)
//! 2. gpui-kit BarChart, plus hand-drawn clickable bars (bar-click filtering;
//!    BarChart has no click handler)
//! 3. virtual DataTable with 5,000 rows, filtered by the selected bar
//! 4. Select dropdown (account picker)

use gpui_kit::component::chart::BarChart;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

const CATEGORIES: [&str; 6] = [
    "Groceries",
    "Dining",
    "Transit",
    "Shopping",
    "Bills",
    "Travel",
];

#[derive(Clone)]
struct Txn {
    date: String,
    merchant: String,
    category: &'static str,
    cents: i64,
}

fn fake_transactions(n: usize) -> Vec<Txn> {
    (0..n)
        .map(|i| Txn {
            date: format!("2026-{:02}-{:02}", 1 + (i % 10), 1 + (i % 28)),
            merchant: format!("MERCHANT {:04}", i % 300),
            category: CATEGORIES[(i * 7 + i / 3) % CATEGORIES.len()],
            cents: -(((i * 3779) % 15_000) as i64 + 150),
        })
        .collect()
}

struct TxnTable {
    rows: Vec<Txn>,
}

impl TableDelegate for TxnTable {
    fn columns_count(&self, _: &App) -> usize {
        4
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("date", "Date"),
            1 => Column::new("merchant", "Merchant"),
            2 => Column::new("category", "Category"),
            _ => Column::new("amount", "Amount"),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let t = &self.rows[row_ix];
        let text: SharedString = match col_ix {
            0 => t.date.clone().into(),
            1 => t.merchant.clone().into(),
            2 => t.category.into(),
            _ => format!("{:.2}", t.cents as f64 / 100.0).into(),
        };
        div().child(text)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let t = &self.rows[row_ix];
        match col_ix {
            0 => t.date.clone(),
            1 => t.merchant.clone(),
            2 => t.category.to_string(),
            _ => format!("{:.2}", t.cents as f64 / 100.0),
        }
    }
}

struct Spike {
    all: Vec<Txn>,
    totals: Vec<(&'static str, f64)>,
    selected: Option<&'static str>,
    dropped: Vec<String>,
    account: Option<SharedString>,
    table: Entity<TableState<TxnTable>>,
    accounts: Entity<SelectState<Vec<SharedString>>>,
    _subs: Vec<Subscription>,
}

impl Spike {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let all = fake_transactions(5_000);
        let mut totals: Vec<(&'static str, f64)> = CATEGORIES
            .iter()
            .map(|c| {
                let sum: i64 = all
                    .iter()
                    .filter(|t| t.category == *c)
                    .map(|t| -t.cents)
                    .sum();
                (*c, sum as f64 / 100.0)
            })
            .collect();
        totals.sort_by(|a, b| b.1.total_cmp(&a.1));

        let table = cx.new(|cx| TableState::new(TxnTable { rows: all.clone() }, window, cx));
        let options: Vec<SharedString> = vec![
            "Amex Cobalt".into(),
            "Amex Gold".into(),
            "Simplii Chequing".into(),
            "Simplii Visa".into(),
        ];
        let accounts = cx.new(|cx| SelectState::new(options, None, window, cx));
        let sub = cx.subscribe(
            &accounts,
            |this, _, ev: &SelectEvent<Vec<SharedString>>, cx| {
                let SelectEvent::Confirm(value) = ev;
                this.account = value.clone();
                cx.notify();
            },
        );

        Self {
            all,
            totals,
            selected: None,
            dropped: Vec::new(),
            account: None,
            table,
            accounts,
            _subs: vec![sub],
        }
    }

    fn select_category(&mut self, cat: &'static str, cx: &mut Context<Self>) {
        self.selected = if self.selected == Some(cat) {
            None
        } else {
            Some(cat)
        };
        let rows: Vec<Txn> = self
            .all
            .iter()
            .filter(|t| self.selected.is_none_or(|c| t.category == c))
            .cloned()
            .collect();
        self.table.update(cx, |state, cx| {
            state.delegate_mut().rows = rows;
            state.refresh(cx);
        });
        cx.notify();
    }

    fn clickable_bars(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let max = self.totals.first().map(|t| t.1).unwrap_or(1.0);
        let accent = cx.theme().primary;
        let muted = cx.theme().muted;
        v_flex()
            .gap_1()
            .children(self.totals.iter().map(|(cat, total)| {
                let cat = *cat;
                let active = self.selected.is_none_or(|s| s == cat);
                h_flex()
                    .id(SharedString::from(cat))
                    .gap_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.select_category(cat, cx)))
                    .child(div().w(px(90.)).child(cat))
                    .child(
                        div()
                            .h(px(16.))
                            .w(relative((*total / max) as f32 * 0.7))
                            .rounded_sm()
                            .bg(if active { accent } else { muted }),
                    )
                    .child(format!("${total:.0}"))
            }))
    }
}

impl Render for Spike {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dropped = if self.dropped.is_empty() {
            "Drop CSV files anywhere on this window".to_string()
        } else {
            format!("Dropped: {}", self.dropped.join(", "))
        };
        let account = self.account.clone().unwrap_or_else(|| "none".into());
        let selected = self.selected.unwrap_or("all categories");

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.dropped = paths
                    .paths()
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                cx.notify();
            }))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .w(px(220.))
                            .child(Select::new(&self.accounts).placeholder("Account")),
                    )
                    .child(format!("Account: {account}"))
                    .child(dropped),
            )
            .child(
                h_flex()
                    .gap_4()
                    .h(px(220.))
                    .child(
                        div().flex_1().h_full().child(
                            BarChart::new(self.totals.clone())
                                .band(|d: &(&'static str, f64)| d.0)
                                .value(|d: &(&'static str, f64)| d.1),
                        ),
                    )
                    .child(div().flex_1().child(self.clickable_bars(cx))),
            )
            .child(format!("Showing: {selected} (click a bar on the right)"))
            .child(
                div()
                    .flex_1()
                    .child(DataTable::new(&self.table).stripe(true)),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(800.)), cx)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Spike::new(window, cx))
        })
        .expect("failed to open window");
    });
}

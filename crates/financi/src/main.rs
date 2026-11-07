//! financi app entry point. There is no CLI: the binary opens the window.
//!
//! Development switches, honoured only in debug builds:
//! - `FINANCI_DB=/path/to/file.sqlite` uses that database instead of the
//!   per-OS app data directory, so dev data never touches the real file.
//! - `FINANCI_DEV_SEED=1` fills an empty database with synthetic data.

#[allow(dead_code)] // wired into the import UI in the next commit
mod flow;

use std::path::PathBuf;

use anyhow::Context as _;
use financi_db::DbWorker;
use gpui_kit::component::chart::BarChart;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::*;

fn db_path() -> anyhow::Result<PathBuf> {
    if cfg!(debug_assertions)
        && let Some(path) = std::env::var_os("FINANCI_DB")
    {
        return Ok(PathBuf::from(path));
    }
    Ok(financi_db::default_db_path()?)
}

fn open_db() -> anyhow::Result<(PathBuf, DbWorker)> {
    let path = db_path()?;
    let mut conn =
        financi_db::open(&path).with_context(|| format!("opening {}", path.display()))?;

    if cfg!(debug_assertions) && std::env::var_os("FINANCI_DEV_SEED").is_some() {
        let empty: bool =
            conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM transactions)", [], |r| {
                r.get(0)
            })?;
        if empty {
            let today = chrono::Local::now().date_naive();
            let s = financi_db::seed::seed(&mut conn, today, 18)?;
            eprintln!(
                "financi: seeded {} categories, {} rules, {} transactions",
                s.categories, s.rules, s.transactions
            );
        }
    }
    Ok((path, DbWorker::spawn(conn)))
}

enum Load {
    Loading,
    Ready(Vec<(String, f64)>),
    Failed(String),
}

struct Shell {
    db_path: PathBuf,
    spend: Load,
    _load: Task<()>,
}

impl Shell {
    fn new(db_path: PathBuf, db: DbWorker, cx: &mut Context<Self>) -> Self {
        // Query on the DB thread; update the view when it answers.
        let load = cx.spawn(async move |this, cx| {
            let result = db
                .call(|conn| financi_db::spend_by_parent_category(conn))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = this.update(cx, |this, cx| {
                this.spend = match result {
                    Ok(rows) => Load::Ready(
                        rows.into_iter()
                            .map(|(name, cents)| (name, cents as f64 / 100.0))
                            .collect(),
                    ),
                    Err(e) => Load::Failed(e),
                };
                cx.notify();
            });
        });
        Self {
            db_path,
            spend: Load::Loading,
            _load: load,
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match &self.spend {
            Load::Loading => div().child("Loading…").into_any_element(),
            Load::Failed(e) => div()
                .child(format!("Database error: {e}"))
                .into_any_element(),
            Load::Ready(rows) if rows.is_empty() => div()
                .child("No transactions yet. Importing arrives in M1.")
                .into_any_element(),
            Load::Ready(rows) => div()
                .h(px(320.))
                .child(
                    BarChart::new(rows.clone())
                        .band(|d: &(String, f64)| d.0.clone())
                        .value(|d: &(String, f64)| d.1)
                        .tooltip_value(|_, v| format!("${v:.2}").into()),
                )
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_xl().child("financi"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Database: {}", self.db_path.display())),
            )
            .child(div().child("Spend by category, all time"))
            .child(body)
    }
}

fn main() -> anyhow::Result<()> {
    let (path, db) = open_db()?;
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1000.), px(560.)), cx)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|cx| Shell::new(path, db, cx)))
            .expect("failed to open window");
    });
    Ok(())
}

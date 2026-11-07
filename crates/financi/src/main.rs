//! financi app entry point. There is no CLI: the binary opens the window.
//!
//! Development switches, honoured only in debug builds:
//! - `FINANCI_DB=/path/to/file.sqlite` uses that database instead of the
//!   per-OS app data directory, so dev data never touches the real file.
//! - `FINANCI_DEV_SEED=1` fills an empty database with synthetic data.
//! - `FINANCI_DEV_OPEN=/path/to/export.csv` opens that file's import
//!   preview at startup (handy for screenshots and manual checks).

mod flow;
mod ui;

use std::path::PathBuf;

use anyhow::Context as _;
use financi_db::DbWorker;
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

fn main() -> anyhow::Result<()> {
    let (path, db) = open_db()?;
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let mut shell = ui::Shell::new(path, db, window, cx);
                if cfg!(debug_assertions)
                    && let Some(file) = std::env::var_os("FINANCI_DEV_OPEN")
                {
                    shell.open_file(PathBuf::from(file), window, cx);
                }
                shell
            })
        })
        .expect("failed to open window");
    });
    Ok(())
}

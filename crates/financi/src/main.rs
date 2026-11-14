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
mod version;

use std::fs::OpenOptions;
use std::io::Write;
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

    // 3.76: the master-CSV self-check that used to run here now rides the
    // launch `DbWorker` job (see the startup branch of `Shell::refresh`),
    // so nothing but opening the database stands between launch and a
    // window.

    Ok((path, DbWorker::spawn(conn)))
}

/// Append a startup failure to `<data dir>/financi.log` (3.76), so the
/// reason survives without a terminal and can be shared. Best effort: if
/// the data directory itself is unwritable there is nowhere left to log.
fn write_startup_log(message: &str) {
    let Ok(path) = db_path() else { return };
    let Some(dir) = path.parent() else { return };
    let Ok(mut log) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("financi.log"))
    else {
        return;
    };
    let _ = writeln!(
        log,
        "[{}] {message}",
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );
}

/// Open the app's main window (3.76: shared by launch and the macOS
/// reopen handler, so a Dock click after the last window closed brings it
/// back).
fn open_main_window(path: PathBuf, db: DbWorker, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
        window_min_size: Some(size(px(1200.), px(720.))),
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
}

fn main() {
    let (path, db) = match open_db() {
        Ok(opened) => opened,
        Err(e) => {
            // 3.76: a startup failure reaches the user as a window, not
            // only a terminal line — and that path needs nothing from the
            // database whose opening failed.
            let message = format!("financi could not open its database:\n{e:#}");
            eprintln!("{message}");
            write_startup_log(&message);
            gpui_kit::application()
                .with_assets(icon_assets())
                .run(move |cx| {
                    gpui_kit::init(cx);
                    ui::init(cx);
                    ui::open_error_window(message, cx);
                });
            return;
        }
    };
    let app = gpui_kit::application().with_assets(icon_assets());
    // 3.76: macOS asks a running app to reopen when its Dock icon is
    // clicked with no windows; bring the main window back. `DbWorker`
    // clones cheaply — the worker thread is shared.
    app.on_reopen({
        let path = path.clone();
        let db = db.clone();
        move |cx: &mut App| {
            if cx.windows().is_empty() {
                open_main_window(path.clone(), db.clone(), cx);
            }
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        // Global wiring once per app (3.76) — before the first window.
        ui::init(cx);
        open_main_window(path, db, cx);
    });
}

/// The icon assets registered on the application (3.14).
///
/// gpui-kit draws a Checkbox's check mark — and every `IconName` icon, e.g.
/// the sidebar's — as an SVG loaded through GPUI's application asset source.
/// GPUI's default source loads nothing, so without this a checked box
/// changes background but shows no check. `AllAssets` is the full bundled
/// catalog; the smaller `Assets` bundle lacks icons the app uses, such as
/// `list-checks`, `tags`, `arrow-left-right` and `download`.
pub(crate) fn icon_assets() -> gpui_kit::assets::AllAssets {
    gpui_kit::assets::AllAssets
}

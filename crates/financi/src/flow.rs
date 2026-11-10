//! Import flow logic used by the UI, kept free of GPUI so it can be tested:
//! load a dropped file, pick candidate accounts, summarize a plan.

use chrono::Datelike;
use std::path::Path;

use financi_core::{AccountKind, RawFile, ReadError, StatementParser, read_raw_file};
use financi_db::Account;
use financi_db::import::ImportPlan;

pub type Parser = Box<dyn StatementParser + Send + Sync>;

/// A dropped CSV, read and matched to a parser. Nothing is written yet.
#[derive(Debug)]
pub struct LoadedFile {
    pub file_name: String,
    pub raw: RawFile,
    pub parser_id: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("could not read {name}: {message}")]
    Io { name: String, message: String },
    #[error("{name}: {source}")]
    Read { name: String, source: ReadError },
    #[error("{name} is not a recognized export (header: {header})")]
    UnknownFormat { name: String, header: String },
}

pub fn load_file(path: &Path) -> Result<LoadedFile, LoadError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let bytes = std::fs::read(path).map_err(|e| LoadError::Io {
        name: name.clone(),
        message: e.to_string(),
    })?;
    let raw = read_raw_file(&bytes).map_err(|source| LoadError::Read {
        name: name.clone(),
        source,
    })?;
    let parser = financi_parsers::detect(&raw.header).ok_or_else(|| LoadError::UnknownFormat {
        name: name.clone(),
        header: raw.header.join(","),
    })?;
    Ok(LoadedFile {
        file_name: name,
        parser_id: parser.id(),
        raw,
    })
}

pub fn parser(id: &str) -> Parser {
    financi_parsers::by_id(id).unwrap_or_else(|| panic!("no parser with id {id:?}"))
}

/// A human name for a parser's format, e.g. "American Express credit card".
/// A format that feeds several account kinds lists them all, joined with
/// " / ".
pub fn format_label(parser: &dyn StatementParser) -> String {
    let institution = match parser.institution() {
        "amex" => "American Express",
        "simplii" => "Simplii Financial",
        other => other,
    };
    let kinds: Vec<_> = parser
        .account_kinds()
        .iter()
        .map(|k| k.label().to_lowercase())
        .collect();
    format!("{institution} {}", kinds.join(" / "))
}

/// Accounts this file may be imported into: same institution, and an
/// account type the format comes from.
pub fn eligible_accounts(accounts: &[Account], parser: &dyn StatementParser) -> Vec<Account> {
    accounts
        .iter()
        .filter(|a| {
            a.institution.key == parser.institution() && parser.account_kinds().contains(&a.kind)
        })
        .cloned()
        .collect()
}

/// Institution key and account type for a new account created from the
/// import preview: both follow from the detected format.
pub fn new_account_defaults(parser: &dyn StatementParser) -> (&'static str, AccountKind) {
    (parser.institution(), parser.account_kinds()[0])
}

/// What the preview shows for a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSummary {
    pub total: usize,
    pub new_rows: usize,
    pub already_present: usize,
    pub first_date: String,
    pub last_date: String,
    /// Other account name and how many rows it already holds.
    pub overlap: Option<(String, usize)>,
}

impl PlanSummary {
    pub fn from_plan(plan: &ImportPlan) -> Self {
        Self {
            total: plan.total_rows,
            new_rows: plan.new_rows.len(),
            already_present: plan.already_present,
            first_date: plan.date_range.0.clone(),
            last_date: plan.date_range.1.clone(),
            overlap: plan
                .overlap
                .as_ref()
                .map(|o| (o.account.display_name(), o.matching_rows)),
        }
    }

    /// The second preview line per ui-conventions:
    /// `32 new of 84 rows · 52 already imported, skipped · Oct 1 – Dec 4, 2026`.
    /// The first span (`32 new`) renders in the foreground, the rest muted.
    pub fn preview_parts(&self) -> (String, String) {
        (
            format!("{} new", self.new_rows),
            format!(
                "of {} rows · {} already imported, skipped · {}",
                self.total,
                self.already_present,
                date_range(&self.first_date, &self.last_date)
            ),
        )
    }
}

/// A plain grouped count for secondary text: `1284` -> `1,284`.
pub fn count(n: i64) -> String {
    let sign = if n < 0 { "-" } else { "" };
    let digits = n.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("{sign}{grouped}")
}

/// Format cents for data regions per ui-conventions: `$1,234.56`, with
/// thousands separators and the sign before the `$` (`-$86.42`).
pub fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let digits = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("{sign}${grouped}.{:02}", abs % 100)
}

/// An ISO `yyyy-mm-dd` date range as shown in history rows:
/// `Sep 1 – Nov 30, 2026` (en dash). One date renders alone; different
/// years show both; unparseable input comes back unchanged.
pub fn date_range(first: &str, last: &str) -> String {
    let parse = |d: &str| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    let (Some(f), Some(l)) = (parse(first), parse(last)) else {
        if last.is_empty() || first == last {
            return first.to_string();
        }
        return format!("{first} – {last}");
    };
    if f == l {
        return long_date(&f);
    }
    if f.year() == l.year() {
        format!("{} – {}", short_date(&f), long_date(&l))
    } else {
        format!("{} – {}", long_date(&f), long_date(&l))
    }
}

fn short_date(d: &chrono::NaiveDate) -> String {
    format!("{} {}", month_name(d.month()), d.day())
}

fn long_date(d: &chrono::NaiveDate) -> String {
    format!("{} {}, {}", month_name(d.month()), d.day(), d.year())
}

fn month_name(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES[(month as usize - 1).min(11)]
}

/// A `YYYY-MM` month key from [`financi_db::monthly_summary`] rendered as a
/// summary column header: `2026-09` → `Sep 2026` (ui-conventions,
/// *Numbers, dates and labels*). Unparseable input comes back unchanged.
pub fn month_label(month: &str) -> String {
    let year = month.get(..4).and_then(|y| y.parse::<i32>().ok());
    let num = month.get(5..7).and_then(|m| m.parse::<u32>().ok());
    match (year, num) {
        (Some(y), Some(m @ 1..=12)) => format!("{} {}", month_name(m), y),
        _ => month.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use financi_db::import::{CommitOptions, commit_import, list_transactions, plan_import};
    use financi_db::{create_account, list_accounts, open_in_memory};
    use std::path::PathBuf;

    fn demo(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/demo")
            .join(name)
    }

    fn import_path(
        conn: &mut financi_db::rusqlite::Connection,
        account: i64,
        path: &Path,
    ) -> usize {
        let file = load_file(path).unwrap();
        commit_import(
            conn,
            account,
            parser(file.parser_id).as_ref(),
            &file.raw,
            &file.file_name,
            "2026-11-07T12:00:00",
            CommitOptions::default(),
        )
        .unwrap()
        .inserted
    }

    #[test]
    fn loads_and_detects_each_demo_export() {
        let amex = load_file(&demo("amex-sep-oct.csv")).unwrap();
        assert_eq!(amex.parser_id, "amex");
        assert_eq!(amex.file_name, "amex-sep-oct.csv");
        let simplii = load_file(&demo("simplii-chequing.csv")).unwrap();
        assert_eq!(simplii.parser_id, "simplii");
        assert_eq!(
            format_label(parser(simplii.parser_id).as_ref()),
            "Simplii Financial chequing / credit card"
        );
    }

    #[test]
    fn load_errors_are_readable() {
        let missing = load_file(Path::new("/nonexistent/x.csv")).unwrap_err();
        assert!(
            missing.to_string().starts_with("could not read x.csv"),
            "{missing}"
        );

        let dir = std::env::temp_dir().join(format!("financi-flow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let odd = dir.join("odd.csv");
        std::fs::write(&odd, "Posted,Payee,Debit\n2026-11-01,X,1.00\n").unwrap();
        let err = load_file(&odd).unwrap_err();
        assert!(matches!(err, LoadError::UnknownFormat { .. }), "{err}");
        assert!(err.to_string().contains("Posted,Payee,Debit"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_matching_institution_and_type_are_eligible() {
        let conn = open_in_memory().unwrap();
        create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
        create_account(&conn, "amex", "Gold", AccountKind::CreditCard).unwrap();
        create_account(&conn, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();
        create_account(&conn, "simplii", "Visa", AccountKind::CreditCard).unwrap();
        let accounts = list_accounts(&conn).unwrap();

        let names = |p: &str| -> Vec<String> {
            eligible_accounts(&accounts, parser(p).as_ref())
                .into_iter()
                .map(|a| a.name)
                .collect()
        };
        assert_eq!(names("amex"), ["Cobalt", "Gold"]);
        assert_eq!(
            names("simplii"),
            ["No Fee Chequing", "Visa"],
            "one format feeds both account kinds (task 4.1)"
        );
        assert_eq!(
            new_account_defaults(parser("simplii").as_ref()),
            ("simplii", AccountKind::Chequing),
            "chequing is listed first, so it stays the default"
        );
    }

    #[test]
    fn demo_files_reimport_cleanly_end_to_end() {
        let mut conn = open_in_memory().unwrap();
        let cobalt = create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
        let chequing =
            create_account(&conn, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();

        assert_eq!(
            import_path(&mut conn, cobalt, &demo("amex-sep-oct.csv")),
            68
        );
        // Sep-Nov overlaps Sep 1 - Oct 15 exactly; processed dates near the
        // boundary differ, which must not matter.
        let file = load_file(&demo("amex-sep-nov.csv")).unwrap();
        let plan = plan_import(
            &conn,
            cobalt,
            parser("amex").as_ref(),
            &file.raw,
            &file.file_name,
        )
        .unwrap();
        let summary = PlanSummary::from_plan(&plan);
        assert_eq!(
            (summary.total, summary.new_rows, summary.already_present),
            (102, 34, 68)
        );
        assert_eq!(summary.overlap, None);
        assert_eq!(
            summary.preview_parts(),
            (
                "34 new".into(),
                "of 102 rows · 68 already imported, skipped · Sep 1 – Nov 6, 2026".into()
            )
        );

        assert_eq!(
            import_path(&mut conn, cobalt, &demo("amex-sep-nov.csv")),
            34
        );
        assert_eq!(import_path(&mut conn, cobalt, &demo("amex-sep-nov.csv")), 0);
        assert_eq!(
            import_path(&mut conn, chequing, &demo("simplii-chequing.csv")),
            11
        );

        assert_eq!(list_transactions(&conn, Some(cobalt)).unwrap().len(), 102);
        assert_eq!(list_transactions(&conn, None).unwrap().len(), 113);
    }

    #[test]
    fn summary_surfaces_other_account_overlap() {
        let mut conn = open_in_memory().unwrap();
        let cobalt = create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
        let gold = create_account(&conn, "amex", "Gold", AccountKind::CreditCard).unwrap();
        import_path(&mut conn, cobalt, &demo("amex-sep-oct.csv"));

        let file = load_file(&demo("amex-sep-nov.csv")).unwrap();
        let plan = plan_import(
            &conn,
            gold,
            parser("amex").as_ref(),
            &file.raw,
            &file.file_name,
        )
        .unwrap();
        assert_eq!(
            PlanSummary::from_plan(&plan).overlap,
            Some(("American Express · Cobalt".into(), 68))
        );
    }

    #[test]
    fn formats_money() {
        assert_eq!(money(-1574), "-$15.74");
        assert_eq!(money(150_000), "$1,500.00");
        assert_eq!(money(-5), "-$0.05");
        assert_eq!(money(0), "$0.00");
        assert_eq!(money(123_456_700), "$1,234,567.00");
    }

    #[test]
    fn formats_date_ranges() {
        assert_eq!(
            date_range("2026-09-01", "2026-11-30"),
            "Sep 1 – Nov 30, 2026"
        );
        assert_eq!(date_range("2026-11-06", "2026-11-06"), "Nov 6, 2026");
        assert_eq!(
            date_range("2025-12-28", "2026-01-03"),
            "Dec 28, 2025 – Jan 3, 2026"
        );
        assert_eq!(date_range("2026-11-06", ""), "2026-11-06");
    }

    #[test]
    fn formats_month_labels() {
        assert_eq!(month_label("2026-09"), "Sep 2026");
        assert_eq!(month_label("2026-01"), "Jan 2026");
        assert_eq!(month_label("2026-12"), "Dec 2026");
        assert_eq!(month_label("2026-13"), "2026-13");
        assert_eq!(month_label("nope"), "nope");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1284), "1,284");
        assert_eq!(count(1_000_000), "1,000,000");
    }
}

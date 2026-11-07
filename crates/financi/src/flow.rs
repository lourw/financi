//! Import flow logic used by the UI, kept free of GPUI so it can be tested:
//! load a dropped file, pick candidate accounts, summarize a plan.

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

/// A human name for a parser's format, e.g. "Simplii Financial chequing".
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

    pub fn headline(&self) -> String {
        format!(
            "{} rows from {} to {}: {} new, {} already imported",
            self.total, self.first_date, self.last_date, self.new_rows, self.already_present
        )
    }
}

/// Format cents for display: `-1574` -> `"-15.74"`.
pub fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
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
        assert_eq!(simplii.parser_id, "simplii-chequing");
        assert_eq!(
            format_label(parser(simplii.parser_id).as_ref()),
            "Simplii Financial chequing"
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
            names("simplii-chequing"),
            ["No Fee Chequing"],
            "Simplii Visa is not eligible"
        );
        assert_eq!(
            new_account_defaults(parser("simplii-chequing").as_ref()),
            ("simplii", AccountKind::Chequing)
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
            summary.headline(),
            "102 rows from 2026-09-01 to 2026-11-06: 34 new, 68 already imported"
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
        assert_eq!(money(-1574), "-15.74");
        assert_eq!(money(150_000), "1500.00");
        assert_eq!(money(-5), "-0.05");
        assert_eq!(money(0), "0.00");
    }
}

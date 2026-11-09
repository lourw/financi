//! Filter model and filtered dashboard queries (task 3.1).
//!
//! [`ViewFilter`] is the single filter shared by both queries:
//! [`spend_by_category`] feeds the category chart and legend,
//! [`monthly_summary`] the monthly grid. With the default (open) filter the
//! spend rules agree with [`crate::spend_by_parent_category`].
//!
//! Classification rules (both queries):
//!
//! - **Spend**: rows whose category kind is `expense`, *or* that are
//!   uncategorized, *or* whose kind is `transfer` when
//!   [`ViewFilter::include_transfers`]. Shown positive:
//!   `spend = -(sum of amount_cents)`, so refunds (positive amounts inside a
//!   spend category) net against their category.
//! - **Income**: rows whose category kind is `income`, shown positive:
//!   `income = sum of amount_cents`.
//! - Hidden rows are always excluded; `from` and `to` are inclusive; an
//!   empty [`ViewFilter::account_ids`] means all accounts.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, ToSql, params_from_iter};

use crate::Result;
use crate::import::TransactionRow;

/// The dashboard filter shared by [`spend_by_category`] and
/// [`monthly_summary`].
///
/// `from` and `to` are inclusive calendar dates; empty `account_ids` means
/// every account. [`Default`] is the open filter: a date range covering
/// everything, all accounts, transfers excluded.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewFilter {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub account_ids: Vec<i64>,
    pub include_transfers: bool,
}

impl Default for ViewFilter {
    fn default() -> Self {
        Self {
            // The open-range sentinels shared with the date presets
            // (`financi_core::dates`): full four-digit years, because
            // `posted_date` is compared as a `YYYY-MM-DD` string, and
            // chrono renders out-of-range years with a sign that breaks
            // that ordering.
            from: financi_core::dates::ALL_TIME_FROM,
            to: financi_core::dates::ALL_TIME_TO,
            account_ids: Vec::new(),
            include_transfers: false,
        }
    }
}

/// Which column the dashboard transaction table is ordered by (task 3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxSortColumn {
    Date,
    Amount,
}

/// The table's ordering: a [`TxSortColumn`] and its direction. The default
/// is date, newest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxSort {
    pub column: TxSortColumn,
    pub descending: bool,
}

impl Default for TxSort {
    fn default() -> Self {
        Self {
            column: TxSortColumn::Date,
            descending: true,
        }
    }
}

/// The dashboard table's query parameters beyond the shared [`ViewFilter`]
/// (task 3.4): a selected category (parent or child), an optional month, a
/// case-insensitive text search, the two table toggles and the sort. With
/// [`Default`] the query is the plain browse table: no category, no month,
/// no search, hidden rows excluded, newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TxTableFilter {
    /// A category id, matched whether it is a parent (its children's rows)
    /// or a child (that child's rows).
    pub category_id: Option<i64>,
    /// A `YYYY-MM` month, matched against the start of `posted_date`.
    pub month: Option<String>,
    /// Case-insensitive substring matched against `description_raw` or
    /// `merchant`; empty or `None` matches everything.
    pub search: Option<String>,
    /// Restrict to uncategorized rows (`category_id IS NULL`).
    pub uncategorized_only: bool,
    /// Include hidden rows; when false they are left out.
    pub show_hidden: bool,
    pub sort: TxSort,
}

/// Browse-table rows for the dashboard (tasks 3.3–3.4): the shared
/// [`ViewFilter`] (date range, accounts, transfers) plus the table-only
/// [`TxTableFilter`] (category, month, search, uncategorized-only, show
/// hidden and sort). Hidden rows are shown only with
/// [`TxTableFilter::show_hidden`]; the search is a case-insensitive
/// substring of `description_raw` or `merchant` as stored; a category
/// selection matches that category and, if it is a parent, its children.
/// The default filter is the plain browse table, newest first.
pub fn list_transactions_filtered(
    conn: &Connection,
    filter: &ViewFilter,
    table: &TxTableFilter,
) -> Result<Vec<TransactionRow>> {
    macro_rules! arg {
        ($args:expr, $value:expr) => {{
            let placeholder = format!("?{}", $args.len() + 1);
            $args.push($value);
            placeholder
        }};
    }

    let mut sql = String::from(
        "SELECT t.id, i.name || ' · ' || a.name, t.posted_date, t.description_raw, t.merchant,
                c.name, t.amount_cents, t.category_id, t.hidden
           FROM transactions t
           JOIN accounts a ON a.id = t.account_id
           JOIN institutions i ON i.id = a.institution_id
           LEFT JOIN categories c ON c.id = t.category_id
           LEFT JOIN categories p ON p.id = c.parent_id
           WHERE 1 = 1",
    );
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();

    // Inclusive date range from the shared filter.
    let from = arg!(args, Box::new(filter.from.format("%Y-%m-%d").to_string()));
    let to = arg!(args, Box::new(filter.to.format("%Y-%m-%d").to_string()));
    sql.push_str(&format!(
        " AND t.posted_date >= {from} AND t.posted_date <= {to}"
    ));

    // Accounts (empty means every account, the ViewFilter convention).
    if !filter.account_ids.is_empty() {
        let marks: Vec<String> = filter
            .account_ids
            .iter()
            .map(|id| arg!(args, Box::new(*id)))
            .collect();
        sql.push_str(" AND t.account_id IN (");
        sql.push_str(&marks.join(", "));
        sql.push(')');
    }
    if !filter.include_transfers {
        sql.push_str(" AND COALESCE(c.kind, 'expense') <> 'transfer'");
    }

    // A category selection matches that category, or a parent's children.
    if let Some(cat) = table.category_id {
        let a = arg!(args, Box::new(cat));
        let b = arg!(args, Box::new(cat));
        sql.push_str(&format!(" AND (t.category_id = {a} OR c.parent_id = {b})"));
    }
    if let Some(month) = table.month.as_ref().filter(|m| !m.is_empty()) {
        let m = arg!(args, Box::new(month.clone()));
        sql.push_str(&format!(" AND substr(t.posted_date, 1, 7) = {m}"));
    }
    if table.uncategorized_only {
        sql.push_str(" AND t.category_id IS NULL");
    }
    // Case-insensitive substring on the raw stored strings.
    if let Some(term) = table.search.as_ref().filter(|s| !s.is_empty()) {
        let term = term.to_lowercase();
        let a = arg!(args, Box::new(term.clone()));
        let b = arg!(args, Box::new(term));
        sql.push_str(&format!(
            " AND (instr(lower(t.description_raw), {a}) > 0 \
               OR instr(lower(COALESCE(t.merchant, '')), {b}) > 0)"
        ));
    }
    if !table.show_hidden {
        sql.push_str(" AND t.hidden = 0");
    }

    let order = match table.sort {
        TxSort {
            column: TxSortColumn::Date,
            descending: true,
        } => "t.posted_date DESC, t.import_id DESC, t.source_line ASC",
        TxSort {
            column: TxSortColumn::Date,
            descending: false,
        } => "t.posted_date ASC, t.import_id ASC, t.source_line DESC",
        TxSort {
            column: TxSortColumn::Amount,
            descending: true,
        } => "t.amount_cents DESC, t.posted_date DESC, t.import_id DESC, t.source_line ASC",
        TxSort {
            column: TxSortColumn::Amount,
            descending: false,
        } => "t.amount_cents ASC, t.posted_date DESC, t.import_id DESC, t.source_line ASC",
    };
    sql.push_str(&format!(" ORDER BY {order}"));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args), |r| {
        Ok(TransactionRow {
            id: r.get(0)?,
            account: r.get(1)?,
            posted_date: r.get(2)?,
            description: r.get(3)?,
            merchant: r.get(4)?,
            category: r.get(5)?,
            amount_cents: r.get(6)?,
            category_id: r.get(7)?,
            hidden: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// One row of [`spend_by_category`]: a parent category (or the
/// `Uncategorized` pseudo-category, `id: None`, in the top-level view), its
/// spend in cents (positive, refunds netted) and its share of the total of
/// the rows returned under the same filter.
#[derive(Clone, Debug, PartialEq)]
pub struct CategorySpend {
    pub id: Option<i64>,
    pub name: String,
    pub spend_cents: i64,
    pub share: f64,
}

/// One row of [`MonthlySummary`]. `months` is aligned with
/// [`MonthlySummary::months`]; `total` is the row total over those months.
/// `id` is `None` for the pseudo rows (`Uncategorized`, `Income`, `Spend`,
/// `Net`).
#[derive(Clone, Debug, PartialEq)]
pub struct MonthlySummaryRow {
    pub id: Option<i64>,
    pub name: String,
    pub months: Vec<i64>,
    pub total: i64,
}

/// The monthly grid: every month between [`ViewFilter::from`] and
/// [`ViewFilter::to`] inclusive (`YYYY-MM`, ascending; zero columns for
/// empty months), clamped to the months the data spans when the range is
/// wider than the summary cap (an open-ended filter like [`Default`]
/// therefore shows only months with data). Then rows = one row per spend
/// parent category (plus `Uncategorized`, plus `Transfers` when the filter
/// includes transfers), each followed by its children, and finally the
/// `Income`, `Spend` and `Net` rows (`Net = Income - Spend`; transfers count
/// in `Spend` and `Net` when the filter includes them).
#[derive(Clone, Debug, PartialEq)]
pub struct MonthlySummary {
    pub months: Vec<String>,
    pub rows: Vec<MonthlySummaryRow>,
}

const UNCATEGORIZED: &str = "Uncategorized";
const INCOME: &str = "Income";
const SPEND: &str = "Spend";
const NET: &str = "Net";

/// Upper bound for the month grid; a filter wider than this (e.g. the
/// open-ended [`Default`]) is clamped to the months the data spans.
const MAX_SUMMARY_MONTHS: i64 = 1200;

/// Spend per parent category, or per child of `parent`.
///
/// With `None`: one row per parent category that has matching rows, plus a
/// single `Uncategorized` row if any uncategorized row matches. With
/// `Some(parent)`: one row per child of that parent with matching rows
/// (uncategorized rows belong to no child). Only spend-bucket rows count
/// (see the module docs); income parents therefore return no rows.
///
/// Sorted by spend descending, name ascending on ties. `share` is the row's
/// spend over the total spend of the rows returned — the overall total in
/// the parent view, the parent's own total in the child view — so the
/// shares of one call sum to 1.0, or are all `0.0` when the total is zero.
pub fn spend_by_category(
    conn: &Connection,
    filter: &ViewFilter,
    parent: Option<i64>,
) -> Result<Vec<CategorySpend>> {
    let agg = aggregate(conn, filter)?;
    let mut rows: Vec<(Option<i64>, String, i64)> = match parent {
        None => {
            let mut rows: Vec<_> = agg
                .parents
                .iter()
                .map(|(id, p)| (Some(*id), p.name.clone(), net_spend(&p.months)))
                .collect();
            if !agg.uncategorized.is_empty() {
                let cents = net_spend(&agg.uncategorized);
                rows.push((None, UNCATEGORIZED.to_string(), cents));
            }
            rows
        }
        Some(pid) => agg
            .parents
            .get(&pid)
            .map(|p| {
                p.children
                    .iter()
                    .map(|(id, c)| (Some(*id), c.name.clone(), net_spend(&c.months)))
                    .collect()
            })
            .unwrap_or_default(),
    };
    sort_rows(&mut rows);
    let total: i64 = rows.iter().map(|r| r.2).sum();
    Ok(rows
        .into_iter()
        .map(|(id, name, spend_cents)| CategorySpend {
            id,
            name,
            spend_cents,
            share: if total == 0 {
                0.0
            } else {
                spend_cents as f64 / total as f64
            },
        })
        .collect())
}

/// The monthly summary grid described on [`MonthlySummary`]. Parent and
/// child rows are sorted by total spend descending (name ascending on
/// ties) within the grid; `Uncategorized` sorts with the parents.
pub fn monthly_summary(conn: &Connection, filter: &ViewFilter) -> Result<MonthlySummary> {
    let agg = aggregate(conn, filter)?;
    let months = month_grid(filter, &agg);

    // (id, name, month values, total, child rows) per displayed parent.
    type ParentEntry = (Option<i64>, String, Vec<i64>, i64, Vec<MonthlySummaryRow>);
    let mut parents: Vec<ParentEntry> = agg
        .parents
        .iter()
        .map(|(id, p)| {
            let (values, total) = row_values(&months, &p.months, true);
            let mut children: Vec<(i64, &str, &BTreeMap<String, i64>)> = p
                .children
                .iter()
                .map(|(cid, c)| (*cid, c.name.as_str(), &c.months))
                .collect();
            children.sort_by(|a, b| net_spend(b.2).cmp(&net_spend(a.2)).then(a.1.cmp(b.1)));
            let children = children
                .into_iter()
                .map(|(cid, name, map)| {
                    let (values, total) = row_values(&months, map, true);
                    MonthlySummaryRow {
                        id: Some(cid),
                        name: name.to_string(),
                        months: values,
                        total,
                    }
                })
                .collect();
            (Some(*id), p.name.clone(), values, total, children)
        })
        .collect();
    if !agg.uncategorized.is_empty() {
        let (values, total) = row_values(&months, &agg.uncategorized, true);
        parents.push((None, UNCATEGORIZED.to_string(), values, total, Vec::new()));
    }
    parents.sort_by(|a, b| b.3.cmp(&a.3).then(a.1.cmp(&b.1)));

    let mut rows = Vec::new();
    for (id, name, months_values, total, children) in parents {
        rows.push(MonthlySummaryRow {
            id,
            name,
            months: months_values,
            total,
        });
        rows.extend(children);
    }

    let (income_values, income_total) = row_values(&months, &agg.income, false);
    let (spend_values, spend_total) = row_values(&months, &agg.spend, true);
    let net_months: Vec<i64> = income_values
        .iter()
        .zip(&spend_values)
        .map(|(i, s)| i - s)
        .collect();
    let net_total = income_total - spend_total;
    rows.push(MonthlySummaryRow {
        id: None,
        name: INCOME.to_string(),
        months: income_values,
        total: income_total,
    });
    rows.push(MonthlySummaryRow {
        id: None,
        name: SPEND.to_string(),
        months: spend_values,
        total: spend_total,
    });
    rows.push(MonthlySummaryRow {
        id: None,
        name: NET.to_string(),
        months: net_months,
        total: net_total,
    });
    Ok(MonthlySummary { months, rows })
}

/// One grouped row of the filtered transactions: raw `SUM(amount_cents)`
/// per `(kind, parent, child, month)`.
struct GroupedRow {
    kind: String,
    parent_id: Option<i64>,
    parent_name: Option<String>,
    child_id: Option<i64>,
    child_name: Option<String>,
    month: String,
    amount: i64,
}

fn grouped_rows(conn: &Connection, filter: &ViewFilter) -> Result<Vec<GroupedRow>> {
    let mut sql = String::from(
        "SELECT COALESCE(c.kind, 'expense'), p.id, p.name, c.id, c.name,
                substr(t.posted_date, 1, 7), SUM(t.amount_cents)
           FROM transactions t
           LEFT JOIN categories c ON c.id = t.category_id
           LEFT JOIN categories p ON p.id = c.parent_id
          WHERE t.hidden = 0 AND t.posted_date >= ?1 AND t.posted_date <= ?2",
    );
    let mut args: Vec<Box<dyn ToSql>> = vec![
        Box::new(filter.from.format("%Y-%m-%d").to_string()),
        Box::new(filter.to.format("%Y-%m-%d").to_string()),
    ];
    if !filter.account_ids.is_empty() {
        let marks: Vec<String> = (0..filter.account_ids.len())
            .map(|i| format!("?{}", i + 3))
            .collect();
        sql.push_str(" AND t.account_id IN (");
        sql.push_str(&marks.join(", "));
        sql.push(')');
        args.extend(
            filter
                .account_ids
                .iter()
                .map(|id| Box::new(*id) as Box<dyn ToSql>),
        );
    }
    if !filter.include_transfers {
        sql.push_str(" AND COALESCE(c.kind, 'expense') <> 'transfer'");
    }
    sql.push_str(" GROUP BY 1, 2, 3, 4, 5, 6");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args), |r| {
        Ok(GroupedRow {
            kind: r.get(0)?,
            parent_id: r.get(1)?,
            parent_name: r.get(2)?,
            child_id: r.get(3)?,
            child_name: r.get(4)?,
            month: r.get(5)?,
            amount: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[derive(Default)]
struct ChildAgg {
    name: String,
    months: BTreeMap<String, i64>,
}

#[derive(Default)]
struct ParentAgg {
    name: String,
    months: BTreeMap<String, i64>,
    children: BTreeMap<i64, ChildAgg>,
}

/// Raw `SUM(amount_cents)` per bucket; the spend buckets (`parents`,
/// `uncategorized`, `spend`) hold money-out as *negative* totals and are
/// negated for display. `spend` covers every spend-bucket row (parents,
/// children, uncategorized and — when the filter includes them —
/// transfers), `income` only income rows.
#[derive(Default)]
struct Aggregate {
    parents: BTreeMap<i64, ParentAgg>,
    uncategorized: BTreeMap<String, i64>,
    income: BTreeMap<String, i64>,
    spend: BTreeMap<String, i64>,
}

fn aggregate(conn: &Connection, filter: &ViewFilter) -> Result<Aggregate> {
    let mut agg = Aggregate::default();
    for row in grouped_rows(conn, filter)? {
        if row.kind == "income" {
            *agg.income.entry(row.month).or_default() += row.amount;
            continue;
        }
        *agg.spend.entry(row.month.clone()).or_default() += row.amount;
        let Some(child_id) = row.child_id else {
            *agg.uncategorized.entry(row.month).or_default() += row.amount;
            continue;
        };
        let (parent_id, parent_name) = match (&row.parent_id, &row.parent_name) {
            (Some(id), Some(name)) => (*id, name.clone()),
            // Defensive: transactions normally hang off children only
            // (`ensure_assignable` refuses parents); treat a row filed
            // directly under a parent as its own parent row.
            _ => (child_id, row.child_name.clone().unwrap_or_default()),
        };
        let parent = agg.parents.entry(parent_id).or_insert_with(|| ParentAgg {
            name: parent_name,
            ..Default::default()
        });
        *parent.months.entry(row.month.clone()).or_default() += row.amount;
        if row.parent_id.is_some() {
            let child = parent.children.entry(child_id).or_insert_with(|| ChildAgg {
                name: row.child_name.clone().unwrap_or_default(),
                ..Default::default()
            });
            *child.months.entry(row.month).or_default() += row.amount;
        }
    }
    Ok(agg)
}

/// Months for the grid: every month between `from` and `to` inclusive. A
/// range wider than [`MAX_SUMMARY_MONTHS`] (an open-ended filter such as
/// [`ViewFilter::default`]) is clamped to the months the filtered data
/// spans, so the grid stays a sane size; no data means no months.
fn month_grid(filter: &ViewFilter, agg: &Aggregate) -> Vec<String> {
    let mut first = month_index(filter.from.year(), filter.from.month());
    let mut last = month_index(filter.to.year(), filter.to.month());
    if last < first {
        return Vec::new();
    }
    if last - first + 1 > MAX_SUMMARY_MONTHS {
        let Some((lo, hi)) = data_span(agg) else {
            return Vec::new();
        };
        first = first.max(lo);
        last = last.min(hi);
    }
    (first..=last).map(month_label).collect()
}

fn data_span(agg: &Aggregate) -> Option<(i64, i64)> {
    let mut span: Option<(i64, i64)> = None;
    let mut extend = |key: &str| {
        let Some(ix) = parse_month_index(key) else {
            return;
        };
        span = Some(match span {
            Some((lo, hi)) => (lo.min(ix), hi.max(ix)),
            None => (ix, ix),
        });
    };
    for key in agg.spend.keys().chain(agg.income.keys()) {
        extend(key);
    }
    span
}

fn month_index(year: i32, month: u32) -> i64 {
    year as i64 * 12 + month as i64 - 1
}

fn parse_month_index(key: &str) -> Option<i64> {
    let year: i32 = key.get(..4)?.parse().ok()?;
    let month: u32 = key.get(5..7)?.parse().ok()?;
    if (1..=12).contains(&month) {
        Some(month_index(year, month))
    } else {
        None
    }
}

fn month_label(index: i64) -> String {
    let year = index.div_euclid(12);
    let month = index.rem_euclid(12) + 1;
    format!("{year:04}-{month:02}")
}

/// Aligned per-month display values and the row total. `negate` turns the
/// raw `SUM(amount_cents)` of a spend bucket into a positive spend.
fn row_values(months: &[String], map: &BTreeMap<String, i64>, negate: bool) -> (Vec<i64>, i64) {
    let mut values = Vec::with_capacity(months.len());
    let mut total = 0;
    for month in months {
        let v = *map.get(month).unwrap_or(&0);
        let v = if negate { -v } else { v };
        total += v;
        values.push(v);
    }
    (values, total)
}

fn net_spend(months: &BTreeMap<String, i64>) -> i64 {
    -months.values().sum::<i64>()
}

fn sort_rows(rows: &mut [(Option<i64>, String, i64)]) {
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{card_payment_category, create_child, create_parent};
    use crate::import::{CommitOptions, commit_import};
    use crate::overrides::set_hidden;
    use crate::{create_account, open_in_memory, spend_by_parent_category};
    use financi_core::{AccountKind, CategoryKind, RawFile, read_raw_file};
    use rusqlite::Connection;

    fn account(conn: &Connection, key: &str) -> i64 {
        let kind = match key {
            "simplii" => AccountKind::Chequing,
            _ => AccountKind::CreditCard,
        };
        create_account(conn, key, &format!("acct-{key}"), kind).unwrap()
    }

    fn demo_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")
    }

    fn import_bytes(
        conn: &mut Connection,
        account_id: i64,
        parser_id: &str,
        bytes: &[u8],
        name: &str,
    ) {
        let file: RawFile = read_raw_file(bytes).unwrap();
        let parser = financi_parsers::by_id(parser_id).unwrap();
        commit_import(
            conn,
            account_id,
            parser.as_ref(),
            &file,
            name,
            "2026-11-07T12:00:00",
            CommitOptions::default(),
        )
        .unwrap();
    }

    fn seed_import(conn: &Connection, account_id: i64) -> i64 {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
            [account_id],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[allow(clippy::too_many_arguments)]
    fn add_txn(
        conn: &Connection,
        account_id: i64,
        import_id: i64,
        date: &str,
        cents: i64,
        category: Option<i64>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                        row_hash, posted_date, amount_cents,
                                        description_raw, category_id)
             VALUES (?1, ?2, 0, X'00', ?3, ?4, ?5, 'ROW', ?6)",
            (
                account_id,
                import_id,
                uuid::Uuid::new_v4().to_string(),
                date,
                cents,
                category,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn range(from: &str, to: &str, accounts: &[i64], transfers: bool) -> ViewFilter {
        ViewFilter {
            from: NaiveDate::parse_from_str(from, "%Y-%m-%d").unwrap(),
            to: NaiveDate::parse_from_str(to, "%Y-%m-%d").unwrap(),
            account_ids: accounts.to_vec(),
            include_transfers: transfers,
        }
    }

    fn named<'a>(summary: &'a MonthlySummary, name: &str) -> &'a MonthlySummaryRow {
        summary
            .rows
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no row named {name:?} in {:?}", summary.rows))
    }

    fn transfers_parent(conn: &Connection) -> i64 {
        let card = card_payment_category(conn).unwrap();
        conn.query_row(
            "SELECT parent_id FROM categories WHERE id = ?1",
            [card],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    }

    #[test]
    fn demo_months_match_expected_json() {
        let path = demo_dir().join("expected.json");
        let expected: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for (file, parser, inst) in [
            ("amex-sep-oct.csv", "amex", "amex"),
            ("amex-sep-nov.csv", "amex", "amex"),
            ("simplii-chequing.csv", "simplii-chequing", "simplii"),
        ] {
            let mut conn = open_in_memory().unwrap();
            let acct = account(&conn, inst);
            let bytes = std::fs::read(demo_dir().join(file)).unwrap();
            import_bytes(&mut conn, acct, parser, &bytes, file);

            let mut want_months: Vec<String> = expected[file]["months"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            want_months.sort();
            let summary = monthly_summary(
                &conn,
                &ViewFilter {
                    include_transfers: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(summary.months, want_months, "{file}: month grid");

            let spend = named(&summary, "Spend");
            let income = named(&summary, "Income");
            let net = named(&summary, "Net");
            let uncategorized = named(&summary, "Uncategorized");
            let transfers = summary.rows.iter().find(|r| r.name == "Transfers");
            for (i, month) in summary.months.iter().enumerate() {
                let totals = &expected[file]["months"][month];
                let out = totals["money_out_cents"].as_i64().unwrap();
                let inn = totals["money_in_cents"].as_i64().unwrap();
                assert_eq!(spend.months[i], out - inn, "{file} {month}: spend");
                assert_eq!(income.months[i], 0, "{file} {month}: no categories");
                assert_eq!(net.months[i], inn - out, "{file} {month}: net");
                // With no categories every row but the card payments the
                // import detected lands in Uncategorized.
                let transfers_cents = transfers.map_or(0, |t| t.months[i]);
                assert_eq!(
                    uncategorized.months[i] + transfers_cents,
                    spend.months[i],
                    "{file} {month}: uncategorized + transfers = spend"
                );
                if parser == "amex" {
                    // Every money-in row in the Amex export is a card payment.
                    assert_eq!(transfers_cents, -inn, "{file} {month}: transfers");
                } else {
                    // The Simplii parser detects no transfers, so the bill
                    // pay rows are plain uncategorized spend.
                    assert!(transfers.is_none(), "{file} {month}: no transfers");
                    assert_eq!(uncategorized.months[i], out - inn, "{file} {month}");
                }
            }

            // With transfers excluded the card payments drop out of spend.
            let closed = monthly_summary(&conn, &ViewFilter::default()).unwrap();
            let closed_spend = named(&closed, "Spend");
            assert!(
                closed.rows.iter().all(|r| r.name != "Transfers"),
                "{file}: transfers row gone when excluded"
            );
            for (i, month) in closed.months.iter().enumerate() {
                let totals = &expected[file]["months"][month];
                let out = totals["money_out_cents"].as_i64().unwrap();
                let inn = totals["money_in_cents"].as_i64().unwrap();
                let want = if parser == "amex" { out } else { out - inn };
                assert_eq!(closed_spend.months[i], want, "{file} {month}: spend");
            }
        }
    }

    /// Expense categories, refunds, income, a transfer and a hidden row,
    /// laid out for every test below.
    struct Fixture {
        conn: Connection,
        account_id: i64,
        other_account_id: i64,
        import_id: i64,
        groceries: i64,
        dining: i64,
        payroll: i64,
        card_payment: i64,
    }

    fn fixture() -> Fixture {
        let conn = open_in_memory().unwrap();
        let account_id = account(&conn, "amex");
        let other_account_id = account(&conn, "simplii");
        let import_id = seed_import(&conn, account_id);
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        let dining = create_child(&conn, food, "Dining").unwrap();
        let payroll_parent = create_parent(&conn, "Income", CategoryKind::Income).unwrap();
        let payroll = create_child(&conn, payroll_parent, "Payroll").unwrap();
        let card_payment = card_payment_category(&conn).unwrap();
        Fixture {
            conn,
            account_id,
            other_account_id,
            import_id,
            groceries,
            dining,
            payroll,
            card_payment,
        }
    }

    #[test]
    fn default_filter_agrees_with_spend_by_parent_category() {
        let f = fixture();
        let add = |conn: &Connection, date, cents, cat| {
            add_txn(conn, f.account_id, f.import_id, date, cents, cat)
        };
        add(&f.conn, "2026-10-05", -5000, Some(f.groceries));
        add(&f.conn, "2026-10-06", 1200, Some(f.groceries)); // refund
        add(&f.conn, "2026-10-07", -3000, Some(f.dining));
        add(&f.conn, "2026-10-08", -700, None);
        add(&f.conn, "2026-10-09", 200, None); // uncategorized refund
        add(&f.conn, "2026-10-10", 450_000, Some(f.payroll)); // income
        let hidden = add(&f.conn, "2026-10-11", -9999, Some(f.dining));
        set_hidden(&f.conn, hidden, true).unwrap();
        add(&f.conn, "2026-10-12", -150_000, Some(f.card_payment));

        let mine: Vec<(String, i64)> = spend_by_category(&f.conn, &ViewFilter::default(), None)
            .unwrap()
            .into_iter()
            .map(|r| (r.name, r.spend_cents))
            .collect();
        assert_eq!(
            mine,
            vec![
                ("Food".to_string(), 6800),
                ("Uncategorized".to_string(), 500)
            ],
            "refunds net against their category; income, transfers and hidden rows drop out"
        );
        let mut mine_sorted = mine;
        mine_sorted.sort();
        let mut theirs = spend_by_parent_category(&f.conn).unwrap();
        theirs.sort();
        assert_eq!(
            mine_sorted, theirs,
            "open filter matches the unfiltered query"
        );
    }

    #[test]
    fn refunds_net_against_their_category() {
        let f = fixture();
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-05",
            -5000,
            Some(f.groceries),
        );
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-06",
            1200,
            Some(f.groceries),
        );
        let rows = spend_by_category(&f.conn, &ViewFilter::default(), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spend_cents, 3800, "the refund nets into Groceries");
        assert_eq!(rows[0].share, 1.0);

        // A refund larger than the charges makes the category net negative.
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-07",
            9000,
            Some(f.groceries),
        );
        let rows = spend_by_category(&f.conn, &ViewFilter::default(), None).unwrap();
        assert_eq!(rows[0].spend_cents, -5200);
    }

    #[test]
    fn transfers_are_excluded_unless_requested() {
        let f = fixture();
        add_txn(&f.conn, f.account_id, f.import_id, "2026-10-05", -700, None);
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-06",
            -100_000,
            Some(f.card_payment),
        );
        let parent = transfers_parent(&f.conn);

        let closed = spend_by_category(&f.conn, &ViewFilter::default(), None).unwrap();
        assert_eq!(closed.len(), 1, "the transfer row is not shown");
        assert_eq!(closed[0].name, "Uncategorized");
        assert!(
            spend_by_category(&f.conn, &ViewFilter::default(), Some(parent))
                .unwrap()
                .is_empty(),
            "drilling into Transfers with the toggle off has no rows"
        );

        let open = ViewFilter {
            include_transfers: true,
            ..Default::default()
        };
        let rows = spend_by_category(&f.conn, &open, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Transfers");
        assert_eq!(rows[0].spend_cents, 100_000, "money out is positive spend");
        let children = spend_by_category(&f.conn, &open, Some(parent)).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Card Payment");
        assert_eq!(children[0].spend_cents, 100_000);

        let summary = monthly_summary(&f.conn, &open).unwrap();
        assert_eq!(
            named(&summary, "Spend").total,
            100_700,
            "the transfer counts in the Spend row when included"
        );
        assert_eq!(named(&summary, "Transfers").total, 100_000);
    }

    #[test]
    fn hidden_rows_are_excluded() {
        let f = fixture();
        let a = add_txn(&f.conn, f.account_id, f.import_id, "2026-10-05", -700, None);
        let b = add_txn(&f.conn, f.account_id, f.import_id, "2026-10-06", -300, None);
        let spend = |conn: &Connection| {
            spend_by_category(conn, &ViewFilter::default(), None)
                .unwrap()
                .iter()
                .map(|r| r.spend_cents)
                .sum::<i64>()
        };
        assert_eq!(spend(&f.conn), 1000);
        set_hidden(&f.conn, a, true).unwrap();
        assert_eq!(spend(&f.conn), 300);
        assert_eq!(
            named(
                &monthly_summary(&f.conn, &ViewFilter::default()).unwrap(),
                "Spend"
            )
            .total,
            300
        );
        set_hidden(&f.conn, a, false).unwrap();
        assert_eq!(spend(&f.conn), 1000);
        set_hidden(&f.conn, b, true).unwrap();
        assert_eq!(spend(&f.conn), 700, "only b is hidden now");
        set_hidden(&f.conn, a, true).unwrap();
        assert!(
            spend_by_category(&f.conn, &ViewFilter::default(), None)
                .unwrap()
                .is_empty(),
            "hiding every row leaves no Uncategorized row at all"
        );
    }

    #[test]
    fn account_filter_selects_accounts_empty_means_all() {
        let f = fixture();
        let other_import = seed_import(&f.conn, f.other_account_id);
        add_txn(&f.conn, f.account_id, f.import_id, "2026-10-05", -700, None);
        add_txn(
            &f.conn,
            f.other_account_id,
            other_import,
            "2026-10-06",
            -300,
            None,
        );
        let total = |filter: &ViewFilter| {
            spend_by_category(&f.conn, filter, None)
                .unwrap()
                .iter()
                .map(|r| r.spend_cents)
                .sum::<i64>()
        };
        assert_eq!(total(&ViewFilter::default()), 1000, "empty = all accounts");
        assert_eq!(
            total(&range("2026-01-01", "2026-12-31", &[f.account_id], false)),
            700
        );
        assert_eq!(
            total(&range(
                "2026-01-01",
                "2026-12-31",
                &[f.other_account_id],
                false
            )),
            300
        );
        assert_eq!(
            total(&range(
                "2026-01-01",
                "2026-12-31",
                &[f.account_id, f.other_account_id],
                false
            )),
            1000
        );
    }

    #[test]
    fn list_filtered_honours_accounts_and_transfers() {
        let f = fixture();
        let other_import = seed_import(&f.conn, f.other_account_id);
        let older = add_txn(&f.conn, f.account_id, f.import_id, "2026-10-05", -700, None);
        let other = add_txn(
            &f.conn,
            f.other_account_id,
            other_import,
            "2026-10-06",
            -300,
            None,
        );
        let transfer = add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-07",
            -5000,
            Some(f.card_payment),
        );
        let ids = |account_ids: &[i64], transfers: bool| -> Vec<i64> {
            let filter = ViewFilter {
                account_ids: account_ids.to_vec(),
                include_transfers: transfers,
                ..Default::default()
            };
            list_transactions_filtered(&f.conn, &filter, &TxTableFilter::default())
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect()
        };
        // Empty account ids mean every account; transfer rows are out.
        assert_eq!(ids(&[], false), vec![other, older], "newest first");
        assert_eq!(ids(&[], true), vec![transfer, other, older]);
        // Account selection narrows to that account's rows.
        assert_eq!(ids(&[f.account_id], false), vec![older]);
        assert_eq!(ids(&[f.other_account_id], false), vec![other]);
        assert_eq!(
            ids(&[f.account_id, f.other_account_id], false),
            vec![other, older]
        );
        // Hidden rows never appear, however the filter is set.
        set_hidden(&f.conn, older, true).unwrap();
        assert_eq!(ids(&[], true), vec![transfer, other]);
    }

    /// A transaction with its own description and merchant for the table's
    /// search, month and category tests.
    #[allow(clippy::too_many_arguments)]
    fn add_row(
        conn: &Connection,
        account_id: i64,
        import_id: i64,
        source_line: i64,
        date: &str,
        cents: i64,
        category: Option<i64>,
        description: &str,
        merchant: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                        row_hash, posted_date, amount_cents,
                                        description_raw, merchant, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                account_id,
                import_id,
                source_line,
                uuid::Uuid::new_v4().to_string(),
                date,
                cents,
                description,
                merchant,
                category,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn table_ids(conn: &Connection, table: &TxTableFilter) -> Vec<i64> {
        list_transactions_filtered(conn, &ViewFilter::default(), table)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    #[test]
    fn date_range_bounds_are_inclusive_in_the_table() {
        let f = fixture();
        let out_before = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            0,
            "2026-01-31",
            -100,
            None,
            "before",
            None,
        );
        let in_first = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            1,
            "2026-02-01",
            -200,
            None,
            "first",
            None,
        );
        let in_last = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            2,
            "2026-02-28",
            -300,
            None,
            "last",
            None,
        );
        let out_after = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            3,
            "2026-03-01",
            -400,
            None,
            "after",
            None,
        );
        let _ = (out_before, out_after);
        let filter = ViewFilter {
            from: NaiveDate::parse_from_str("2026-02-01", "%Y-%m-%d").unwrap(),
            to: NaiveDate::parse_from_str("2026-02-28", "%Y-%m-%d").unwrap(),
            ..Default::default()
        };
        let ids = list_transactions_filtered(&f.conn, &filter, &TxTableFilter::default())
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![in_last, in_first], "both bounds in, newest first");
    }

    #[test]
    fn month_filter_matches_the_posted_month() {
        let f = fixture();
        let sep = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            0,
            "2026-09-30",
            -100,
            None,
            "sep",
            None,
        );
        let oct_first = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            1,
            "2026-10-01",
            -200,
            None,
            "oct1",
            None,
        );
        let oct_last = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            2,
            "2026-10-31",
            -300,
            None,
            "oct31",
            None,
        );
        let nov = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            3,
            "2026-11-01",
            -400,
            None,
            "nov",
            None,
        );
        let _ = (sep, nov);
        let oct = table_ids(
            &f.conn,
            &TxTableFilter {
                month: Some("2026-10".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(oct, vec![oct_last, oct_first], "whole month, newest first");
    }

    #[test]
    fn category_filter_matches_parent_and_child() {
        let f = fixture();
        let food = conn_food(&f.conn);
        let groceries = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            0,
            "2026-10-01",
            -100,
            Some(f.groceries),
            "g",
            None,
        );
        let dining = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            1,
            "2026-10-02",
            -200,
            Some(f.dining),
            "d",
            None,
        );
        let plain = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            2,
            "2026-10-03",
            -300,
            None,
            "plain",
            None,
        );
        let _ = plain;

        let by_child = table_ids(
            &f.conn,
            &TxTableFilter {
                category_id: Some(f.groceries),
                ..Default::default()
            },
        );
        assert_eq!(by_child, vec![groceries], "a child selects only its rows");

        let by_parent = table_ids(
            &f.conn,
            &TxTableFilter {
                category_id: Some(food),
                ..Default::default()
            },
        );
        assert_eq!(
            by_parent,
            vec![dining, groceries],
            "a parent selects its children, not uncategorized"
        );
    }

    #[test]
    fn uncategorized_only_filter() {
        let f = fixture();
        let groceries = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            0,
            "2026-10-01",
            -100,
            Some(f.groceries),
            "g",
            None,
        );
        let a = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            1,
            "2026-10-02",
            -200,
            None,
            "a",
            None,
        );
        let b = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            2,
            "2026-10-03",
            -300,
            None,
            "b",
            None,
        );
        let _ = groceries;
        let only = table_ids(
            &f.conn,
            &TxTableFilter {
                uncategorized_only: true,
                ..Default::default()
            },
        );
        assert_eq!(only, vec![b, a]);
    }

    #[test]
    fn search_is_case_insensitive_on_description_and_merchant() {
        let f = fixture();
        let cafe = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            0,
            "2026-10-01",
            -100,
            None,
            "CORNER CAFE",
            Some("Corner Cafe"),
        );
        let store = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            1,
            "2026-10-02",
            -200,
            None,
            "GROCER STORE",
            Some("Fresh Mart"),
        );
        let _payroll = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            2,
            "2026-10-03",
            999,
            None,
            "PAYROLL DEPOSIT",
            None,
        );

        let search = |term: &str| {
            table_ids(
                &f.conn,
                &TxTableFilter {
                    search: Some(term.to_string()),
                    ..Default::default()
                },
            )
        };
        assert_eq!(
            search("cafe"),
            vec![cafe],
            "matches description, case-insensitive"
        );
        assert_eq!(search("CORNER"), vec![cafe]);
        assert_eq!(search("fresh"), vec![store], "matches the merchant column");
        assert_eq!(
            search("grocer store"),
            vec![store],
            "substring across words"
        );
        let all = table_ids(&f.conn, &TxTableFilter::default());
        assert_eq!(all.len(), 3, "no search term matches everything");
    }

    #[test]
    fn show_hidden_toggle() {
        let f = fixture();
        let shown = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            0,
            "2026-10-01",
            -100,
            None,
            "shown",
            None,
        );
        let hidden = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            1,
            "2026-10-02",
            -200,
            None,
            "hidden",
            None,
        );
        set_hidden(&f.conn, hidden, true).unwrap();

        let off = table_ids(&f.conn, &TxTableFilter::default());
        assert_eq!(off, vec![shown], "hidden rows are out by default");
        let on = table_ids(
            &f.conn,
            &TxTableFilter {
                show_hidden: true,
                ..Default::default()
            },
        );
        assert_eq!(
            on,
            vec![hidden, shown],
            "show hidden brings them back, newest first"
        );
    }

    #[test]
    fn sort_by_date_and_amount() {
        let f = fixture();
        let small = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            0,
            "2026-10-01",
            -100,
            None,
            "small",
            None,
        );
        let refund = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            1,
            "2026-10-02",
            250,
            None,
            "refund",
            None,
        );
        let big = add_row(
            &f.conn,
            f.account_id,
            f.import_id,
            2,
            "2026-10-03",
            -900,
            None,
            "big",
            None,
        );
        let _ = refund;

        let sort = |column: TxSortColumn, descending: bool| {
            table_ids(
                &f.conn,
                &TxTableFilter {
                    sort: TxSort { column, descending },
                    ..Default::default()
                },
            )
        };
        assert_eq!(sort(TxSortColumn::Date, true), vec![big, refund, small]);
        assert_eq!(sort(TxSortColumn::Date, false), vec![small, refund, big]);
        assert_eq!(
            sort(TxSortColumn::Amount, true),
            vec![refund, small, big],
            "amount descending: money in (positive) first, then the smaller spend"
        );
        assert_eq!(
            sort(TxSortColumn::Amount, false),
            vec![big, small, refund],
            "amount ascending: the biggest money-out first"
        );
    }

    #[test]
    fn date_bounds_are_inclusive() {
        let f = fixture();
        add_txn(&f.conn, f.account_id, f.import_id, "2026-01-31", -100, None);
        add_txn(&f.conn, f.account_id, f.import_id, "2026-02-01", -200, None);
        add_txn(&f.conn, f.account_id, f.import_id, "2026-02-28", -300, None);
        add_txn(&f.conn, f.account_id, f.import_id, "2026-03-01", -400, None);
        let filter = range("2026-02-01", "2026-02-28", &[], false);
        let rows = spend_by_category(&f.conn, &filter, None).unwrap();
        assert_eq!(rows.iter().map(|r| r.spend_cents).sum::<i64>(), 500);
        let summary = monthly_summary(&f.conn, &filter).unwrap();
        assert_eq!(summary.months, ["2026-02"]);
        assert_eq!(named(&summary, "Spend").months, [500]);
    }

    #[test]
    fn shares_sum_to_one_in_both_views() {
        let f = fixture();
        let fun = create_parent(&f.conn, "Fun", CategoryKind::Expense).unwrap();
        let games = create_child(&f.conn, fun, "Games").unwrap();
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-05",
            -4000,
            Some(f.groceries),
        );
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-06",
            -3000,
            Some(f.dining),
        );
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-07",
            -3000,
            Some(games),
        );

        let rows = spend_by_category(&f.conn, &ViewFilter::default(), None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].name.as_str(), rows[0].spend_cents), ("Food", 7000));
        assert!((rows[0].share - 0.7).abs() < 1e-9);
        assert!((rows[1].share - 0.3).abs() < 1e-9);
        assert!((rows.iter().map(|r| r.share).sum::<f64>() - 1.0).abs() < 1e-9);

        let children =
            spend_by_category(&f.conn, &ViewFilter::default(), Some(f.groceries)).unwrap();
        assert!(children.is_empty(), "Groceries is a child, not a parent");
        let food = conn_food(&f.conn);
        let children = spend_by_category(&f.conn, &ViewFilter::default(), Some(food)).unwrap();
        assert_eq!(
            children
                .iter()
                .map(|r| (r.name.as_str(), r.spend_cents))
                .collect::<Vec<_>>(),
            vec![("Groceries", 4000), ("Dining", 3000)],
            "children sorted by spend descending, uncategorized rows left out"
        );
        assert!((children.iter().map(|r| r.share).sum::<f64>() - 1.0).abs() < 1e-9);
    }

    fn conn_food(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT id FROM categories WHERE name = 'Food' AND parent_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn zero_total_gives_zero_shares() {
        let f = fixture();
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-05",
            -5000,
            Some(f.groceries),
        );
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-10-06",
            5000,
            Some(f.groceries),
        );
        let rows = spend_by_category(&f.conn, &ViewFilter::default(), None).unwrap();
        assert_eq!(rows.len(), 1, "a netted-to-zero category still has a row");
        assert_eq!((rows[0].spend_cents, rows[0].share), (0, 0.0));
    }

    #[test]
    fn monthly_summary_grid_children_and_net_rows() {
        let f = fixture();
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-01-15",
            -1000,
            Some(f.groceries),
        );
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-03-20",
            -3000,
            Some(f.groceries),
        );
        add_txn(
            &f.conn,
            f.account_id,
            f.import_id,
            "2026-02-14",
            45_000,
            Some(f.payroll),
        );
        let filter = range("2026-01-01", "2026-03-31", &[], false);
        let summary = monthly_summary(&f.conn, &filter).unwrap();
        assert_eq!(summary.months, ["2026-01", "2026-02", "2026-03"]);

        let food = named(&summary, "Food");
        assert_eq!(food.id, Some(conn_food(&f.conn)));
        assert_eq!(
            food.months,
            [1000, 0, 3000],
            "empty February is a zero column"
        );
        assert_eq!(food.total, 4000);
        let groceries = named(&summary, "Groceries");
        assert_eq!(groceries.id, Some(f.groceries));
        assert_eq!(groceries.months, [1000, 0, 3000]);
        // child rows sit directly under their parent
        let names: Vec<&str> = summary.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Food", "Groceries", "Income", "Spend", "Net"]);
        assert_eq!(named(&summary, "Income").months, [0, 45_000, 0]);
        assert_eq!(named(&summary, "Income").id, None);
        assert_eq!(named(&summary, "Spend").months, [1000, 0, 3000]);
        let net = named(&summary, "Net");
        assert_eq!(net.months, [-1000, 45_000, -3000], "Net = Income - Spend");
        assert_eq!(net.total, 41_000);
    }

    #[test]
    fn monthly_summary_open_filter_clamps_to_data_months() {
        let f = fixture();
        add_txn(&f.conn, f.account_id, f.import_id, "2026-09-30", -100, None);
        add_txn(&f.conn, f.account_id, f.import_id, "2026-11-01", -200, None);
        let summary = monthly_summary(&f.conn, &ViewFilter::default()).unwrap();
        assert_eq!(summary.months, ["2026-09", "2026-10", "2026-11"]);
        assert_eq!(named(&summary, "Spend").months, [100, 0, 200]);

        let conn = open_in_memory().unwrap();
        let summary = monthly_summary(&conn, &ViewFilter::default()).unwrap();
        assert!(summary.months.is_empty(), "no data, no month columns");
        assert_eq!(summary.rows.len(), 3, "Income, Spend and Net remain");
    }
}

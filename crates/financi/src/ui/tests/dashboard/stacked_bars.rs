//! 3.93: the *Monthly spending by category* panel — `monthly_stacks`
//! unit tests (the GPUI-free shaping) and the panel's UI tests.

use super::*;
use crate::ui::dashboard_chart::{MonthStack, StackSegment, axis_amount, monthly_stacks};
use financi_db::{MonthlySummary, MonthlySummaryRow};
use financi_db::categories::CategoryNode;

// -- monthly_stacks unit tests -----------------------------------------

/// Food (Groceries, Restaurants), Transport (Bus) and the system
/// `Transfers` parent.
fn tree() -> Vec<CategoryNode> {
    let node = |id: i64, name: &str, children: Vec<CategoryNode>| CategoryNode {
        id,
        name: name.into(),
        kind: CategoryKind::Expense,
        is_system: false,
        children,
    };
    vec![
        node(
            1,
            "Food",
            vec![
                node(11, "Groceries", vec![]),
                node(12, "Restaurants", vec![]),
            ],
        ),
        node(2, "Transport", vec![node(21, "Bus", vec![])]),
        node(3, "Transfers", vec![]),
    ]
}

fn row(id: Option<i64>, name: &str, months: &[i64]) -> MonthlySummaryRow {
    MonthlySummaryRow {
        id,
        name: name.into(),
        months: months.to_vec(),
        total: months.iter().sum(),
    }
}

/// The summary shape `monthly_summary` produces: parents each followed
/// by their children (ordered by total over the range — Food, Transport,
/// then Transfers and Uncategorized tied at 200 by name), then `Income`,
/// `Spend` and `Net`. March has the most spend, April less, May none.
/// With transfers the `Spend` row is `[2100, 700, 0]`, without it
/// `[2000, 600, 0]`.
fn summary(with_transfers: bool) -> MonthlySummary {
    let spend = if with_transfers {
        [2100, 700, 0]
    } else {
        [2000, 600, 0]
    };
    let income = [210_500i64, 210_500, 0];
    let mut rows = vec![
        row(Some(1), "Food", &[1500, 600, 0]),
        row(Some(11), "Groceries", &[1000, 600, 0]),
        row(Some(12), "Restaurants", &[500, 0, 0]),
        row(Some(2), "Transport", &[300, 0, 0]),
        row(Some(21), "Bus", &[300, 0, 0]),
    ];
    if with_transfers {
        rows.push(row(Some(3), "Transfers", &[100, 100, 0]));
    }
    rows.push(row(None, "Uncategorized", &[200, 0, 0]));
    rows.push(row(None, "Income", &income));
    rows.push(row(None, "Spend", &spend));
    rows.push(row(
        None,
        "Net",
        &(0..3)
            .map(|ix| income[ix] - spend[ix])
            .collect::<Vec<i64>>(),
    ));
    MonthlySummary {
        months: ["2026-03", "2026-04", "2026-05"]
            .iter()
            .map(|m| m.to_string())
            .collect(),
        rows,
    }
}

#[test]
fn stack_segments_sum_to_the_spend_row_for_each_month() {
    let stacks = monthly_stacks(&summary(true), None, &tree());
    assert_eq!(stacks.len(), 3, "one bar per month");
    for stack in &stacks {
        let seg_sum: i64 = stack.segments.iter().map(|s| s.cents).sum();
        assert_eq!(
            stack.total, seg_sum,
            "{}: bar height {} is not the sum of its segments {seg_sum}",
            stack.month, stack.total
        );
    }
    assert_eq!(
        stacks.iter().map(|s| s.total).collect::<Vec<i64>>(),
        [2100, 700, 0],
        "the bar height is the month's `Spend` row"
    );
}

#[test]
fn income_net_and_child_rows_are_left_out_of_the_segments() {
    let stacks = monthly_stacks(&summary(true), None, &tree());
    let names: Vec<&str> = stacks[0].segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["Food", "Transport", "Transfers", "Uncategorized"],
        "parents plus Uncategorized last; no children, no total rows"
    );
    for stack in &stacks {
        for seg in &stack.segments {
            assert!(
                !["Groceries", "Restaurants", "Bus", "Income", "Spend", "Net"]
                    .contains(&seg.name.as_str()),
                "{} must not be a segment",
                seg.name
            );
        }
    }
}

#[test]
fn drilling_swaps_in_the_children_and_the_parents_row_is_the_height() {
    let stacks = monthly_stacks(&summary(true), Some(1), &tree());
    let names: Vec<&str> = stacks[0].segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Groceries", "Restaurants"]);
    assert_eq!(
        stacks.iter().map(|s| s.total).collect::<Vec<i64>>(),
        [1500, 600, 0],
        "the bar height is Food's own row"
    );
    // April: only Groceries still has spend.
    assert_eq!(stacks[1].segments.len(), 1);
    assert_eq!(stacks[1].segments[0].name.as_str(), "Groceries");
}

#[test]
fn transfers_appears_only_when_the_summary_includes_it() {
    let with = monthly_stacks(&summary(true), None, &tree());
    assert!(
        with[0]
            .segments
            .iter()
            .any(|s| s.name.as_str() == "Transfers"),
        "transfers included: the row is a segment"
    );
    let without = monthly_stacks(&summary(false), None, &tree());
    assert!(
        !without
            .iter()
            .any(|s| s.segments.iter().any(|g| g.name.as_str() == "Transfers")),
        "transfers excluded: no segment anywhere"
    );
    assert_eq!(
        without.iter().map(|s| s.total).collect::<Vec<i64>>(),
        [2000, 600, 0],
        "and the bar follows the (transfer-free) Spend row"
    );
}

#[test]
fn segment_order_is_stable_across_months() {
    let stacks = monthly_stacks(&summary(true), None, &tree());
    fn order(s: &MonthStack) -> Vec<&str> {
        s.segments
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<&str>>()
    }
    assert_eq!(
        order(&stacks[0]),
        ["Food", "Transport", "Transfers", "Uncategorized"]
    );
    // April is Groceries-only plus transfers — but the order is fixed by
    // the totals over the range, so the survivors keep their places.
    assert_eq!(order(&stacks[1]), ["Food", "Transfers"]);
    // May has no spend at all: the slot stays, empty.
    assert_eq!(stacks[2].segments, Vec::<StackSegment>::new());
    assert_eq!(stacks[2].total, 0);
}

#[test]
fn a_negative_month_clamps_to_an_empty_bar() {
    // A refund-heavy month: the Spend row goes negative, so the bar
    // clamps to zero and no segment paints.
    let mut s = summary(false);
    let n = s.rows.len();
    s.rows[n - 2].months[0] = -500; // Spend
    let stacks = monthly_stacks(&s, None, &tree());
    assert_eq!(stacks[0].total, 0);
    assert!(
        stacks[0].segments.iter().all(|g| g.cents > 0),
        "negative segments are dropped, never drawn below the axis"
    );
}

#[test]
fn a_summary_without_the_three_total_rows_yields_no_bars() {
    let s = MonthlySummary {
        months: vec!["2026-03".into()],
        rows: vec![row(Some(1), "Food", &[100])],
    };
    assert!(monthly_stacks(&s, None, &tree()).is_empty());
}

#[test]
fn axis_amounts_stay_compact() {
    assert_eq!(axis_amount(42), "$0.42");
    assert_eq!(axis_amount(100), "$1");
    assert_eq!(axis_amount(8_642), "$86");
    assert_eq!(axis_amount(99_999), "$999");
    assert_eq!(axis_amount(100_000), "$1k");
    assert_eq!(axis_amount(123_456), "$1.2k");
    assert_eq!(axis_amount(1_234_567), "$12.3k");
}

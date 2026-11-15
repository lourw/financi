//! Chart label plans: truncation, collision fallbacks and share labels.

use super::*;

#[test]
fn band_labels_truncate_to_the_character_budget() {
    let names = vec![
        "Food".to_string(),
        "Public Transport Costs".to_string(),
        "Café du Marché".to_string(),
    ];
    let shorts = short_chart_labels(&names, 8);
    assert_eq!(shorts[0].as_str(), "Food", "a short name is left alone");
    assert_eq!(shorts[1].as_str(), "Public…");
    assert_eq!(shorts[2].as_str(), "Café du…", "truncates by characters");

    // No budget means no truncation.
    let all = short_chart_labels(&names, 0);
    assert_eq!(all[1].as_str(), "Public Transport Costs");
}

#[test]
fn colliding_short_labels_fall_back_to_their_full_names() {
    // The band text is also `BarChart`'s band key, so two rows must never
    // be shortened to the same string.
    let names = vec![
        "Travel International".to_string(),
        "Travel Domestic".to_string(),
    ];
    let shorts = short_chart_labels(&names, 8);
    assert_eq!(shorts[0].as_str(), "Travel International");
    assert_eq!(shorts[1].as_str(), "Travel Domestic");

    // Without a collision the shortening stands.
    let names = vec!["Travel International".to_string(), "Food".to_string()];
    assert_eq!(short_chart_labels(&names, 8)[0].as_str(), "Travel…");
}

#[test]
fn band_label_budget_keeps_labels_inside_their_spacing() {
    // A wide plot with few bands: label every band with room to spare.
    let (chars, labels) = band_label_budget(779., 7);
    assert_eq!(labels, 7);
    assert!(chars >= 12, "{chars} chars at 779px across 7 bands");

    // The minimum-window plot (~167px) with 7 bands: too tight for all
    // seven, so labels are thinned out but still legible.
    let (chars, labels) = band_label_budget(167., 7);
    assert!((1..7).contains(&labels), "{labels} labels at 167px");
    assert!(chars >= CHART_LABEL_MIN_CHARS, "{chars}");

    // The invariant for every window size and category count: each drawn
    // label owns `chart_width / labels` pixels, enough for its characters
    // plus the gap, so centred labels cannot overlap or run past the
    // plot's edges.
    for width in [100., 167., 400., 779., 1600., 3000.] {
        for count in 1..=30usize {
            let (chars, labels) = band_label_budget(width, count);
            assert!(
                labels >= 1 && labels <= count,
                "width {width}, count {count}"
            );
            assert!((1..=CHART_LABEL_MAX_CHARS).contains(&chars));
            let per = width / labels as f32;
            assert!(
                chars as f32 * CHART_CHAR_PX + CHART_LABEL_GAP <= per + 0.001,
                "width {width}, count {count}: {chars} chars need more than {per:.1}px"
            );
        }
    }
    assert_eq!(band_label_budget(779., 0), (0, 0));
}

// -- Bar share labels and the slim legend (3.31) ------------------------

#[test]
fn bar_share_labels_read_as_whole_percents() {
    assert_eq!(bar_share_label(0.75), "75%");
    assert_eq!(bar_share_label(0.182), "18%");
    assert_eq!(bar_share_label(1.0), "100%");
    assert_eq!(
        bar_share_label(0.999),
        "100%",
        "rounds up to a whole percent"
    );
    assert_eq!(
        bar_share_label(0.01),
        "1%",
        "one percent is a whole percent"
    );
    assert_eq!(
        bar_share_label(0.009),
        "0.9%",
        "below one percent, one decimal"
    );
    assert_eq!(bar_share_label(0.004), "0.4%");
    assert_eq!(bar_share_label(0.0), "0%");
    assert_eq!(
        bar_share_label(0.0095),
        "0.9%",
        "0.95–1% still reads with its one decimal"
    );
    assert_eq!(bar_share_label(0.0094), "0.9%");
    assert_eq!(
        bar_share_label(0.0099),
        "1.0%",
        "0.99% rounds to one percent with its decimal, not a bare 1%"
    );
    assert_eq!(
        bar_share_label(0.0004),
        "0.1%",
        "a sliver below 0.05% ($1 of $2,500) rounds up, never to 0.0%"
    );
    assert_eq!(bar_share_label(0.00004), "0.1%");

    // The tooltip body is the bar's and the legend row's one string.
    assert_eq!(
        chart_tooltip_value(123456, 0.182, " of Food"),
        "$1,234.56 · 18.2% of Food"
    );
    assert_eq!(chart_tooltip_value(1500, 0.75, ""), "$15.00 · 75.0%");
}

#[test]
fn bar_label_plan_thins_share_labels_without_collisions() {
    // A wide plot with few bands: every bar keeps its share label.
    assert_eq!(bar_label_plan(779., 7), 7);
    // The 1200 × 720 plot (~247 px) with 15 bars: too narrow for all
    // fifteen, so the labels are thinned out.
    let labels = bar_label_plan(247., 15);
    assert!((1..15).contains(&labels), "{labels} labels at 247px");
    assert_eq!(bar_label_plan(0., 5), 1, "a degenerate plot labels one bar");
    assert_eq!(bar_label_plan(100., 0), 0);
    assert_eq!(bar_label_plan(100., 1), 1);

    // The invariant for every window size and category count: drawn
    // labels are spread first-to-last and their band centres sit at
    // least a full label apart, so centred labels cannot overlap.
    let need = BAR_LABEL_MAX_CHARS as f32 * CHART_CHAR_PX;
    for width in [100., 247., 400., 779., 1600., 3000.] {
        for count in 1..=30usize {
            let labels = bar_label_plan(width, count);
            assert!(
                (1..=count).contains(&labels),
                "width {width}, count {count}: {labels} labels"
            );
            let slots = bar_label_slots(count, labels);
            let drawn: Vec<usize> = slots
                .iter()
                .enumerate()
                .filter(|(_, on)| **on)
                .map(|(ix, _)| ix)
                .collect();
            assert_eq!(drawn.len(), labels, "width {width}, count {count}");
            assert_eq!(drawn.first(), Some(&0), "the first bar is labelled");
            assert_eq!(drawn.last(), Some(&(count - 1)), "the last bar is labelled");
            let band_w = width / count as f32;
            for pair in drawn.windows(2) {
                let spacing = (pair[1] - pair[0]) as f32 * band_w;
                assert!(
                    spacing >= need - 0.001,
                    "width {width}, count {count}: labels on bars {} and {} \
                     are {spacing:.1}px apart, need {need:.1}",
                    pair[0],
                    pair[1],
                );
            }
        }
    }
}

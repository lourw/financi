//! Category rules: pattern matching against a transaction description.
//!
//! A [`Rule`] maps a `pattern` (plain substring or regex, matched
//! case-insensitively) to a child category. Matching is description only —
//! no account or sign filters — and lives here as the pure
//! [`pick_category`]; storage and validation-on-save live in `financi-db`.
//!
//! Winner when several rules match: substring rules first, longest pattern
//! wins; only if no substring rule matches, regex rules, longest pattern
//! wins; ties broken by lowest rule id.

use regex::RegexBuilder;

/// One categorization rule, as stored in `category_rules`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: i64,
    pub pattern: String,
    /// `false`: case-insensitive substring of the description.
    /// `true`: regex, case-insensitive, [`regex`] crate syntax.
    pub is_regex: bool,
    /// Target category: always a child (enforced on save by `financi-db`).
    pub category_id: i64,
}

/// Case-insensitive check against a rule, or `false` when it can't match:
/// an empty pattern never matches, and a regex that fails to compile never
/// matches (invalid rules are rejected on save; `pick_category` stays pure
/// and skips them instead of panicking).
fn rule_matches(rule: &Rule, description: &str, description_lower: &str) -> bool {
    if rule.pattern.is_empty() {
        return false;
    }
    if rule.is_regex {
        match RegexBuilder::new(&rule.pattern)
            .case_insensitive(true)
            .build()
        {
            Ok(re) => re.is_match(description),
            Err(_) => false,
        }
    } else {
        // ASCII case folding, consistent with the rest of core (parse.rs).
        description_lower.contains(&rule.pattern.to_ascii_lowercase())
    }
}

fn pattern_len(rule: &Rule) -> usize {
    rule.pattern.chars().count()
}

/// Longer pattern wins; on equal length the lower rule id wins.
fn more_specific(a: &Rule, b: &Rule) -> std::cmp::Ordering {
    pattern_len(a).cmp(&pattern_len(b)).then(b.id.cmp(&a.id))
}

/// The category a description belongs to, or `None` when no rule matches.
///
/// Substring rules beat regex rules even when a regex pattern is longer;
/// within each kind the longest pattern wins, ties go to the lowest id.
pub fn pick_category(rules: &[Rule], description: &str) -> Option<i64> {
    let description_lower = description.to_ascii_lowercase();
    for is_regex in [false, true] {
        let winner = rules
            .iter()
            .filter(|rule| rule.is_regex == is_regex)
            .filter(|rule| rule_matches(rule, description, &description_lower))
            .max_by(|a, b| more_specific(a, b));
        if let Some(rule) = winner {
            return Some(rule.category_id);
        }
    }
    None
}

/// Validate a regex rule pattern exactly as [`pick_category`] uses it.
/// `Err` carries the `regex` crate's message for the user.
pub fn validate_regex(pattern: &str) -> Result<(), String> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: i64, pattern: &str, is_regex: bool, category_id: i64) -> Rule {
        Rule {
            id,
            pattern: pattern.into(),
            is_regex,
            category_id,
        }
    }

    #[test]
    fn no_rules_matches_nothing() {
        assert_eq!(pick_category(&[], "COFFEE PURCHASE"), None);
    }

    #[test]
    fn single_substring_rule_matches() {
        let rules = [rule(1, "COFFEE", false, 10)];
        assert_eq!(pick_category(&rules, "TIM HORTONS COFFEE 4.50"), Some(10));
        assert_eq!(pick_category(&rules, "TIM HORTONS DONUTS"), None);
    }

    #[test]
    fn substring_matching_is_case_insensitive_both_ways() {
        let upper = [rule(1, "COFFEE", false, 10)];
        assert_eq!(pick_category(&upper, "coffee purchase"), Some(10));
        let lower = [rule(1, "coffee", false, 10)];
        assert_eq!(pick_category(&lower, "COFFEE PURCHASE"), Some(10));
    }

    #[test]
    fn longest_substring_wins() {
        let rules = [
            rule(1, "COFFEE", false, 10),
            rule(2, "COFFEE SHOP", false, 20),
        ];
        assert_eq!(
            pick_category(&rules, "CORNER COFFEE SHOP VANCOUVER"),
            Some(20)
        );
    }

    #[test]
    fn substring_beats_regex_even_when_regex_is_longer() {
        let rules = [
            rule(1, "CO.*SHOP", true, 10),
            rule(2, "COFFEE SHOP", false, 20),
        ];
        assert_eq!(pick_category(&rules, "COFFEE SHOP"), Some(20));
    }

    #[test]
    fn regex_used_only_when_no_substring_matches() {
        let rules = [rule(1, "CAFE.*", true, 10), rule(2, "COFFEE", false, 20)];
        assert_eq!(pick_category(&rules, "CAFE O VANCOUVER"), Some(10));
        assert_eq!(pick_category(&rules, "COFFEE AND CAFE O"), Some(20));
    }

    #[test]
    fn longest_regex_wins() {
        let rules = [
            rule(1, "CAFE.*", true, 10),
            rule(2, "CAFE.*VANCOUVER", true, 20),
        ];
        assert_eq!(pick_category(&rules, "CAFE O VANCOUVER BC"), Some(20));
    }

    #[test]
    fn regex_matching_is_case_insensitive() {
        let rules = [rule(1, "cafe o", true, 10)];
        assert_eq!(pick_category(&rules, "CAFE O VANCOUVER"), Some(10));
    }

    #[test]
    fn equal_length_substrings_tie_break_to_lowest_id() {
        let rules = [rule(5, "COFFEE", false, 10), rule(3, "MUFFIN", false, 20)];
        assert_eq!(pick_category(&rules, "COFFEE AND MUFFIN"), Some(20));
    }

    #[test]
    fn equal_length_regexes_tie_break_to_lowest_id() {
        let rules = [
            rule(5, r"coffee\d", true, 10),
            rule(3, r"muffin\d", true, 20),
        ];
        assert_eq!(pick_category(&rules, "coffee1 muffin2"), Some(20));
    }

    #[test]
    fn invalid_regex_rules_are_skipped() {
        let rules = [rule(1, "(unclosed", true, 10), rule(2, "CAFE", true, 20)];
        assert_eq!(pick_category(&rules, "CAFE O"), Some(20));
        assert_eq!(pick_category(&rules, "UNCLOSED"), None);
    }

    #[test]
    fn empty_patterns_never_match() {
        let rules = [rule(1, "", false, 10), rule(2, "", true, 20)];
        assert_eq!(pick_category(&rules, "ANY DESCRIPTION"), None);
    }

    #[test]
    fn validate_regex_accepts_what_pick_category_compiles() {
        validate_regex(r"cafe\s+\w+").unwrap();
        validate_regex("(unclosed").unwrap_err();
    }
}

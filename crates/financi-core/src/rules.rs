//! Category rules: pattern matching against a transaction description.
//!
//! A [`Rule`] maps a `pattern` (plain substring or regex, matched
//! case-insensitively) to a child category. Matching is description only —
//! no account or sign filters — and lives here as the pure
//! [`winning_rule`] and [`pick_category`]; storage and validation-on-save
//! live in `financi-db`.
//!
//! Winner when several rules match: substring rules first, longest pattern
//! wins; only if no substring rule matches, regex rules, longest pattern
//! wins; ties broken by lowest rule id.
//!
//! [`pick_category`] compiles what it needs per call, which is fine for
//! one description; callers matching many descriptions against the same
//! rules build [`CompiledRules`] once and [`pick`](CompiledRules::pick)
//! per description (3.40), or use [`CompiledPattern`] for a single
//! pattern.

use std::cmp::Ordering;

use regex::{Regex, RegexBuilder};

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

/// What a [`CompiledPattern`] holds after compiling once: a lowercased
/// substring, a case-insensitive regex, or a marker that the pattern can
/// never match (an empty pattern, or a regex that fails to compile —
/// invalid rules are rejected on save; matching stays pure and skips them
/// instead of panicking).
enum Pattern {
    Substring(String),
    Regex(Regex),
    Never,
}

/// One rule pattern, compiled once for repeated matching (3.40): the
/// substring is lowercased once, a regex is built once. The single-pattern
/// form of [`CompiledRules`], for callers that match one pattern (plus an
/// excluded rule) over many descriptions.
pub struct CompiledPattern {
    kind: Pattern,
}

impl CompiledPattern {
    /// Compile `pattern` under [`pattern_matches`] semantics:
    /// case-insensitive substring, or case-insensitive regex. An empty
    /// pattern or an invalid regex compiles to a pattern that never
    /// matches.
    pub fn new(pattern: &str, is_regex: bool) -> Self {
        let kind = if pattern.is_empty() {
            Pattern::Never
        } else if is_regex {
            match RegexBuilder::new(pattern).case_insensitive(true).build() {
                Ok(re) => Pattern::Regex(re),
                Err(_) => Pattern::Never,
            }
        } else {
            // ASCII case folding, consistent with the rest of core (parse.rs).
            Pattern::Substring(pattern.to_ascii_lowercase())
        };
        Self { kind }
    }

    /// Does this pattern match `description`? An empty pattern or an
    /// invalid regex never matches.
    pub fn is_match(&self, description: &str) -> bool {
        self.matches(description, &description.to_ascii_lowercase())
    }

    /// Like [`CompiledPattern::is_match`], given the caller's ASCII-
    /// lowercased description, so matching one description against many
    /// patterns lowers it only once.
    fn matches(&self, description: &str, description_lower: &str) -> bool {
        match &self.kind {
            Pattern::Substring(needle) => description_lower.contains(needle.as_str()),
            Pattern::Regex(re) => re.is_match(description),
            Pattern::Never => false,
        }
    }
}

/// One rule inside a [`CompiledRules`], with the tie-break data from its
/// source rule.
struct CompiledRule {
    pattern: CompiledPattern,
    /// `pattern.chars().count()` of the source rule, for tie-breaks.
    len: usize,
    id: i64,
    category_id: i64,
}

/// A rule set with every pattern compiled once (3.40). [`pick_category`]
/// builds each regex rule per call, so matching it row by row compiles
/// every regex once per transaction; `reapply_rules` and the live previews
/// build this instead — each regex is built once, case-insensitive,
/// invalid ones dropped, each substring pattern lowercased once — and
/// [`pick`](Self::pick) or [`winning`](Self::winning) per description,
/// with exactly [`pick_category`] semantics.
pub struct CompiledRules {
    /// Substring rules; consulted first, like [`winning_rule`] does.
    substrings: Vec<CompiledRule>,
    /// Regex rules; only consulted when no substring rule matches.
    regexes: Vec<CompiledRule>,
}

impl CompiledRules {
    /// Compile `rules` once. Empty patterns and regex patterns that fail
    /// to compile are dropped, exactly as [`pick_category`] treats them.
    pub fn new(rules: &[Rule]) -> Self {
        let mut compiled = Self {
            substrings: Vec::new(),
            regexes: Vec::new(),
        };
        for rule in rules {
            let pattern = CompiledPattern::new(&rule.pattern, rule.is_regex);
            if let Pattern::Never = pattern.kind {
                continue;
            }
            let entry = CompiledRule {
                pattern,
                len: pattern_len(rule),
                id: rule.id,
                category_id: rule.category_id,
            };
            if rule.is_regex {
                compiled.regexes.push(entry);
            } else {
                compiled.substrings.push(entry);
            }
        }
        compiled
    }

    /// The category these rules assign `description`, or `None` when
    /// nothing matches — [`pick_category`] over the compiled rules.
    pub fn pick(&self, description: &str) -> Option<i64> {
        self.winner(description).map(|rule| rule.category_id)
    }

    /// The id of the rule that decides `description`, or `None` when
    /// nothing matches — [`winning_rule`] over the compiled rules.
    pub fn winning(&self, description: &str) -> Option<i64> {
        self.winner(description).map(|rule| rule.id)
    }

    /// Substring rules beat regex rules even when a regex pattern is
    /// longer; within each kind the longest pattern wins, ties go to the
    /// lowest id (the first candidate, since distinct rules never tie —
    /// matching `max_by(more_specific)`'s keep-the-greater walk).
    fn winner(&self, description: &str) -> Option<&CompiledRule> {
        let description_lower = description.to_ascii_lowercase();
        for rules in [&self.substrings, &self.regexes] {
            let mut best: Option<&CompiledRule> = None;
            for candidate in rules {
                if !candidate.pattern.matches(description, &description_lower) {
                    continue;
                }
                best = Some(match best {
                    Some(current) if more_specific(candidate, current) == Ordering::Less => current,
                    _ => candidate,
                });
            }
            if best.is_some() {
                return best;
            }
        }
        None
    }
}

fn pattern_len(rule: &Rule) -> usize {
    rule.pattern.chars().count()
}

/// Longer pattern wins; on equal length the lower rule id wins.
fn more_specific(a: &CompiledRule, b: &CompiledRule) -> Ordering {
    a.len.cmp(&b.len).then(b.id.cmp(&a.id))
}

/// The id of the rule that decides `description`, or `None` when no rule
/// matches. Substring rules beat regex rules even when a regex pattern is
/// longer; within each kind the longest pattern wins, ties go to the lowest
/// id. [`pick_category`] delegates here, so the two can't drift.
///
/// Matching many descriptions? Build [`CompiledRules`] once and use
/// [`CompiledRules::winning`].
pub fn winning_rule(rules: &[Rule], description: &str) -> Option<i64> {
    CompiledRules::new(rules).winning(description)
}

/// The category a description belongs to, or `None` when no rule matches.
///
/// Substring rules beat regex rules even when a regex pattern is longer;
/// within each kind the longest pattern wins, ties go to the lowest id.
///
/// Matching many descriptions? Build [`CompiledRules`] once and use
/// [`CompiledRules::pick`].
pub fn pick_category(rules: &[Rule], description: &str) -> Option<i64> {
    CompiledRules::new(rules).pick(description)
}

/// Does `pattern` match `description` under [`pick_category`] semantics:
/// case-insensitive substring, or case-insensitive regex. An empty pattern
/// or an invalid regex never matches.
///
/// Matching many descriptions? Build a [`CompiledPattern`] once.
pub fn pattern_matches(pattern: &str, is_regex: bool, description: &str) -> bool {
    CompiledPattern::new(pattern, is_regex).is_match(description)
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
    fn winning_rule_returns_the_deciding_rule_id() {
        let rules = [rule(1, "COFFEE", false, 10), rule(2, "TEA", false, 20)];
        assert_eq!(winning_rule(&rules, "BEST COFFEE IN TOWN"), Some(1));
        assert_eq!(winning_rule(&rules, "LOOSE LEAF TEA"), Some(2));
    }

    #[test]
    fn winning_rule_none_when_nothing_matches() {
        let rules = [rule(1, "COFFEE", false, 10)];
        assert_eq!(winning_rule(&rules, "HARDWARE STORE"), None);
        assert_eq!(winning_rule(&[], "ANYTHING"), None);
    }

    #[test]
    fn winning_rule_substring_beats_longer_regex() {
        let rules = [
            rule(1, "CO.*SHOP", true, 10),
            rule(2, "COFFEE SHOP", false, 20),
        ];
        assert_eq!(winning_rule(&rules, "COFFEE SHOP"), Some(2));
    }

    #[test]
    fn winning_rule_longest_pattern_wins_within_its_kind() {
        let substrings = [
            rule(1, "COFFEE", false, 10),
            rule(2, "COFFEE SHOP", false, 20),
        ];
        assert_eq!(winning_rule(&substrings, "CORNER COFFEE SHOP"), Some(2));
        let regexes = [
            rule(3, "CAFE.*", true, 10),
            rule(4, "CAFE.*VANCOUVER", true, 20),
        ];
        assert_eq!(winning_rule(&regexes, "CAFE O VANCOUVER BC"), Some(4));
    }

    #[test]
    fn winning_rule_lowest_id_breaks_equal_length_ties() {
        let rules = [rule(5, "COFFEE", false, 10), rule(3, "MUFFIN", false, 20)];
        assert_eq!(winning_rule(&rules, "COFFEE AND MUFFIN"), Some(3));
    }

    #[test]
    fn winning_rule_skips_invalid_regexes_and_empty_patterns() {
        let rules = [
            rule(1, "(unclosed", true, 10),
            rule(2, "CAFE", true, 20),
            rule(3, "", false, 30),
        ];
        assert_eq!(winning_rule(&rules, "CAFE O"), Some(2));
        assert_eq!(winning_rule(&rules, "UNCLOSED"), None);
    }

    #[test]
    fn pick_category_uses_winning_rule() {
        let rules = [
            rule(7, "CAFE.*", true, 10),
            rule(8, "COFFEE SHOP", false, 20),
        ];
        assert_eq!(
            pick_category(&rules, "CORNER COFFEE SHOP"),
            Some(rules[1].category_id)
        );
        assert_eq!(
            pick_category(&rules, "CAFE NOIR"),
            Some(rules[0].category_id)
        );
    }

    #[test]
    fn validate_regex_accepts_what_pick_category_compiles() {
        validate_regex(r"cafe\s+\w+").unwrap();
        validate_regex("(unclosed").unwrap_err();
    }

    #[test]
    fn pattern_matches_follows_pick_category_semantics() {
        assert!(pattern_matches("tim hortons", false, "TIM HORTONS #123"));
        assert!(!pattern_matches("^bean", false, "TIM HORTONS #123"));
        assert!(pattern_matches(r"CAFE\s+\w+", true, "best CAFE noir sign"));
        assert!(!pattern_matches(r"^bean", true, "TIM HORTONS #123"));
        assert!(!pattern_matches("(unclosed", true, "anything"));
        assert!(!pattern_matches("", false, "anything"));
    }

    /// 3.40: the compiled form decides exactly like the one-shot
    /// functions — the cases `pick_category`'s tests pin down, run
    /// against `CompiledRules` directly (not against the wrapper).
    #[test]
    fn compiled_rules_match_pick_category_cases() {
        let rules = [
            rule(1, "COFFEE", false, 10),
            rule(2, "COFFEE SHOP", false, 20),
            rule(3, "CAFE.*", true, 30),
            rule(4, "CO.*SHOP", true, 40),
            rule(5, "(unclosed", true, 50),
            rule(6, "", false, 60),
            rule(7, "", true, 70),
            rule(8, "MUFFIN", false, 80),
        ];
        let compiled = CompiledRules::new(&rules);
        assert_eq!(compiled.pick("TIM HORTONS COFFEE 4.50"), Some(10));
        assert_eq!(
            compiled.pick("CORNER COFFEE SHOP VANCOUVER"),
            Some(20),
            "longest substring wins"
        );
        assert_eq!(
            compiled.pick("COFFEE SHOP"),
            Some(20),
            "substring beats regex even when a regex pattern is longer"
        );
        assert_eq!(
            compiled.pick("CAFE O VANCOUVER BC"),
            Some(30),
            "regex used only when no substring matches; longest regex wins"
        );
        assert_eq!(
            compiled.pick("COFFEE AND MUFFIN"),
            Some(10),
            "equal-length substrings tie-break to the lowest id"
        );
        assert_eq!(compiled.pick("UNCLOSED"), None, "invalid regex is dropped");
        assert_eq!(compiled.pick("ANY DESCRIPTION"), None);
        assert_eq!(
            CompiledRules::new(&[]).pick("ANYTHING"),
            None,
            "no rules match nothing"
        );
    }

    #[test]
    fn compiled_rules_winning_matches_winning_rule_cases() {
        let rules = [
            rule(5, "COFFEE", false, 10),
            rule(3, "MUFFIN", false, 20),
            rule(9, r"latte\d", true, 30),
            rule(11, r"mocha\d", true, 40),
        ];
        let compiled = CompiledRules::new(&rules);
        assert_eq!(compiled.winning("COFFEE AND MUFFIN"), Some(3));
        assert_eq!(
            compiled.winning("latte1 mocha2"),
            Some(9),
            "equal-length regexes tie-break to the lowest id"
        );
        assert_eq!(compiled.winning("HARDWARE STORE"), None);
    }

    #[test]
    fn compiled_pattern_matches_pattern_matches_cases() {
        assert!(CompiledPattern::new("tim hortons", false).is_match("TIM HORTONS #123"));
        assert!(!CompiledPattern::new("^bean", false).is_match("TIM HORTONS #123"));
        assert!(CompiledPattern::new(r"CAFE\s+\w+", true).is_match("best CAFE noir sign"));
        assert!(!CompiledPattern::new("(unclosed", true).is_match("anything"));
        assert!(!CompiledPattern::new("", false).is_match("anything"));
        assert!(!CompiledPattern::new("", true).is_match("anything"));
    }
}

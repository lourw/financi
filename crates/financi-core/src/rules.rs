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
//!
//! 3.55: [`CompiledRules`] matches with two engines instead of walking
//! every rule per description — one Aho-Corasick automaton over the
//! substring needles (an overlapping scan reports every needle occurring
//! in the description, in any order) and one case-insensitive
//! [`regex::RegexSet`] over the regex patterns (one DFA pass answers all
//! of them at once). The deciding rule is unchanged: substring rules
//! first, longest pattern wins, ties to the lowest id.

use std::cmp::Ordering;

use aho_corasick::{AhoCorasick, MatchKind};
use regex::{Regex, RegexBuilder, RegexSet, RegexSetBuilder};

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
            // Unicode case folding (3.72): regex rules are Unicode
            // case-insensitive, so substring rules fold the same way and
            // `Montréal` matches `MONTRÉAL`. `is_match` folds the
            // description the same way — or skips folding when both sides
            // are ASCII, where the two fold identically.
            Pattern::Substring(pattern.to_lowercase())
        };
        Self { kind }
    }

    /// Does this pattern match `description`? An empty pattern or an
    /// invalid regex never matches.
    pub fn is_match(&self, description: &str) -> bool {
        match &self.kind {
            // 3.55 fast path: with an ASCII needle, comparing bytes
            // case-insensitively folds exactly like Unicode lowering both
            // sides (and a non-ASCII needle could never match an all-ASCII
            // description anyway), so no lowercased copy is needed.
            Pattern::Substring(needle) if needle.is_ascii() => description
                .as_bytes()
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle.as_bytes())),
            Pattern::Substring(needle) => description.to_lowercase().contains(needle.as_str()),
            Pattern::Regex(re) => re.is_match(description),
            Pattern::Never => false,
        }
    }
}

/// One rule inside a [`CompiledRules`], with the tie-break data from its
/// source rule. The matcher itself lives in the [`Substrings`] /
/// [`Regexes`] engine, indexed in the same order as these entries.
struct CompiledRule {
    /// `pattern.chars().count()` of the source rule, for tie-breaks.
    len: usize,
    id: i64,
    category_id: i64,
}

/// The substring half of [`CompiledRules`]: one Aho-Corasick automaton
/// over the lowercased needles (3.55), with `needles[i]` and `rules[i]`
/// describing the same rule. `automaton` is `None` when there are no
/// needles or building it failed, and matching falls back to a
/// [`str::contains`] walk — the pre-3.55 path.
struct Substrings {
    rules: Vec<CompiledRule>,
    needles: Vec<String>,
    /// Every needle is ASCII, so the automaton is built ASCII
    /// case-insensitive and can scan an all-ASCII description directly
    /// (3.55): ASCII case folding is Unicode case folding there, and a
    /// non-ASCII needle could never match an ASCII description anyway.
    all_ascii: bool,
    automaton: Option<AhoCorasick>,
}

impl Substrings {
    fn new(rules: &[&Rule]) -> Self {
        let needles: Vec<String> = rules
            .iter()
            .map(|rule| rule.pattern.to_lowercase())
            .collect();
        let all_ascii = needles.iter().all(|needle| needle.is_ascii());
        let automaton = if needles.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                // Standard (the default) is the only match kind an
                // overlapping search accepts; see [`Substrings::best_match`].
                .match_kind(MatchKind::Standard)
                .ascii_case_insensitive(all_ascii)
                .build(&needles)
                .ok()
        };
        Self {
            rules: rules.iter().map(|rule| CompiledRule::new(rule)).collect(),
            needles,
            all_ascii,
            automaton,
        }
    }

    /// The winning substring rule for `description`.
    ///
    /// With `MatchKind::Standard`, the overlapping search reports every
    /// pattern occurring in the haystack at every position — including
    /// needles nested in one another (`COFFEE` inside `COFFEE SHOP`) and
    /// duplicate needles — so scanning its output and keeping the most
    /// specific match decides exactly like walking every rule did.
    ///
    /// The haystack is the Unicode lowercase fold of the description,
    /// except that an all-ASCII needle set scans the original description
    /// (see [`Substrings::all_ascii`]) — no per-row copy on the common
    /// path (3.55).
    fn best_match(&self, description: &str) -> Option<&CompiledRule> {
        match &self.automaton {
            Some(automaton) if self.all_ascii && description.is_ascii() => {
                self.scan(automaton, description)
            }
            Some(automaton) => self.scan(automaton, &description.to_lowercase()),
            None => self.walk(&description.to_lowercase()),
        }
    }

    fn scan(&self, automaton: &AhoCorasick, haystack: &str) -> Option<&CompiledRule> {
        let mut best: Option<&CompiledRule> = None;
        for found in automaton.find_overlapping_iter(haystack) {
            let candidate = &self.rules[found.pattern().as_usize()];
            best = Some(most_specific(best, candidate));
        }
        best
    }

    /// Pre-3.55 fallback walk, for a lowercased haystack and needles.
    fn walk(&self, description_lower: &str) -> Option<&CompiledRule> {
        let mut best: Option<&CompiledRule> = None;
        for (needle, rule) in self.needles.iter().zip(&self.rules) {
            if description_lower.contains(needle.as_str()) {
                best = Some(most_specific(best, rule));
            }
        }
        best
    }
}

/// The regex half of [`CompiledRules`] (3.55). Fast path: every pattern
/// in one case-insensitive [`RegexSet`] — a pattern matches inside the
/// set exactly when it matches on its own, so one DFA pass answers all
/// rules at once. Building the set can still fail (a pattern the set
/// refuses, the whole set over its size limit); then each rule compiles
/// individually instead and matching is the pre-3.55 one-by-one walk.
/// Either way a regex that fails to compile is dropped, as
/// [`pick_category`] treats it.
///
/// The engine is built lazily, on the first [`best_match`](Self::best_match)
/// (3.55): the live preview compiles a hypothetical rule set per keystroke
/// and only asks the regexes when a row is not already decided by a
/// substring rule, so building eagerly would pay the multi-millisecond
/// `RegexSet` build for nothing.
struct Regexes {
    /// The regex rules in set order, kept until the engine builds.
    source: Vec<Rule>,
    engine: std::sync::OnceLock<RegexEngine>,
}

/// The compiled form [`Regexes`] builds on first use.
enum RegexEngine {
    Set {
        set: RegexSet,
        /// Same order as the patterns handed to the set.
        rules: Vec<CompiledRule>,
    },
    Individual {
        entries: Vec<(CompiledRule, Regex)>,
    },
}

impl Regexes {
    fn new(rules: &[&Rule]) -> Self {
        Self {
            source: rules.iter().map(|rule| (*rule).clone()).collect(),
            engine: std::sync::OnceLock::new(),
        }
    }

    fn engine(&self) -> &RegexEngine {
        self.engine.get_or_init(|| build_engine(&self.source))
    }

    /// The winning regex rule for `description` — matched against the
    /// original description, like [`CompiledPattern`] does, not its
    /// lowercase fold.
    fn best_match(&self, description: &str) -> Option<&CompiledRule> {
        let mut best: Option<&CompiledRule> = None;
        match self.engine() {
            RegexEngine::Set { set, rules } => {
                for index in set.matches(description).iter() {
                    best = Some(most_specific(best, &rules[index]));
                }
            }
            RegexEngine::Individual { entries } => {
                for (rule, re) in entries {
                    if re.is_match(description) {
                        best = Some(most_specific(best, rule));
                    }
                }
            }
        }
        best
    }

    #[cfg(test)]
    fn engine_is_set(&self) -> bool {
        matches!(self.engine(), RegexEngine::Set { .. })
    }
}

/// Compile `rules` (all regex rules, in id order) into the engine
/// [`Regexes`] matches with; see the type docs for the fallbacks.
fn build_engine(rules: &[Rule]) -> RegexEngine {
    if rules.is_empty() {
        return RegexEngine::Individual {
            entries: Vec::new(),
        };
    }
    let patterns: Vec<&str> = rules.iter().map(|rule| rule.pattern.as_str()).collect();
    if let Some(set) = build_set(&patterns) {
        return RegexEngine::Set {
            set,
            rules: rules.iter().map(CompiledRule::new).collect(),
        };
    }
    // One invalid pattern fails the whole set build, and an oversized
    // set can fail it too. Compile each rule on its own — dropping the
    // invalid ones, as [`pick_category`] treats them — then retry the
    // set with only the valid patterns; if that still fails, matching
    // is the pre-3.55 per-regex walk.
    let mut entries: Vec<(&Rule, CompiledRule, Regex)> = Vec::new();
    for rule in rules {
        if let Ok(re) = RegexBuilder::new(&rule.pattern)
            .case_insensitive(true)
            .build()
        {
            entries.push((rule, CompiledRule::new(rule), re));
        }
    }
    if !entries.is_empty() && entries.len() < rules.len() {
        // Dropped at least one invalid pattern; the smaller set may
        // still build.
        let valid: Vec<&str> = entries
            .iter()
            .map(|(rule, ..)| rule.pattern.as_str())
            .collect();
        if let Some(set) = build_set(&valid) {
            return RegexEngine::Set {
                set,
                rules: entries.into_iter().map(|(_, rule, _)| rule).collect(),
            };
        }
    }
    RegexEngine::Individual {
        entries: entries
            .into_iter()
            .map(|(_, rule, re)| (rule, re))
            .collect(),
    }
}

fn build_set(patterns: &[&str]) -> Option<RegexSet> {
    let mut builder = RegexSetBuilder::new(patterns);
    builder.case_insensitive(true);
    #[cfg(test)]
    if let Some(limit) = TEST_SET_SIZE_LIMIT.with(|limit| limit.get()) {
        builder.size_limit(limit);
    }
    builder.build().ok()
}

// Test hook: when set, `build_set` caps the shared `RegexSet` at this
// many bytes so tests can force the per-regex fallback.
#[cfg(test)]
thread_local! {
    pub(crate) static TEST_SET_SIZE_LIMIT: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

/// A rule set with every pattern compiled once (3.40). [`pick_category`]
/// builds each regex rule per call, so matching it row by row compiles
/// every regex once per transaction; `reapply_rules` and the live previews
/// build this instead — and since 3.55 compile the whole set into an
/// Aho-Corasick automaton and a `RegexSet` (the latter on first regex
/// need) — then [`pick`](Self::pick) or [`winning`](Self::winning) per
/// description costs two linear scans regardless of rule count, with
/// exactly [`pick_category`] semantics.
pub struct CompiledRules {
    /// Substring rules; consulted first, like [`winning_rule`] does.
    substrings: Substrings,
    /// Regex rules; only consulted when no substring rule matches.
    regexes: Regexes,
}

impl CompiledRules {
    /// Compile `rules` once. Empty patterns and regex patterns that fail
    /// to compile are dropped, exactly as [`pick_category`] treats them.
    pub fn new(rules: &[Rule]) -> Self {
        let mut substrings: Vec<&Rule> = Vec::new();
        let mut regexes: Vec<&Rule> = Vec::new();
        for rule in rules {
            if rule.pattern.is_empty() {
                continue;
            }
            if rule.is_regex {
                regexes.push(rule);
            } else {
                substrings.push(rule);
            }
        }
        Self {
            substrings: Substrings::new(&substrings),
            regexes: Regexes::new(&regexes),
        }
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
    /// lowest id.
    fn winner(&self, description: &str) -> Option<&CompiledRule> {
        self.substrings
            .best_match(description)
            .or_else(|| self.regexes.best_match(description))
    }
}

impl CompiledRule {
    fn new(rule: &Rule) -> Self {
        Self {
            len: pattern_len(rule),
            id: rule.id,
            category_id: rule.category_id,
        }
    }
}

fn pattern_len(rule: &Rule) -> usize {
    rule.pattern.chars().count()
}

/// The winner of `current` against `candidate`: longer pattern wins, on
/// equal length the lower rule id (the comparison the pre-3.55 per-rule
/// walk made at each step).
fn most_specific<'a>(
    current: Option<&'a CompiledRule>,
    candidate: &'a CompiledRule,
) -> &'a CompiledRule {
    match current {
        Some(current) if more_specific(candidate, current) == Ordering::Less => current,
        _ => candidate,
    }
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
/// One-off helper for a single description: it compiles the whole rule set
/// — every regex — on every call. Matching many descriptions against the
/// same rules? Build [`CompiledRules`] once and use [`CompiledRules::pick`].
pub fn pick_category(rules: &[Rule], description: &str) -> Option<i64> {
    CompiledRules::new(rules).pick(description)
}

/// Does `pattern` match `description` under [`pick_category`] semantics:
/// case-insensitive substring, or case-insensitive regex. An empty pattern
/// or an invalid regex never matches.
///
/// One-off helper for a single description: it compiles `pattern` on every
/// call. Matching many descriptions? Build a [`CompiledPattern`] once.
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

    /// 3.72: substring rules fold accented case like the (Unicode
    /// case-insensitive) regex rules do.
    #[test]
    fn substring_matching_ignores_accented_case() {
        let rules = [rule(1, "Montréal", false, 10)];
        assert_eq!(pick_category(&rules, "CAFE    MONTRÉAL"), Some(10));
        let rules = [rule(1, "MONTRÉAL", false, 10)];
        assert_eq!(pick_category(&rules, "Café Montréal"), Some(10));
        // Folding case, not accents: `cafe` still misses `CAFÉ`.
        let rules = [rule(1, "cafe", false, 10)];
        assert_eq!(pick_category(&rules, "CAFÉ PURCHASE"), None);
    }

    #[test]
    fn pattern_matches_ignores_accented_case() {
        assert!(pattern_matches("café", false, "CAFÉ PURCHASE"));
        assert!(pattern_matches("CAFÉ", false, "café purchase"));
        assert!(!pattern_matches("cafe", false, "CAFÉ PURCHASE"));
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

    /// 3.72: tie-break lengths come from `chars().count()` of the
    /// original pattern, not the lowercased one — `İ` (U+0130) lowercases
    /// to two chars (`i` plus a combining dot). With lowercased lengths
    /// `İ` and `ZZ` would tie and the lowest id would win; with original
    /// lengths `ZZ` (2) beats `İ` (1).
    #[test]
    fn tie_break_length_uses_the_original_pattern() {
        let rules = [rule(1, "İ", false, 10), rule(2, "ZZ", false, 20)];
        assert_eq!(pick_category(&rules, "ZZİ RESTAURANT"), Some(20));
        let compiled = CompiledRules::new(&rules);
        assert_eq!(compiled.winning("ZZİ RESTAURANT"), Some(2));
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

    /// 3.72: the compiled form folds accented case exactly like
    /// `pick_category`, on both the pattern and the description side.
    #[test]
    fn compiled_rules_fold_accented_case() {
        let rules = [rule(1, "Montréal", false, 10), rule(2, "CAFÉ.*", true, 20)];
        let compiled = CompiledRules::new(&rules);
        assert_eq!(compiled.pick("CAFE    MONTRÉAL"), Some(10));
        assert_eq!(compiled.pick("CAFÉ NOIR"), Some(20));
        assert_eq!(compiled.pick("café noir"), Some(20));
        assert_eq!(
            compiled.pick("CAFE NOIR"),
            None,
            "accents are not folded away"
        );
        assert!(CompiledPattern::new("Montréal", false).is_match("CAFE MONTRÉAL"));
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

#[cfg(test)]
mod scale_tests;

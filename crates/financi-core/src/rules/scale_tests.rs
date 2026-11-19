//! 3.55 scale tests for [`CompiledRules`]: the Aho-Corasick + `RegexSet`
//! engine must decide exactly like the pre-3.55 per-rule loop, including
//! when the shared `RegexSet` refuses to build and matching falls back to
//! per-regex matching.

use std::collections::HashMap;

use regex::Regex;

use super::{CompiledRules, Rule, TEST_SET_SIZE_LIMIT};

/// The pre-3.55 loop, verbatim in semantics: substring rules first, then
/// regex rules only if no substring matched; within each kind the longest
/// pattern wins, ties go to the lowest id; empty patterns and invalid
/// regexes never match. This is the oracle the compiled engine is
/// compared against. `cache` holds each rule's compiled regex so a
/// description sweep compiles every pattern once, not once per row.
fn reference_pick(
    rules: &[Rule],
    description: &str,
    cache: &mut HashMap<i64, Option<Regex>>,
) -> Option<(i64, i64)> {
    let description_lower = description.to_lowercase();
    for is_regex in [false, true] {
        let mut best: Option<(usize, i64, i64)> = None; // (len, id, category)
        for rule in rules.iter().filter(|rule| rule.is_regex == is_regex) {
            if rule.pattern.is_empty() {
                continue;
            }
            let matches = if rule.is_regex {
                let compiled = cache.entry(rule.id).or_insert_with(|| {
                    regex::RegexBuilder::new(&rule.pattern)
                        .case_insensitive(true)
                        .build()
                        .ok()
                });
                compiled.as_ref().is_some_and(|re| re.is_match(description))
            } else {
                description_lower.contains(&rule.pattern.to_lowercase())
            };
            if !matches {
                continue;
            }
            let candidate = (rule.pattern.chars().count(), rule.id, rule.category_id);
            best = Some(match best {
                Some(current)
                    if (candidate.0, std::cmp::Reverse(candidate.1))
                        < (current.0, std::cmp::Reverse(current.1)) =>
                {
                    current
                }
                _ => candidate,
            });
        }
        if let Some((_, id, category)) = best {
            return Some((id, category));
        }
    }
    None
}

fn assert_engine_matches_reference(rules: &[Rule], descriptions: &[String]) {
    let compiled = CompiledRules::new(rules);
    let mut cache: HashMap<i64, Option<Regex>> = HashMap::new();
    for description in descriptions {
        let expected = reference_pick(rules, description, &mut cache);
        assert_eq!(
            compiled.winning(description),
            expected.map(|(id, _)| id),
            "winning rule for {description:?} with rules {rules:?}"
        );
        assert_eq!(
            compiled.pick(description),
            expected.map(|(_, category)| category),
            "picked category for {description:?} with rules {rules:?}"
        );
    }
}

/// Tiny deterministic LCG (no rand crate): enough randomness to shake out
/// ordering and tie-break differences, fixed seeds so failures replay.
struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() as usize) % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }
}

/// Fragments for patterns and descriptions: mixed case, accents (3.72
/// folding), `İ` (lowercases to two chars — tie-break lengths come from
/// the original), digits, regex syntax, and things that make invalid
/// regexes (`(`, `a)`).
const FRAGMENTS: &[&str] = &[
    "COFFEE", "coffee", "Cafe", "SHOP", "shop", "o", " ", "#", "42", "É", "é", "É", "ñ", "İ", "i",
    r"\w+", r"\d+", ".*", "(", ")", "[", "]", "+", "?", "^", "$", "-", "CAFE", "MONTR", "REAL",
    "À", "à", "ss", "ß", "ST", "ST",
];

const DESCRIPTION_FRAGMENTS: &[&str] = &[
    "COFFEE",
    "coffee",
    "Cafe",
    "CAFÉ",
    "MONTRÉAL",
    "MONTR",
    "REAL",
    "SHOP",
    "shop",
    " #",
    "42",
    " ",
    "İ",
    "i",
    "ß",
    "ST",
    "st",
    "CAFE O",
    "-",
    "É",
    "é",
];

fn random_text(rng: &mut Lcg, fragments: &[&str], max_parts: usize) -> String {
    let parts = 1 + rng.below(max_parts);
    let mut text = String::new();
    for _ in 0..parts {
        let fragment = fragments[rng.below(fragments.len())];
        if rng.chance(50) {
            text.push_str(fragment.to_lowercase().as_str());
        } else {
            text.push_str(fragment.to_uppercase().as_str());
        }
    }
    text
}

#[test]
fn compiled_engine_decides_exactly_like_the_per_rule_loop() {
    let mut rng = Lcg(0x5eed_2026_1118);
    for round in 0..600 {
        let rule_count = rng.below(12);
        let mut rules = Vec::with_capacity(rule_count);
        for i in 0..rule_count {
            // Distinct ids, but shuffled relative to rule order so the
            // tie-break is exercised whichever way the list is sorted.
            let id = ((i * 7 + round * 13) % 97) as i64 + 1;
            if rules.iter().any(|r: &Rule| r.id == id) {
                continue;
            }
            rules.push(Rule {
                id,
                pattern: random_text(&mut rng, FRAGMENTS, 4),
                is_regex: rng.chance(50),
                category_id: 100 + i as i64,
            });
        }
        let mut descriptions: Vec<String> = (0..8)
            .map(|_| random_text(&mut rng, DESCRIPTION_FRAGMENTS, 6))
            .collect();
        // Each rule's own pattern as a description, so most rounds test a
        // real match, not just the no-match path.
        descriptions.extend(rules.iter().map(|rule| rule.pattern.clone()));
        descriptions.extend(rules.iter().map(|rule| {
            let mut text = random_text(&mut rng, DESCRIPTION_FRAGMENTS, 2);
            text.push_str(&rule.pattern);
            text.push_str(&random_text(&mut rng, DESCRIPTION_FRAGMENTS, 2));
            text
        }));
        assert_engine_matches_reference(&rules, &descriptions);
    }
}

#[test]
fn unicode_folding_and_original_length_survive_the_rewrite() {
    // 3.72 semantics through the new engine: fold case, not accents, and
    // tie-break on `chars().count()` of the original pattern.
    let rules = [
        Rule {
            id: 1,
            pattern: "Montréal".into(),
            is_regex: false,
            category_id: 10,
        },
        Rule {
            id: 2,
            pattern: "İ".into(),
            is_regex: false,
            category_id: 20,
        },
        Rule {
            id: 3,
            pattern: "ZZ".into(),
            is_regex: false,
            category_id: 30,
        },
        Rule {
            id: 4,
            pattern: "café.*".into(),
            is_regex: true,
            category_id: 40,
        },
    ];
    let descriptions: Vec<String> = [
        "CAFE    MONTRÉAL",
        "ZZİ RESTAURANT",
        "CAFÉ NOIR",
        "café noir",
        "CAFE NOIR",
        "İİİ",
        "",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_engine_matches_reference(&rules, &descriptions);
}

#[test]
fn plain_rule_sets_take_the_regex_set_fast_path() {
    let rules: Vec<Rule> = (0..50i64)
        .map(|i| Rule {
            id: i,
            pattern: format!(r"BR{i:03}\s*#\d{{1,3}}"),
            is_regex: true,
            category_id: 100 + i,
        })
        .collect();
    let compiled = CompiledRules::new(&rules);
    assert!(
        compiled.regexes.engine_is_set(),
        "expected the RegexSet fast path for 50 plain regexes"
    );
    assert_eq!(compiled.winning("PAYMENT BR007 #12"), Some(7));
}

/// 200 `\w+`-heavy regex rules — Unicode `\w` classes are huge, so the
/// shared `RegexSet` can refuse the whole set (size limit). Matching must
/// then fall back to per-regex matching and still decide like the loop.
/// The hook forces the refusal even on machines where the default limit
/// would let this set through.
#[test]
fn regex_set_failure_falls_back_to_per_regex_matching() {
    let rules: Vec<Rule> = (0..200i64)
        .map(|i| Rule {
            id: i,
            pattern: format!(r"(?:\w+\s+)*BR{i:03}\w*#\d{{1,3}}"),
            is_regex: true,
            category_id: 100 + i,
        })
        .collect();
    let descriptions: Vec<String> = (0..20)
        .map(|i| format!("SOME WORDS BR{i:03}EXTRA#{i} TAIL"))
        .chain((190..210).map(|i| format!("other BR{i:03} stuff")))
        .chain(std::iter::once("nothing here at all".to_string()))
        .collect();

    // With the hook the set build cannot succeed; matching must take the
    // per-regex fallback and still agree with the reference loop.
    TEST_SET_SIZE_LIMIT.with(|limit| limit.set(Some(1)));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let compiled = CompiledRules::new(&rules);
        assert!(
            !compiled.regexes.engine_is_set(),
            "expected the per-regex fallback under the size-limit hook"
        );
        assert_engine_matches_reference(&rules, &descriptions);
    }));
    TEST_SET_SIZE_LIMIT.with(|limit| limit.set(None));
    outcome.expect("fallback comparison failed");
}

/// An invalid regex among valid ones must not poison the whole set: the
/// valid rules keep deciding, the invalid one never matches — on the set
/// path if the valid ones still build, on the fallback if not.
#[test]
fn invalid_regex_excluded_from_a_valid_set() {
    let rules = [
        Rule {
            id: 1,
            pattern: "(unclosed".into(),
            is_regex: true,
            category_id: 10,
        },
        Rule {
            id: 2,
            pattern: r"CAFE\s+\w+".into(),
            is_regex: true,
            category_id: 20,
        },
        Rule {
            id: 3,
            pattern: r"\bBRAND\b".into(),
            is_regex: true,
            category_id: 30,
        },
    ];
    let descriptions: Vec<String> = [
        "CAFE O VANCOUVER",
        "best BRAND store",
        "UNCLOSED",
        "(unclosed (unclosed",
        "nothing",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_engine_matches_reference(&rules, &descriptions);
}

#[test]
fn duplicate_substring_needles_all_match() {
    // The overlapping Aho-Corasick search must report duplicate needles,
    // not just one of them — the tie is then broken by id, longest first.
    let rules = [
        Rule {
            id: 9,
            pattern: "COFFEE".into(),
            is_regex: false,
            category_id: 10,
        },
        Rule {
            id: 4,
            pattern: "COFFEE".into(),
            is_regex: false,
            category_id: 20,
        },
        Rule {
            id: 1,
            pattern: "COFFEE SHOP".into(),
            is_regex: false,
            category_id: 30,
        },
    ];
    let descriptions: Vec<String> = [
        "CORNER COFFEE SHOP",
        "TIM HORTONS COFFEE 4.50",
        "coffee",
        "none",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_engine_matches_reference(&rules, &descriptions);
    let compiled = CompiledRules::new(&rules);
    assert_eq!(compiled.winning("TIM HORTONS COFFEE"), Some(4));
    assert_eq!(compiled.winning("CORNER COFFEE SHOP"), Some(1));
}

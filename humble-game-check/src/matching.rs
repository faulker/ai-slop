//! Fuzzy matching of Humble key names against the Steam library. Names are
//! normalized (lowercased, punctuation dropped, edition suffixes stripped),
//! then compared by a mix of string edit distance and token overlap, with a
//! hard rule that sequel numerals must agree so "Portal" never matches
//! "Portal 2".

use crate::humble::HumbleKey;
use crate::steam::SteamIndex;
use serde::Serialize;
use std::collections::BTreeSet;

/// Similarity at or above this is reported as `LikelyOwned`.
pub const HIGH: f64 = 0.85;
/// Similarity at or above this (but below `HIGH`) is a `PossibleMatch`.
pub const LOW: f64 = 0.65;

/// Edition/marketing suffixes that don't change which game a name refers to.
/// Ordered longest first so multi-word phrases are removed before their
/// single-word parts. Numerals and words like "collection"/"bundle"/"pack"
/// are deliberately absent: those usually mean a different product.
const EDITION_PHRASES: &[&str] = &[
    "game of the year edition",
    "game of the year",
    "goty edition",
    "goty",
    "definitive edition",
    "deluxe edition",
    "complete edition",
    "ultimate edition",
    "gold edition",
    "enhanced edition",
    "special edition",
    "anniversary edition",
    "legendary edition",
    "directors cut",
    "remastered",
    "remaster",
    "steam key",
    "steam edition",
    "edition",
    "deluxe",
    "definitive",
    "complete",
    "ultimate",
];

const ROMAN_NUMERALS: &[&str] = &["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"];
const NUMBER_WORDS: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];

/// How confident we are that a Humble key is already owned on Steam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    OwnedExact,
    OwnedName,
    LikelyOwned,
    PossibleMatch,
    NotOwned,
}

/// The Steam library entry a key was matched (or nearly matched) against.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub appid: u32,
    pub name: String,
    pub score: f64,
}

/// The result of classifying one Humble key.
#[derive(Debug, Clone, Serialize)]
pub struct Match {
    pub verdict: Verdict,
    pub candidate: Option<Candidate>,
}

/// The normalization steps that don't touch meaning: lowercase, drop
/// trademark symbols and apostrophes, spell out `&`, turn all other
/// punctuation into spaces, and collapse whitespace. Used on its own to
/// decide whether two raw names were "the same" before edition stripping.
pub(crate) fn basic_normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.to_lowercase().chars() {
        match ch {
            '\u{2122}' | '\u{AE}' | '\u{A9}' | '\'' | '\u{2019}' => {}
            '&' => out.push_str(" and "),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Removes every whole-word occurrence of `phrase` from `toks`, unless doing
/// so would leave nothing. Returns whether anything was removed.
fn strip_phrase(toks: &mut Vec<String>, phrase: &[&str]) -> bool {
    let mut removed = false;
    let mut i = 0;
    while i + phrase.len() <= toks.len() {
        let window_matches = toks[i..i + phrase.len()]
            .iter()
            .zip(phrase)
            .all(|(t, p)| t == p);
        if window_matches && toks.len() > phrase.len() {
            toks.drain(i..i + phrase.len());
            removed = true;
        } else {
            i += 1;
        }
    }
    removed
}

/// Full normalization: `basic_normalize` followed by edition-phrase
/// stripping, repeated until nothing more comes off.
pub fn normalize(name: &str) -> String {
    let basic = basic_normalize(name);
    let mut toks = tokens(&basic);
    let phrases: Vec<Vec<&str>> = EDITION_PHRASES
        .iter()
        .map(|p| p.split(' ').collect())
        .collect();
    loop {
        let mut changed = false;
        for phrase in &phrases {
            changed |= strip_phrase(&mut toks, phrase);
        }
        if !changed {
            break;
        }
    }
    toks.join(" ")
}

/// Splits a normalized name into its whitespace-separated tokens.
pub fn tokens(norm: &str) -> Vec<String> {
    norm.split_whitespace().map(|s| s.to_string()).collect()
}

/// The tokens that mark a sequel or episode number: plain digits, roman
/// numerals up to ten, and spelled-out numbers up to ten.
pub fn sequel_markers(tokens: &[String]) -> BTreeSet<String> {
    tokens
        .iter()
        .filter(|t| {
            t.chars().all(|c| c.is_ascii_digit())
                || ROMAN_NUMERALS.contains(&t.as_str())
                || NUMBER_WORDS.contains(&t.as_str())
        })
        .cloned()
        .collect()
}

/// Scores two normalized names in `0.0..=1.0`. Differing sequel markers
/// force `0.0`. Otherwise the score is the better of normalized Levenshtein
/// on the strings and a token-set score: half Jaccard, half containment
/// (overlap divided by the smaller set), so a name that is a superset of the
/// other still scores well.
pub fn similarity(a_norm: &str, a_tokens: &[String], b_norm: &str, b_tokens: &[String]) -> f64 {
    if sequel_markers(a_tokens) != sequel_markers(b_tokens) {
        return 0.0;
    }
    let a_set: BTreeSet<&str> = a_tokens.iter().map(String::as_str).collect();
    let b_set: BTreeSet<&str> = b_tokens.iter().map(String::as_str).collect();
    let inter = a_set.intersection(&b_set).count() as f64;
    let union = a_set.union(&b_set).count() as f64;
    let smaller = a_set.len().min(b_set.len()) as f64;
    let token_score = if union == 0.0 || smaller == 0.0 {
        0.0
    } else {
        0.5 * (inter / union) + 0.5 * (inter / smaller)
    };
    let lev = strsim::normalized_levenshtein(a_norm, b_norm);
    lev.max(token_score)
}

/// Classifies one Humble key against the Steam library, in priority order:
/// Humble's own Steam app id, then an exact normalized-name hit, then the
/// best fuzzy score over every named library entry.
pub fn classify(key: &HumbleKey, index: &SteamIndex) -> Match {
    if let Some(appid) = key.app_id {
        if index.has_appid(appid) {
            // Delisted apps have no name in the library; report an empty one.
            let name = index
                .entries()
                .iter()
                .find(|e| e.appid == appid)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            return Match {
                verdict: Verdict::OwnedExact,
                candidate: Some(Candidate {
                    appid,
                    name,
                    score: 1.0,
                }),
            };
        }
    }

    let norm = normalize(&key.name);
    if let Some((appid, name)) = index.by_norm(&norm) {
        // If the names only agree once an edition suffix is gone, the key
        // may be a different SKU, so leave it visible as LikelyOwned.
        let verdict = if basic_normalize(&key.name) == basic_normalize(name) {
            Verdict::OwnedName
        } else {
            Verdict::LikelyOwned
        };
        return Match {
            verdict,
            candidate: Some(Candidate {
                appid: *appid,
                name: name.clone(),
                score: 1.0,
            }),
        };
    }

    let key_tokens = tokens(&norm);
    let mut best: Option<Candidate> = None;
    for entry in index.entries() {
        let score = similarity(&norm, &key_tokens, &entry.norm, &entry.tokens);
        if score > 0.0 && best.as_ref().map_or(true, |b| score > b.score) {
            best = Some(Candidate {
                appid: entry.appid,
                name: entry.name.clone(),
                score,
            });
        }
    }
    let verdict = match &best {
        Some(c) if c.score >= HIGH => Verdict::LikelyOwned,
        Some(c) if c.score >= LOW => Verdict::PossibleMatch,
        _ => Verdict::NotOwned,
    };
    Match {
        verdict,
        candidate: best,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam::OwnedGame;

    /// Builds a Humble key with the given name and optional app id.
    fn key(name: &str, app_id: Option<u32>) -> HumbleKey {
        HumbleKey {
            name: name.into(),
            machine_name: "test_steam".into(),
            order_name: "Test Bundle".into(),
            gamekey: "testkey".into(),
            app_id,
        }
    }

    /// Builds a Steam index from `(appid, name)` pairs; `None` names model
    /// delisted apps.
    fn index(games: &[(u32, Option<&str>)]) -> SteamIndex {
        let owned: Vec<OwnedGame> = games
            .iter()
            .map(|(appid, name)| OwnedGame {
                appid: *appid,
                name: name.map(str::to_string),
            })
            .collect();
        SteamIndex::build(&owned)
    }

    /// Scores two raw names through the same path `classify` uses.
    fn sim(a: &str, b: &str) -> f64 {
        let an = normalize(a);
        let bn = normalize(b);
        similarity(&an, &tokens(&an), &bn, &tokens(&bn))
    }

    #[test]
    fn normalize_strips_symbols_and_remastered() {
        assert_eq!(normalize("DARK SOULS\u{2122}: REMASTERED"), "dark souls");
    }

    #[test]
    fn normalize_strips_editions_to_same_base() {
        assert_eq!(normalize("Awesome Game 2 Deluxe"), "awesome game 2");
        assert_eq!(
            normalize("Awesome Game 2: Game of the Year Edition"),
            "awesome game 2"
        );
    }

    #[test]
    fn normalize_keeps_numerals() {
        assert_eq!(normalize("Portal 2"), "portal 2");
        assert_eq!(normalize("Dark Souls II"), "dark souls ii");
    }

    #[test]
    fn normalize_never_returns_empty() {
        assert_eq!(normalize("Remastered"), "remastered");
        assert_eq!(normalize("Deluxe Edition"), "deluxe");
    }

    #[test]
    fn normalize_spells_out_ampersand() {
        assert_eq!(normalize("Tom & Jerry"), "tom and jerry");
    }

    #[test]
    fn normalize_strips_directors_cut() {
        assert_eq!(normalize("Some Game Director's Cut"), "some game");
        assert_eq!(normalize("Some Game Director\u{2019}s Cut"), "some game");
    }

    #[test]
    fn normalize_keeps_collection() {
        assert_eq!(normalize("Complete Collection"), "collection");
    }

    #[test]
    fn basic_normalize_keeps_edition_words() {
        assert_eq!(
            basic_normalize("Awesome Game 2: Deluxe"),
            "awesome game 2 deluxe"
        );
    }

    #[test]
    fn sequel_markers_find_digits_and_words() {
        let toks = tokens("half life 2 episode one");
        let markers = sequel_markers(&toks);
        let expected: BTreeSet<String> = ["2", "one"].iter().map(|s| s.to_string()).collect();
        assert_eq!(markers, expected);
    }

    #[test]
    fn similarity_superset_scores_by_tokens() {
        let s = sim("awesome game 2 ultra pack", "awesome game 2");
        assert!((s - 0.80).abs() < 0.01, "got {s}");
    }

    #[test]
    fn similarity_zero_when_sequel_markers_differ() {
        assert_eq!(sim("portal", "portal 2"), 0.0);
        assert_eq!(sim("left 4 dead", "left 4 dead 2"), 0.0);
    }

    #[test]
    fn similarity_unrelated_names_below_low() {
        let s = sim("dead cells", "dead space");
        assert!(s < LOW, "got {s}");
    }

    #[test]
    fn classify_exact_appid_wins_over_mismatched_name() {
        let idx = index(&[(620, Some("Portal 2"))]);
        let m = classify(&key("Totally Different", Some(620)), &idx);
        assert_eq!(m.verdict, Verdict::OwnedExact);
        let c = m.candidate.unwrap();
        assert_eq!(c.appid, 620);
        assert_eq!(c.name, "Portal 2");
        assert_eq!(c.score, 1.0);
    }

    #[test]
    fn classify_identical_name_is_owned_name() {
        let idx = index(&[(570, Some("DARK SOULS\u{2122}: REMASTERED"))]);
        let m = classify(&key("DARK SOULS\u{2122}: REMASTERED", None), &idx);
        assert_eq!(m.verdict, Verdict::OwnedName);
        assert_eq!(m.candidate.unwrap().appid, 570);
    }

    #[test]
    fn classify_edition_suffix_is_likely_owned() {
        let idx = index(&[(570, Some("Awesome Game 2"))]);
        let m = classify(&key("Awesome Game 2 Deluxe", None), &idx);
        assert_eq!(m.verdict, Verdict::LikelyOwned);
        let c = m.candidate.unwrap();
        assert_eq!(c.appid, 570);
        assert_eq!(c.name, "Awesome Game 2");
        assert_eq!(c.score, 1.0);
    }

    #[test]
    fn classify_superset_name_is_possible_match() {
        let idx = index(&[(570, Some("Awesome Game 2"))]);
        let m = classify(&key("Awesome Game 2 Ultra Pack", None), &idx);
        assert_eq!(m.verdict, Verdict::PossibleMatch);
        let c = m.candidate.unwrap();
        assert_eq!(c.name, "Awesome Game 2");
        assert!((c.score - 0.80).abs() < 0.01, "got {}", c.score);
    }

    #[test]
    fn classify_sequel_is_not_owned_without_candidate() {
        let idx = index(&[(400, Some("Portal"))]);
        let m = classify(&key("Portal 2", None), &idx);
        assert_eq!(m.verdict, Verdict::NotOwned);
        assert!(m.candidate.is_none());
    }

    #[test]
    fn classify_nameless_app_matches_only_by_appid() {
        let idx = index(&[(999, None)]);
        let m = classify(&key("Anything", Some(999)), &idx);
        assert_eq!(m.verdict, Verdict::OwnedExact);
        let c = m.candidate.unwrap();
        assert_eq!(c.appid, 999);
        assert_eq!(c.name, "");

        let m = classify(&key("Anything", None), &idx);
        assert_eq!(m.verdict, Verdict::NotOwned);
        assert!(m.candidate.is_none());
    }

    #[test]
    fn classify_not_owned_still_reports_best_candidate() {
        let idx = index(&[(1, Some("Dead Space")), (2, Some("Portal"))]);
        let m = classify(&key("Dead Cells", None), &idx);
        assert_eq!(m.verdict, Verdict::NotOwned);
        let c = m.candidate.unwrap();
        assert_eq!(c.name, "Dead Space");
        assert!(c.score > 0.0 && c.score < LOW);
    }

    #[test]
    fn classify_unowned_appid_does_not_block_name_match() {
        let idx = index(&[(570, Some("Awesome Game"))]);
        let m = classify(&key("Awesome Game", Some(4242)), &idx);
        assert_eq!(m.verdict, Verdict::OwnedName);
        assert_eq!(m.candidate.unwrap().appid, 570);
    }

    // Boundary cases use subset names so the token score is exact:
    // containment is 1 and Jaccard is |short| / |long|.
    const TEN_WORDS: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
    const ELEVEN_WORDS: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";

    #[test]
    fn boundary_exactly_high_is_likely_owned() {
        let seven = "alpha beta gamma delta epsilon zeta eta";
        let s = sim(seven, TEN_WORDS);
        assert!(s >= HIGH && s < HIGH + 1e-9, "got {s}");
        let m = classify(&key(seven, None), &index(&[(1, Some(TEN_WORDS))]));
        assert_eq!(m.verdict, Verdict::LikelyOwned);
    }

    #[test]
    fn boundary_just_under_high_is_possible_match() {
        let seven = "alpha beta gamma delta epsilon zeta eta";
        let s = sim(seven, ELEVEN_WORDS);
        assert!(s < HIGH && s >= LOW, "got {s}");
        let m = classify(&key(seven, None), &index(&[(1, Some(ELEVEN_WORDS))]));
        assert_eq!(m.verdict, Verdict::PossibleMatch);
    }

    #[test]
    fn boundary_exactly_low_is_possible_match() {
        let three = "alpha beta gamma";
        let s = sim(three, TEN_WORDS);
        assert!(s >= LOW && s < LOW + 1e-9, "got {s}");
        let m = classify(&key(three, None), &index(&[(1, Some(TEN_WORDS))]));
        assert_eq!(m.verdict, Verdict::PossibleMatch);
    }

    #[test]
    fn boundary_just_under_low_is_not_owned_with_candidate() {
        let three = "alpha beta gamma";
        let s = sim(three, ELEVEN_WORDS);
        assert!(s < LOW && s > 0.0, "got {s}");
        let m = classify(&key(three, None), &index(&[(1, Some(ELEVEN_WORDS))]));
        assert_eq!(m.verdict, Verdict::NotOwned);
        assert!(m.candidate.is_some());
    }
}

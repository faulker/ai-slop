//! Renders classified Humble keys as either a human-readable text report or
//! structured JSON. Both formats group rows by verdict and hide the OWNED
//! group unless `show_owned` is set.

use crate::error::{AppError, Result};
use crate::humble::HumbleKey;
use crate::matching::{Candidate, Match, Verdict};
use serde::Serialize;

/// One classified Humble key: the key itself plus how it scored against the
/// Steam library.
pub struct Row {
    pub key: HumbleKey,
    pub m: Match,
}

/// Appends one report section (header, padded rows, trailing blank line) to
/// `out`. Empty sections still print their header with a `(0)` count.
fn render_section(out: &mut String, title: &str, rows: &[&Row]) {
    out.push_str(&format!("{title} ({})\n", rows.len()));
    let width = rows
        .iter()
        .map(|r| r.key.name.chars().count())
        .max()
        .unwrap_or(0);
    for row in rows {
        out.push_str("  ");
        out.push_str(&format!("{:<width$}", row.key.name, width = width));
        out.push(' ');
        out.push_str(&row.key.humble_url());
        if let Some(c) = &row.m.candidate {
            out.push_str(&format!("   best: {} ({}) {:.2}", c.name, c.appid, c.score));
        }
        out.push('\n');
    }
    out.push('\n');
}

/// Renders the plain-text report: NOT OWNED, POSSIBLE MATCH, LIKELY OWNED,
/// then OWNED (only with `show_owned`), followed by a count footer.
pub fn render_text(rows: &[Row], show_owned: bool) -> String {
    let mut not_owned = Vec::new();
    let mut possible = Vec::new();
    let mut likely_owned = Vec::new();
    let mut owned = Vec::new();
    for row in rows {
        match row.m.verdict {
            Verdict::NotOwned => not_owned.push(row),
            Verdict::PossibleMatch => possible.push(row),
            Verdict::LikelyOwned => likely_owned.push(row),
            Verdict::OwnedExact | Verdict::OwnedName => owned.push(row),
        }
    }

    let mut out = String::new();
    render_section(&mut out, "NOT OWNED", &not_owned);
    render_section(&mut out, "POSSIBLE MATCH, review", &possible);
    render_section(
        &mut out,
        "LIKELY OWNED, edition or near-name match",
        &likely_owned,
    );
    if show_owned {
        render_section(&mut out, "OWNED", &owned);
    }

    out.push_str(&format!(
        "total unrevealed steam keys: {}  not owned: {}  possible: {}  likely owned: {}  owned: {}",
        rows.len(),
        not_owned.len(),
        possible.len(),
        likely_owned.len(),
        owned.len(),
    ));
    out
}

/// One row as it appears in the JSON report. `redeemed_key_val` never made it
/// this far (see `HumbleKey`), so there is nothing to redact here.
#[derive(Serialize)]
struct JsonEntry<'a> {
    name: &'a str,
    order_name: &'a str,
    gamekey: &'a str,
    machine_name: &'a str,
    app_id: Option<u32>,
    humble_url: String,
    verdict: Verdict,
    candidate: Option<Candidate>,
}

/// Counts mirroring the text report's footer.
#[derive(Serialize)]
struct Counts {
    total: usize,
    not_owned: usize,
    possible: usize,
    likely_owned: usize,
    owned: usize,
}

/// Top-level JSON shape: one array per verdict group plus `counts`.
#[derive(Serialize)]
struct Output<'a> {
    not_owned: Vec<JsonEntry<'a>>,
    possible: Vec<JsonEntry<'a>>,
    likely_owned: Vec<JsonEntry<'a>>,
    owned: Vec<JsonEntry<'a>>,
    counts: Counts,
}

/// Renders the JSON report. `counts` always reflects the real number of owned
/// keys; the `owned` array itself is emptied when `show_owned` is false.
pub fn render_json(rows: &[Row], show_owned: bool) -> Result<String> {
    let mut not_owned = Vec::new();
    let mut possible = Vec::new();
    let mut likely_owned = Vec::new();
    let mut owned = Vec::new();

    for row in rows {
        let entry = JsonEntry {
            name: &row.key.name,
            order_name: &row.key.order_name,
            gamekey: &row.key.gamekey,
            machine_name: &row.key.machine_name,
            app_id: row.key.app_id,
            humble_url: row.key.humble_url(),
            verdict: row.m.verdict,
            candidate: row.m.candidate.clone(),
        };
        match row.m.verdict {
            Verdict::NotOwned => not_owned.push(entry),
            Verdict::PossibleMatch => possible.push(entry),
            Verdict::LikelyOwned => likely_owned.push(entry),
            Verdict::OwnedExact | Verdict::OwnedName => owned.push(entry),
        }
    }

    let counts = Counts {
        total: rows.len(),
        not_owned: not_owned.len(),
        possible: possible.len(),
        likely_owned: likely_owned.len(),
        owned: owned.len(),
    };
    if !show_owned {
        owned.clear();
    }

    let output = Output {
        not_owned,
        possible,
        likely_owned,
        owned,
        counts,
    };
    serde_json::to_string_pretty(&output).map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam::{OwnedGame, SteamIndex};

    /// Builds a Humble key with the given name; app id/gamekey are fixed
    /// unique-enough placeholders for these tests.
    fn key(name: &str, gamekey: &str) -> HumbleKey {
        HumbleKey {
            name: name.into(),
            machine_name: "test_steam".into(),
            order_name: "Test Bundle".into(),
            gamekey: gamekey.into(),
            app_id: None,
        }
    }

    /// Builds a small set of rows covering every verdict bucket: one
    /// NOT OWNED, one POSSIBLE MATCH, one LIKELY OWNED, one OWNED.
    fn sample_rows() -> Vec<Row> {
        let steam_games = vec![
            OwnedGame {
                appid: 1,
                name: Some("Awesome Game 2".into()),
            },
            OwnedGame {
                appid: 2,
                name: Some("Awesome Game".into()),
            },
        ];
        let index = SteamIndex::build(&steam_games);

        let not_owned = key("Some Indie Title", "keyNotOwned");
        let possible = key("Awesome Game 2 Ultra Pack", "keyPossible");
        let likely = key("Awesome Game 2 Deluxe", "keyLikely");
        let owned = key("Awesome Game", "keyOwned");

        vec![
            Row {
                m: crate::matching::classify(&not_owned, &index),
                key: not_owned,
            },
            Row {
                m: crate::matching::classify(&possible, &index),
                key: possible,
            },
            Row {
                m: crate::matching::classify(&likely, &index),
                key: likely,
            },
            Row {
                m: crate::matching::classify(&owned, &index),
                key: owned,
            },
        ]
    }

    #[test]
    fn section_headers_appear_in_order() {
        let text = render_text(&sample_rows(), true);
        let not_owned_pos = text.find("NOT OWNED (1)").unwrap();
        let possible_pos = text.find("POSSIBLE MATCH, review (1)").unwrap();
        let likely_pos = text
            .find("LIKELY OWNED, edition or near-name match (1)")
            .unwrap();
        // "OWNED (1)" is also a substring of "NOT OWNED (1)", so anchor the
        // search after the LIKELY OWNED header to find the real OWNED one.
        let owned_pos = likely_pos + text[likely_pos..].find("OWNED (1)").unwrap();
        assert!(not_owned_pos < possible_pos);
        assert!(possible_pos < likely_pos);
        assert!(likely_pos < owned_pos);
    }

    #[test]
    fn owned_section_hidden_without_show_owned() {
        let text = render_text(&sample_rows(), false);
        assert!(!text.contains("\nOWNED ("), "got: {text}");
        assert!(text.contains("NOT OWNED (1)"));
    }

    #[test]
    fn empty_sections_still_print_their_header() {
        let text = render_text(&[], true);
        assert!(text.contains("NOT OWNED (0)"));
        assert!(text.contains("POSSIBLE MATCH, review (0)"));
        assert!(text.contains("LIKELY OWNED, edition or near-name match (0)"));
        assert!(text.contains("OWNED (0)"));
    }

    #[test]
    fn row_contains_humble_url_with_gamekey() {
        let text = render_text(&sample_rows(), true);
        assert!(text.contains("https://www.humblebundle.com/downloads?key=keyNotOwned"));
    }

    #[test]
    fn names_are_padded_so_urls_align() {
        let rows = vec![
            Row {
                key: key("Short", "k1"),
                m: Match {
                    verdict: Verdict::NotOwned,
                    candidate: None,
                },
            },
            Row {
                key: key("A Much Longer Title", "k2"),
                m: Match {
                    verdict: Verdict::NotOwned,
                    candidate: None,
                },
            },
        ];
        let text = render_text(&rows, false);
        let url_col: Vec<usize> = text
            .lines()
            .filter_map(|line| line.find("https://"))
            .collect();
        assert_eq!(url_col.len(), 2);
        assert_eq!(
            url_col[0], url_col[1],
            "urls should start at the same column: {text}"
        );
    }

    #[test]
    fn footer_reports_counts() {
        let text = render_text(&sample_rows(), true);
        assert!(text.contains("total unrevealed steam keys: 4"));
        assert!(text.contains("not owned: 1"));
        assert!(text.contains("possible: 1"));
        assert!(text.contains("likely owned: 1"));
        assert!(text.contains("owned: 1"));
    }

    #[test]
    fn json_round_trips_with_expected_keys() {
        let json = render_json(&sample_rows(), true).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["not_owned"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["possible"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["likely_owned"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["owned"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["counts"]["total"], 4);

        let entry = &parsed["not_owned"][0];
        for field in [
            "name",
            "order_name",
            "gamekey",
            "machine_name",
            "app_id",
            "humble_url",
            "verdict",
            "candidate",
        ] {
            assert!(entry.get(field).is_some(), "missing field {field}: {entry}");
        }
    }

    #[test]
    fn json_owned_is_empty_without_show_owned() {
        let json = render_json(&sample_rows(), false).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["owned"].as_array().unwrap().len(), 0);
        // Counts still reflect the real number.
        assert_eq!(parsed["counts"]["owned"], 1);
    }
}

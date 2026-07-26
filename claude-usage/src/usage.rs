//! Normalized usage model shared by both sources, plus its text rendering.
//!
//! The JSON serialization of `Usage` is the documented contract other apps read
//! (see the README). Keys are additive-only, and `null` always means "this
//! source does not report the field", never "zero".

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where a `Usage` came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Local,
    Api,
}

/// One usage window (5-hour session or 7-day).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    pub used_percentage: f64,
    /// Authoritative reset time from the API, as an opaque ISO string. Never
    /// parsed, and always `None` on the local source.
    pub resets_at: Option<String>,
    /// Reset time derived from the local history's observed window
    /// boundaries. Populated on both sources: it's the only source for the
    /// local source, and a best-effort supplement on the api source, which
    /// otherwise only carries the authoritative `resets_at` above.
    pub resets_at_estimated_ms: Option<i64>,
}

impl Window {
    /// A window with a percentage and nothing else known about it.
    pub fn from_percentage(used_percentage: f64) -> Self {
        Window {
            used_percentage,
            resets_at: None,
            resets_at_estimated_ms: None,
        }
    }
}

/// Usage normalized across sources.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Usage {
    pub source: Source,
    /// When the underlying measurement was taken (epoch ms). For the api source
    /// this is the time of the request.
    pub sampled_at_ms: i64,
    pub age_seconds: i64,
    pub stale: bool,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub extra_usage_dollars: Option<f64>,
}

/// Current wall-clock time in epoch milliseconds.
///
/// Clamps to 0 for pre-epoch clocks rather than panicking; a nonsense clock
/// should degrade the age display, not kill the process.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Renders an elapsed duration as a short relative age ("4m", "3h 12m").
pub fn format_age(seconds: i64) -> String {
    if seconds <= 0 {
        return "just now".to_string();
    }
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3600 {
        return format!("{}m", seconds / 60);
    }
    if seconds < 86_400 {
        let (h, m) = (seconds / 3600, (seconds % 3600) / 60);
        if m == 0 {
            return format!("{h}h");
        }
        return format!("{h}h {m}m");
    }
    let (d, h) = (seconds / 86_400, (seconds % 86_400) / 3600);
    if h == 0 {
        format!("{d}d")
    } else {
        format!("{d}d {h}h")
    }
}

/// Renders a remaining duration for the estimated-reset column.
pub fn format_until(seconds: i64) -> String {
    if seconds <= 0 {
        return "now".to_string();
    }
    format_age(seconds)
}

/// Formats one window line, e.g. "5h window   79% used   resets in ~1h 20m (est)".
fn render_window(label: &str, window: &Window, now: i64) -> String {
    let mut line = format!("{label:<11}{:>3.0}% used", window.used_percentage);
    if let Some(resets_at) = &window.resets_at {
        line.push_str(&format!("   resets at {resets_at}"));
    } else if let Some(reset_ms) = window.resets_at_estimated_ms {
        let remaining = (reset_ms - now) / 1000;
        line.push_str(&format!("   resets in ~{} (est)", format_until(remaining)));
    }
    line
}

/// Renders the human-readable default output.
pub fn render_text(usage: &Usage) -> String {
    let now = usage.sampled_at_ms + usage.age_seconds * 1000;
    let mut lines = Vec::new();

    for (label, window) in [
        ("5h window", &usage.five_hour),
        ("7d window", &usage.seven_day),
    ] {
        if let Some(window) = window {
            lines.push(render_window(label, window, now));
        }
    }

    if let Some(dollars) = usage.extra_usage_dollars {
        lines.push(format!("extra usage: ${dollars:.2}"));
    }

    match usage.source {
        Source::Local => {
            lines.push(format!(
                "source: local, sampled {} ago",
                format_age(usage.age_seconds)
            ));
            let estimated = usage
                .five_hour
                .as_ref()
                .is_some_and(|w| w.resets_at_estimated_ms.is_some());
            if estimated {
                lines.push("note: reset time is estimated".to_string());
            } else {
                lines.push("note: local source has no reset time".to_string());
            }
        }
        Source::Api => lines.push("source: api, live".to_string()),
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_usage(five_hour: Window) -> Usage {
        Usage {
            source: Source::Local,
            sampled_at_ms: 1_784_989_528_179,
            age_seconds: 240,
            stale: false,
            five_hour: Some(five_hour),
            seven_day: Some(Window::from_percentage(18.0)),
            extra_usage_dollars: None,
        }
    }

    #[test]
    fn format_age_covers_each_scale() {
        assert_eq!(format_age(0), "just now");
        assert_eq!(format_age(45), "45s");
        assert_eq!(format_age(90), "1m");
        assert_eq!(format_age(3600), "1h");
        assert_eq!(format_age(12_720), "3h 32m");
        assert_eq!(format_age(93_600), "1d 2h");
        assert_eq!(format_age(450_000), "5d 5h");
    }

    #[test]
    fn format_age_handles_negative_clock_skew() {
        assert_eq!(format_age(-30), "just now");
    }

    #[test]
    fn format_until_says_now_at_zero() {
        assert_eq!(format_until(0), "now");
        assert_eq!(format_until(4800), "1h 20m");
    }

    #[test]
    fn render_text_local_with_estimate() {
        let sampled = 1_784_989_528_179_i64;
        let usage = local_usage(Window {
            used_percentage: 79.0,
            resets_at: None,
            // 1h 20m after "now" (sampled + 240s)
            resets_at_estimated_ms: Some(sampled + 240_000 + 4_800_000),
        });
        let out = render_text(&usage);
        assert!(out.contains("5h window"), "got: {out}");
        assert!(out.contains("79% used"), "got: {out}");
        assert!(out.contains("resets in ~1h 20m (est)"), "got: {out}");
        assert!(out.contains("source: local, sampled 4m ago"), "got: {out}");
        assert!(out.contains("reset time is estimated"), "got: {out}");
        assert!(
            !out.contains("resets at "),
            "must not claim a real reset time"
        );
    }

    #[test]
    fn render_text_local_without_estimate() {
        let out = render_text(&local_usage(Window::from_percentage(0.0)));
        assert!(out.contains("0% used"), "got: {out}");
        assert!(
            !out.contains("resets"),
            "no reset column when unknown: {out}"
        );
        assert!(out.contains("no reset time"), "got: {out}");
    }

    #[test]
    fn render_text_api_shows_the_real_reset_time() {
        let usage = Usage {
            source: Source::Api,
            sampled_at_ms: 1_784_989_528_179,
            age_seconds: 0,
            stale: false,
            five_hour: Some(Window {
                used_percentage: 79.0,
                resets_at: Some("2026-07-25T20:00:00Z".into()),
                // Best-effort local supplement; resets_at takes priority in text.
                resets_at_estimated_ms: Some(1_784_989_528_179 + 4_800_000),
            }),
            seven_day: Some(Window::from_percentage(18.0)),
            extra_usage_dollars: None,
        };
        let out = render_text(&usage);
        assert!(out.contains("resets at 2026-07-25T20:00:00Z"), "got: {out}");
        assert!(out.contains("source: api, live"), "got: {out}");
        assert!(
            !out.contains("(est)"),
            "the real resets_at takes priority over the estimate in text: {out}"
        );
    }

    #[test]
    fn render_text_shows_extra_usage_dollars() {
        let mut usage = local_usage(Window::from_percentage(0.0));
        usage.extra_usage_dollars = Some(41.66);
        assert!(render_text(&usage).contains("extra usage: $41.66"));
    }

    #[test]
    fn json_contract_keys_are_all_present_with_nulls() {
        let usage = local_usage(Window::from_percentage(0.0));
        let value = serde_json::to_value(&usage).unwrap();
        let obj = value.as_object().unwrap();

        for key in [
            "source",
            "sampled_at_ms",
            "age_seconds",
            "stale",
            "five_hour",
            "seven_day",
            "extra_usage_dollars",
        ] {
            assert!(obj.contains_key(key), "missing contract key {key}");
        }
        assert_eq!(obj["source"], "local");
        assert!(
            obj["extra_usage_dollars"].is_null(),
            "nulls must be serialized"
        );

        let five_hour = obj["five_hour"].as_object().unwrap();
        for key in ["used_percentage", "resets_at", "resets_at_estimated_ms"] {
            assert!(five_hour.contains_key(key), "missing window key {key}");
        }
        assert!(five_hour["resets_at"].is_null());
    }
}

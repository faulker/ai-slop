//! The live source: the Anthropic OAuth usage endpoint.
//!
//! Has an authoritative reset time, unlike the local file, but needs the
//! Keychain token, which is what triggers the macOS password prompt. Fetch
//! and parse are separate so the parsing rules are testable offline.
//!
//! The response also carries `seven_day_opus` / `seven_day_sonnet` fields, but
//! they're consistently `null` in practice (verified against a live response;
//! the `limits` array has no per-model entries either), so this tool doesn't
//! surface them.

use crate::error::{AppError, Result};
use crate::usage::{Source, Usage, Window};
use serde::Deserialize;

pub const DEFAULT_API_BASE: &str = "https://api.anthropic.com";

/// Every field is optional so a change on the API side degrades the output
/// instead of failing the run. Unlisted response fields (`limits`, `spend`,
/// `cinder_cove`, and friends) are ignored; use `--format raw` to see them.
#[derive(Debug, Default, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    five_hour: Option<ApiWindow>,
    #[serde(default)]
    seven_day: Option<ApiWindow>,
    #[serde(default)]
    extra_usage: Option<ApiExtraUsage>,
}

#[derive(Debug, Deserialize)]
struct ApiWindow {
    /// Used percentage. The wire name is `utilization`; the alias covers the
    /// `used_percentage` spelling that Claude Code uses internally.
    #[serde(default, alias = "used_percentage")]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

impl ApiWindow {
    /// Drops windows carrying no percentage; a window without a number tells the
    /// caller nothing and would render as a misleading "0% used".
    fn to_window(&self) -> Option<Window> {
        Some(Window {
            used_percentage: self.utilization?,
            resets_at: self.resets_at.clone(),
            resets_at_estimated_ms: None,
        })
    }
}

/// Extra usage spend. `used_credits` is in minor units, scaled by
/// `decimal_places` (2 for USD), matching the `spend.used.amount_minor` /
/// `exponent` pair in the same response.
#[derive(Debug, Deserialize)]
struct ApiExtraUsage {
    #[serde(default)]
    used_credits: Option<f64>,
    #[serde(default)]
    decimal_places: Option<u32>,
}

impl ApiExtraUsage {
    /// Converts credits to whole currency units, so the value is comparable to
    /// the local source's `xu`.
    fn to_dollars(&self) -> Option<f64> {
        let scale = 10_f64.powi(self.decimal_places.unwrap_or(2) as i32);
        Some(self.used_credits? / scale)
    }
}

/// Performs the usage request and returns the raw response body.
pub fn fetch_usage_body(base_url: &str, token: &str) -> Result<String> {
    let url = format!("{}/api/oauth/usage", base_url.trim_end_matches('/'));

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Content-Type", "application/json")
        .header("User-Agent", "claude-code/2.1.62")
        .send()?;

    let status = resp.status();
    let body = resp.text()?;

    if !status.is_success() {
        return Err(AppError::Http {
            msg: format!("API returned {status}: {body}"),
        });
    }

    Ok(body)
}

/// Normalizes a usage response body.
pub fn parse_api_usage(body: &str, now_ms: i64) -> Result<Usage> {
    let parsed: ApiResponse = serde_json::from_str(body)?;

    Ok(Usage {
        source: Source::Api,
        sampled_at_ms: now_ms,
        age_seconds: 0,
        stale: false,
        five_hour: parsed.five_hour.as_ref().and_then(ApiWindow::to_window),
        seven_day: parsed.seven_day.as_ref().and_then(ApiWindow::to_window),
        extra_usage_dollars: parsed
            .extra_usage
            .as_ref()
            .and_then(ApiExtraUsage::to_dollars),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real response: `utilization` is the wire field name, the
    /// per-model windows are present but null, and `limits`/`spend` are ignored.
    const BODY: &str = r#"{
        "five_hour": { "utilization": 60.0, "resets_at": "2026-07-25T19:19:59.113565+00:00",
                       "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
        "seven_day": { "utilization": 24.0, "resets_at": "2026-07-31T22:00:00.113586+00:00",
                       "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
        "seven_day_opus": null,
        "seven_day_sonnet": null,
        "cinder_cove": null,
        "extra_usage": { "is_enabled": true, "monthly_limit": null, "used_credits": 13777.0,
                         "utilization": null, "currency": "USD", "decimal_places": 2,
                         "disabled_reason": null, "user_disabled": false },
        "limits": [],
        "spend": { "used": { "amount_minor": 13777, "currency": "USD", "exponent": 2 } },
        "member_dashboard_available": false
    }"#;

    #[test]
    fn parse_api_usage_reads_windows_and_reset_times() {
        let usage = parse_api_usage(BODY, 1_784_989_528_179).expect("should parse");

        assert_eq!(usage.source, Source::Api);
        assert_eq!(usage.age_seconds, 0);
        assert!(!usage.stale);

        let five_hour = usage.five_hour.expect("five_hour window");
        assert_eq!(five_hour.used_percentage, 60.0);
        assert_eq!(
            five_hour.resets_at.as_deref(),
            Some("2026-07-25T19:19:59.113565+00:00")
        );
        assert!(
            five_hour.resets_at_estimated_ms.is_none(),
            "api never estimates"
        );

        assert_eq!(usage.seven_day.unwrap().used_percentage, 24.0);
    }

    #[test]
    fn parse_api_usage_converts_credits_to_dollars() {
        let usage = parse_api_usage(BODY, 1).expect("should parse");
        assert_eq!(usage.extra_usage_dollars, Some(137.77));
    }

    #[test]
    fn parse_api_usage_defaults_credit_scale_to_two_places() {
        let usage = parse_api_usage(r#"{"extra_usage":{"used_credits":4166.0}}"#, 1).unwrap();
        assert_eq!(usage.extra_usage_dollars, Some(41.66));
    }

    #[test]
    fn parse_api_usage_accepts_the_used_percentage_alias() {
        let usage = parse_api_usage(r#"{"five_hour":{"used_percentage":79.0}}"#, 1).unwrap();
        assert_eq!(usage.five_hour.unwrap().used_percentage, 79.0);
    }

    #[test]
    fn parse_api_usage_tolerates_an_empty_object() {
        let usage = parse_api_usage("{}", 1).expect("must not fail on unknown shapes");
        assert!(usage.five_hour.is_none());
        assert!(usage.seven_day.is_none());
        assert!(usage.extra_usage_dollars.is_none());
    }

    #[test]
    fn parse_api_usage_skips_a_window_without_a_percentage() {
        let usage = parse_api_usage(r#"{"five_hour":{"resets_at":"2026-07-25T20:00:00Z"}}"#, 1)
            .expect("should parse");
        assert!(
            usage.five_hour.is_none(),
            "a window with no percentage must not render as 0%"
        );
    }

    #[test]
    fn parse_api_usage_rejects_malformed_json() {
        let err = parse_api_usage("not json", 1).expect_err("should fail");
        assert!(matches!(err, AppError::JsonParse { .. }));
    }

    #[test]
    fn fetch_usage_body_reports_a_connection_failure() {
        // Port 1 refuses connections, so this exercises the transport error path.
        let err = fetch_usage_body("http://127.0.0.1:1", "test-token").expect_err("should fail");
        assert!(matches!(err, AppError::Http { .. }));
    }
}

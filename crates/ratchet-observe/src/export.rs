//! Static HTML report generation.
//!
//! Produces a single self-contained `.html` file with the
//! [`ObserveReport`] JSON inlined as a `<script type="application/json">`
//! block and rendered client-side. Zero external dependencies — the
//! file opens offline and can be attached to a Slack / GitHub comment
//! without a server.
//!
//! The template lives alongside this module (`export_template.html`)
//! so designers can iterate on it without rebuilding the Rust code —
//! `include_str!` pulls the bytes at compile time.

use anyhow::{Context, Result};

use crate::report::ObserveReport;

/// Compile-time template. `{{PROGRAM_NAME}}` / `{{REPORT_JSON}}` are
/// the two substitution points.
const TEMPLATE: &str = include_str!("export_template.html");

/// Render a self-contained HTML report for `report`. Returns the full
/// bytes ready to write to disk.
pub fn render_html(report: &ObserveReport) -> Result<String> {
    let payload = serde_json::to_string(report).context("serialising report for HTML embed")?;
    let name = report.program_name.as_deref().unwrap_or("<unnamed>");
    // `<script type="application/json">` only needs us to escape
    // `</script>` substrings. serde_json::to_string produces valid
    // JSON with no raw `<` / `&`, so the only realistic escape is the
    // close-tag sequence.
    let safe_payload = payload.replace("</script>", r#"<\/script>"#);
    let safe_name = html_escape(name);
    Ok(TEMPLATE
        .replace("{{REPORT_JSON}}", &safe_payload)
        .replace("{{PROGRAM_NAME}}", &safe_name))
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::IxMetrics;
    use crate::report::{ObserveReport, ObserveWindow};

    fn sample() -> ObserveReport {
        ObserveReport {
            program_id: "PROG".into(),
            program_name: Some("escrow".into()),
            window: ObserveWindow {
                seconds: 86_400,
                tx_count: 1_200,
                earliest_block_time: None,
                latest_block_time: None,
            },
            instructions: vec![IxMetrics {
                name: "deposit".into(),
                count: 900,
                success_count: 895,
                error_count: 5,
                success_rate: Some(895.0 / 900.0),
                cu_p50: Some(42_000),
                cu_p95: Some(47_000),
                cu_p99: Some(52_000),
            }],
            errors: vec![],
            recent_failures: vec![],
            upgrade_history: None,
            account_counts: vec![],
        }
    }

    #[test]
    fn html_renders_contain_program_name_and_payload() {
        let html = render_html(&sample()).unwrap();
        assert!(html.contains("escrow"));
        assert!(html.contains("PROG"));
        assert!(html.contains(r#""program_name":"escrow""#));
        assert!(!html.contains("{{REPORT_JSON}}"));
        assert!(!html.contains("{{PROGRAM_NAME}}"));
    }

    #[test]
    fn script_close_tag_in_payload_is_escaped() {
        // Exercise the escape path using a crafted program name —
        // serde_json won't emit `</script>` on its own, but this
        // defends against future payload fields that could.
        let mut r = sample();
        r.program_name = Some("</script><script>alert(1)</script>".into());
        let html = render_html(&r).unwrap();
        // Both embed paths (HTML body heading + JSON payload) must
        // neutralise the close-tag.
        assert!(!html.contains("</script>\""));
        // The JSON payload's embedded close-tag must be escaped.
        assert!(html.contains(r#"<\/script>"#));
        // The heading still shows the escaped visual form.
        assert!(html.contains("&lt;/script&gt;"));
    }

    #[test]
    fn html_escape_covers_canonical_entities() {
        assert_eq!(
            html_escape("<a href=\"x&y\">'q'</a>"),
            "&lt;a href=&quot;x&amp;y&quot;&gt;&#39;q&#39;&lt;/a&gt;"
        );
    }
}

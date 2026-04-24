//! Post-report alert evaluation.
//!
//! Callers (CLI, hosted dashboard, MCP server) describe threshold
//! checks by constructing an [`AlertConfig`]; [`evaluate`] returns a
//! list of concrete [`AlertBreach`]es with enough context for the
//! operator to act on each one. The CLI prints and exits non-zero; a
//! hosted dashboard might forward to a Slack webhook; an MCP server
//! returns the same struct verbatim. One rule set, three surfaces.
//!
//! The surface is deliberately narrow in this first pass — error
//! rate, min tx volume, CU p99 ceiling. The structured-flag story
//! covers ~90% of "is my program limping?" without a parser. A real
//! DSL lands later if we outgrow the flags.

use serde::{Deserialize, Serialize};

use crate::report::ObserveReport;

/// Threshold configuration. All fields optional — omitted fields
/// don't fire.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertConfig {
    /// Fail when any instruction's error rate exceeds this value
    /// (expressed as a percentage, e.g. `5.0` for 5%).
    pub max_error_rate_pct: Option<f64>,
    /// Limit the `max_error_rate_pct` check to a single instruction
    /// name. `None` means "any instruction in the report."
    pub error_rate_ix: Option<String>,
    /// Fail when the overall transaction count in the window is below
    /// this floor. Catches outages / dropped traffic.
    pub min_tx_count: Option<usize>,
    /// Fail when any instruction's CU p99 exceeds this value. Catches
    /// post-deploy efficiency regressions. `None` disables the check.
    pub max_cu_p99: Option<u64>,
    /// Limit the `max_cu_p99` check to a single instruction.
    pub cu_p99_ix: Option<String>,
}

impl AlertConfig {
    /// `true` when no threshold was configured, so `evaluate` can
    /// short-circuit without scanning the report.
    pub fn is_empty(&self) -> bool {
        self.max_error_rate_pct.is_none()
            && self.min_tx_count.is_none()
            && self.max_cu_p99.is_none()
    }
}

/// One materialised breach. Rendered verbatim in the CLI's warn-
/// summary output; round-tripped as JSON in `--json` mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertBreach {
    /// Short machine-friendly identifier, e.g. `"error-rate"`.
    pub rule: &'static str,
    /// Human message with the actual and threshold values interpolated.
    pub message: String,
}

pub fn evaluate(report: &ObserveReport, config: &AlertConfig) -> Vec<AlertBreach> {
    let mut out = Vec::new();

    if let Some(min) = config.min_tx_count {
        if report.window.tx_count < min {
            out.push(AlertBreach {
                rule: "min-tx-count",
                message: format!(
                    "observed {} transactions; expected at least {} in the window",
                    report.window.tx_count, min
                ),
            });
        }
    }

    if let Some(threshold) = config.max_error_rate_pct {
        let threshold_frac = threshold / 100.0;
        for ix in &report.instructions {
            if let Some(filter) = &config.error_rate_ix {
                if filter != &ix.name {
                    continue;
                }
            }
            if let Some(rate) = ix.success_rate {
                let error_rate = 1.0 - rate;
                if error_rate > threshold_frac {
                    out.push(AlertBreach {
                        rule: "error-rate",
                        message: format!(
                            "ix `{}` error rate {:.2}% exceeds threshold {:.2}%",
                            ix.name,
                            error_rate * 100.0,
                            threshold
                        ),
                    });
                }
            }
        }
    }

    if let Some(threshold) = config.max_cu_p99 {
        for ix in &report.instructions {
            if let Some(filter) = &config.cu_p99_ix {
                if filter != &ix.name {
                    continue;
                }
            }
            if let Some(p99) = ix.cu_p99 {
                if p99 > threshold {
                    out.push(AlertBreach {
                        rule: "cu-p99",
                        message: format!(
                            "ix `{}` CU p99 {} exceeds threshold {}",
                            ix.name, p99, threshold
                        ),
                    });
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::IxMetrics;
    use crate::report::{ObserveReport, ObserveWindow};

    fn report_with(ix: Vec<IxMetrics>, tx_count: usize) -> ObserveReport {
        ObserveReport {
            program_id: "p".into(),
            program_name: None,
            window: ObserveWindow {
                seconds: 3600,
                tx_count,
                earliest_block_time: None,
                latest_block_time: None,
            },
            instructions: ix,
            errors: vec![],
            recent_failures: vec![],
            upgrade_history: None,
            account_counts: vec![],
        }
    }

    fn ix(name: &str, rate: f64, p99: Option<u64>) -> IxMetrics {
        IxMetrics {
            name: name.into(),
            count: 100,
            success_count: (rate * 100.0) as u64,
            error_count: 100 - (rate * 100.0) as u64,
            success_rate: Some(rate),
            cu_p50: None,
            cu_p95: None,
            cu_p99: p99,
        }
    }

    #[test]
    fn empty_config_fires_nothing() {
        let report = report_with(vec![ix("deposit", 1.0, Some(10_000))], 100);
        assert!(evaluate(&report, &AlertConfig::default()).is_empty());
    }

    #[test]
    fn error_rate_breach_names_the_offending_ix() {
        let report = report_with(vec![ix("deposit", 0.90, None)], 100);
        let breaches = evaluate(
            &report,
            &AlertConfig {
                max_error_rate_pct: Some(5.0),
                ..Default::default()
            },
        );
        assert_eq!(breaches.len(), 1);
        assert!(breaches[0].message.contains("deposit"));
        assert!(breaches[0].message.contains("10"));
    }

    #[test]
    fn error_rate_filter_scopes_to_one_ix() {
        let report = report_with(
            vec![ix("deposit", 0.90, None), ix("withdraw", 1.0, None)],
            200,
        );
        let breaches = evaluate(
            &report,
            &AlertConfig {
                max_error_rate_pct: Some(5.0),
                error_rate_ix: Some("withdraw".into()),
                ..Default::default()
            },
        );
        assert!(breaches.is_empty());
    }

    #[test]
    fn min_tx_count_fires_on_outage() {
        let report = report_with(vec![], 3);
        let breaches = evaluate(
            &report,
            &AlertConfig {
                min_tx_count: Some(100),
                ..Default::default()
            },
        );
        assert_eq!(breaches.len(), 1);
        assert_eq!(breaches[0].rule, "min-tx-count");
    }

    #[test]
    fn cu_p99_fires_on_regression() {
        let report = report_with(vec![ix("deposit", 1.0, Some(75_000))], 100);
        let breaches = evaluate(
            &report,
            &AlertConfig {
                max_cu_p99: Some(50_000),
                ..Default::default()
            },
        );
        assert_eq!(breaches.len(), 1);
        assert_eq!(breaches[0].rule, "cu-p99");
    }
}

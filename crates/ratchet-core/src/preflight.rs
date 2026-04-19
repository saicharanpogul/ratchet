//! Single-surface readiness check ("preflight") — runs on one
//! [`ProgramSurface`] and reports design patterns that make future
//! upgrades harder or deployments riskier.
//!
//! Distinct from the main diff engine in [`crate::engine::check`],
//! which compares two surfaces. A developer preparing their first
//! mainnet deploy should run preflight *before* shipping to catch
//! missing version fields, absent reserved padding, name collisions,
//! and other design choices that hurt later.
//!
//! Rule IDs in this module use the `PNNN` prefix (P for preflight)
//! so they never collide with the `RNNN` diff rules.

use crate::diagnostics::{Finding, Report, Severity};
use crate::rule::CheckContext;
use crate::surface::ProgramSurface;

/// A rule that inspects a single program surface — the readiness
/// counterpart to [`crate::rule::Rule`].
pub trait PreflightRule: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn check(&self, surface: &ProgramSurface, ctx: &CheckContext) -> Vec<Finding>;

    /// Convenience mirror of [`crate::rule::Rule::finding`] so a rule
    /// impl can write `self.finding(Severity::Unsafe).message(...)`.
    fn finding(&self, severity: Severity) -> Finding {
        Finding::new(severity, self.id(), self.name())
    }
}

/// Run every preflight rule against `surface` and aggregate findings.
/// Allow-flag demotion mirrors [`crate::engine::check`]: if a finding
/// carries an `allow_flag` enabled in `ctx`, its severity is demoted
/// to [`Severity::Additive`] so the report stays visible without
/// failing CI.
pub fn preflight(
    surface: &ProgramSurface,
    ctx: &CheckContext,
    rules: &[Box<dyn PreflightRule>],
) -> Report {
    let mut report = Report::new();
    for rule in rules {
        for mut finding in rule.check(surface, ctx) {
            if let Some(flag) = finding.allow_flag.as_deref() {
                if ctx.is_allowed(flag) {
                    finding.severity = Severity::Additive;
                }
            }
            report.push(finding);
        }
    }
    report
}

/// All preflight rules that ship with ratchet. Fresh allocation per
/// call to avoid any risk of shared state between runs.
pub fn default_preflight_rules() -> Vec<Box<dyn PreflightRule>> {
    crate::rules::preflight::all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::ProgramSurface;

    struct AlwaysUnsafe;
    impl PreflightRule for AlwaysUnsafe {
        fn id(&self) -> &'static str {
            "P000"
        }
        fn name(&self) -> &'static str {
            "always-unsafe"
        }
        fn description(&self) -> &'static str {
            "test rule"
        }
        fn check(&self, _: &ProgramSurface, _: &CheckContext) -> Vec<Finding> {
            vec![self
                .finding(Severity::Unsafe)
                .message("always unsafe")
                .allow_flag("allow-always-unsafe")]
        }
    }

    #[test]
    fn preflight_aggregates_findings_and_demotes_allowed_flags() {
        let rules: Vec<Box<dyn PreflightRule>> = vec![Box::new(AlwaysUnsafe)];
        let surface = ProgramSurface::default();

        let report = preflight(&surface, &CheckContext::new(), &rules);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, Severity::Unsafe);
        assert_eq!(report.exit_code(), 2);

        let ctx = CheckContext::new().with_allow("allow-always-unsafe");
        let report = preflight(&surface, &ctx, &rules);
        assert_eq!(report.findings[0].severity, Severity::Additive);
        assert_eq!(report.exit_code(), 0);
    }
}

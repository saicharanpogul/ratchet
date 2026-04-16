//! Top-level dispatch: run a set of rules over a pair of program surfaces
//! and collect their findings into a [`Report`].

use crate::diagnostics::{Report, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

/// Run every rule in `rules` against `(old, new)` and collect findings.
///
/// Findings whose [`allow_flag`](crate::Finding::allow_flag) is enabled in
/// `ctx.allowed_unsafes` are demoted to [`Severity::Additive`] so they are
/// still reported but no longer fail the run.
pub fn check(
    old: &ProgramSurface,
    new: &ProgramSurface,
    ctx: &CheckContext,
    rules: &[Box<dyn Rule>],
) -> Report {
    let mut report = Report::new();
    for rule in rules {
        for mut finding in rule.check(old, new, ctx) {
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

/// Built-in rules. The vector is intentionally a fresh allocation per call
/// so rule state cannot leak between runs.
pub fn default_rules() -> Vec<Box<dyn Rule>> {
    crate::rules::all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Finding;

    struct BreakingRule;
    impl Rule for BreakingRule {
        fn id(&self) -> &'static str {
            "R000"
        }
        fn name(&self) -> &'static str {
            "always-breaking"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn check(
            &self,
            _: &ProgramSurface,
            _: &ProgramSurface,
            _: &CheckContext,
        ) -> Vec<Finding> {
            vec![self
                .finding(Severity::Breaking)
                .message("boom")
                .allow_flag("unsafe-allow-boom")]
        }
    }

    #[test]
    fn engine_aggregates_rule_findings() {
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(BreakingRule)];
        let report = check(
            &ProgramSurface::default(),
            &ProgramSurface::default(),
            &CheckContext::new(),
            &rules,
        );
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn engine_demotes_findings_with_allowed_flag() {
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(BreakingRule)];
        let ctx = CheckContext::new().with_allow("unsafe-allow-boom");
        let report = check(
            &ProgramSurface::default(),
            &ProgramSurface::default(),
            &ctx,
            &rules,
        );
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, Severity::Additive);
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn default_rules_contains_r001() {
        let rules = default_rules();
        assert!(rules.iter().any(|r| r.id() == "R001"));
    }
}

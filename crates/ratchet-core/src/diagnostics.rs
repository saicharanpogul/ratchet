//! Findings, severity, and the aggregate report emitted by the rule engine.

use serde::{Deserialize, Serialize};

/// Classification of a single diff between two program versions.
///
/// Ordering is meaningful: `Additive < Unsafe < Breaking`. Aggregating a set
/// of findings by taking the max severity yields the overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Backward-compatible. Existing accounts and existing clients keep
    /// working after the upgrade lands.
    Additive,
    /// Would be breaking, but the caller can acknowledge it via an
    /// `--unsafe-*` flag or a declared `Migration<From, To>` and proceed.
    Unsafe,
    /// Will corrupt existing on-chain state, break existing clients, or
    /// orphan existing PDAs. Cannot be made safe without a migration
    /// instruction.
    Breaking,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Additive => "additive",
            Severity::Unsafe => "unsafe",
            Severity::Breaking => "breaking",
        }
    }
}

/// Structured location of a finding inside a program surface.
///
/// Each segment is conventionally a `kind:name` pair, e.g. `"account:Vault"`,
/// `"field:amount"`, `"ix:deposit"`, `"arg:mint"`. Keeping the segments as
/// strings lets findings cross the rule / renderer boundary without a
/// surface-level dependency.
pub type Path = Vec<String>;

/// A single diagnostic emitted by a rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Stable rule identifier, e.g. `"R006"`.
    pub rule_id: String,
    /// Kebab-case rule name, e.g. `"account-discriminator-change"`.
    pub rule_name: String,
    pub severity: Severity,
    pub path: Path,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// If set, passing this flag demotes the finding to `Additive`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_flag: Option<String>,
}

/// Aggregate result of diffing two program versions.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    pub fn extend<I: IntoIterator<Item = Finding>>(&mut self, iter: I) {
        self.findings.extend(iter);
    }

    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn max_severity(&self) -> Option<Severity> {
        self.findings.iter().map(|f| f.severity).max()
    }

    /// Process exit code implied by this report.
    ///
    /// - `0` — no findings, or only additive findings
    /// - `1` — at least one breaking finding
    /// - `2` — at least one unsafe finding (and no breaking findings)
    pub fn exit_code(&self) -> i32 {
        match self.max_severity() {
            None | Some(Severity::Additive) => 0,
            Some(Severity::Unsafe) => 2,
            Some(Severity::Breaking) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(severity: Severity) -> Finding {
        Finding {
            rule_id: "R000".into(),
            rule_name: "test".into(),
            severity,
            path: vec!["account:Foo".into()],
            message: "test".into(),
            old: None,
            new: None,
            suggestion: None,
            allow_flag: None,
        }
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Additive < Severity::Unsafe);
        assert!(Severity::Unsafe < Severity::Breaking);
    }

    #[test]
    fn empty_report_exits_zero() {
        assert_eq!(Report::new().exit_code(), 0);
    }

    #[test]
    fn additive_only_report_exits_zero() {
        let mut r = Report::new();
        r.push(finding(Severity::Additive));
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn unsafe_report_exits_two() {
        let mut r = Report::new();
        r.push(finding(Severity::Additive));
        r.push(finding(Severity::Unsafe));
        assert_eq!(r.exit_code(), 2);
        assert_eq!(r.max_severity(), Some(Severity::Unsafe));
    }

    #[test]
    fn breaking_wins_over_unsafe() {
        let mut r = Report::new();
        r.push(finding(Severity::Unsafe));
        r.push(finding(Severity::Breaking));
        assert_eq!(r.exit_code(), 1);
        assert_eq!(r.max_severity(), Some(Severity::Breaking));
    }
}

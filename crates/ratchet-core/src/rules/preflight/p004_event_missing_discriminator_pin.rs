//! P004 — event-missing-discriminator-pin.
//!
//! Parallel to P003, for `#[event]` logs. A rename re-hashes the
//! default `sha256("event:<Name>")[..8]`, which desyncs every
//! off-chain indexer filtering on the old bytes (R016 catches it on
//! upgrade).

use crate::diagnostics::{Finding, Severity};
use crate::preflight::PreflightRule;
use crate::rule::CheckContext;
use crate::surface::ProgramSurface;

pub const ID: &str = "P004";
pub const NAME: &str = "event-missing-discriminator-pin";
pub const DESCRIPTION: &str =
    "Event uses Anchor's default discriminator; pinning it survives renames without desyncing indexers.";

pub struct EventMissingDiscriminatorPin;

impl PreflightRule for EventMissingDiscriminatorPin {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn description(&self) -> &'static str {
        DESCRIPTION
    }

    fn check(&self, surface: &ProgramSurface, _ctx: &CheckContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        for (name, event) in &surface.events {
            let default = default_event_discriminator(name);
            if event.discriminator != default {
                continue;
            }
            findings.push(
                self.finding(Severity::Additive)
                    .at([format!("event:{name}"), "discriminator".into()])
                    .message(format!(
                        "event `{name}` uses the default discriminator; a future rename would silently desync every indexer filtering on the old 8-byte selector",
                    ))
                    .suggestion(
                        "Pin the event discriminator explicitly at declaration time. Off-chain consumers rely on it being stable.",
                    ),
            );
        }
        findings
    }
}

fn default_event_discriminator(name: &str) -> [u8; 8] {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("event:{name}").as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{EventDef, ProgramSurface};

    fn surface_with(name: &str, disc: [u8; 8]) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.events.insert(
            name.into(),
            EventDef {
                name: name.into(),
                discriminator: disc,
            },
        );
        s
    }

    #[test]
    fn default_event_disc_is_flagged() {
        let s = surface_with("Deposited", default_event_discriminator("Deposited"));
        let findings = EventMissingDiscriminatorPin.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
    }

    #[test]
    fn pinned_event_disc_is_not_flagged() {
        let s = surface_with("Deposited", [0xca, 0xfe, 0, 0, 0, 0, 0, 0]);
        assert!(EventMissingDiscriminatorPin
            .check(&s, &CheckContext::new())
            .is_empty());
    }
}

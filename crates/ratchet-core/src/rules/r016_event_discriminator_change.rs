//! R016 — event-discriminator-change.
//!
//! Anchor events are CPI log entries prefixed by an 8-byte discriminator
//! (default: `sha256("event:<Name>")[..8]`). Off-chain consumers —
//! indexers, analytics pipelines, bots watching the program's logs —
//! filter by that exact byte sequence. If it changes, every listener
//! goes dark on the next emitted event; the events keep landing
//! on-chain but no subscriber picks them up.
//!
//! Parallel to R006 (account) and R014 (instruction) for events:
//! `Breaking` with `allow-event-rename` escape hatch.

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R016";
pub const NAME: &str = "event-discriminator-change";
pub const DESCRIPTION: &str =
    "An event's discriminator changed; every off-chain listener filtering for the old value goes silent.";

pub struct EventDiscriminatorChange;

impl Rule for EventDiscriminatorChange {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn description(&self) -> &'static str {
        DESCRIPTION
    }

    fn check(
        &self,
        old: &ProgramSurface,
        new: &ProgramSurface,
        _ctx: &CheckContext,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();
        for (name, old_event) in &old.events {
            let Some(new_event) = new.events.get(name) else {
                continue;
            };
            if old_event.discriminator == new_event.discriminator {
                continue;
            }
            findings.push(
                self.finding(Severity::Breaking)
                    .at([format!("event:{name}"), "discriminator".into()])
                    .message(format!(
                        "discriminator of event `{name}` changed: {} → {}",
                        hex(&old_event.discriminator),
                        hex(&new_event.discriminator)
                    ))
                    .old(hex(&old_event.discriminator))
                    .new_value(hex(&new_event.discriminator))
                    .allow_flag("allow-event-rename")
                    .suggestion(
                        "If the event was renamed, restore the original name. If the rename is \
                         deliberate, pin the original discriminator explicitly so downstream \
                         indexers and listeners stay connected.",
                    ),
            );
        }
        findings
    }
}

fn hex(disc: &[u8; 8]) -> String {
    let mut out = String::with_capacity(18);
    out.push_str("0x");
    for b in disc {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{Discriminator, EventDef};

    fn ev(name: &str, disc: Discriminator) -> EventDef {
        EventDef {
            name: name.into(),
            discriminator: disc,
        }
    }

    fn surface_with<I: IntoIterator<Item = EventDef>>(events: I) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        for e in events {
            s.events.insert(e.name.clone(), e);
        }
        s
    }

    #[test]
    fn identical_events_no_finding() {
        let s = surface_with([ev("Deposited", [1; 8])]);
        assert!(EventDiscriminatorChange
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn discriminator_change_is_breaking_with_allow() {
        let old = surface_with([ev("Deposited", [1, 2, 3, 4, 5, 6, 7, 8])]);
        let new = surface_with([ev("Deposited", [9, 10, 11, 12, 13, 14, 15, 16])]);
        let findings = EventDiscriminatorChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.path, vec!["event:Deposited", "discriminator"]);
        assert_eq!(f.allow_flag.as_deref(), Some("allow-event-rename"));
        assert_eq!(f.old.as_deref(), Some("0x0102030405060708"));
    }

    #[test]
    fn removed_event_not_in_scope() {
        // Event-removal is handled elsewhere (or silently — no off-chain
        // listener is broken any worse than a removed ix).
        let old = surface_with([ev("Deposited", [1; 8])]);
        let new = surface_with([] as [EventDef; 0]);
        assert!(EventDiscriminatorChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn new_events_not_in_scope() {
        let old = surface_with([] as [EventDef; 0]);
        let new = surface_with([ev("Deposited", [1; 8])]);
        assert!(EventDiscriminatorChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}

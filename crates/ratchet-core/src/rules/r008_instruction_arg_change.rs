//! R008 — instruction-arg-change.
//!
//! An instruction's argument signature changed (args reordered, retyped,
//! added, or removed). Instruction data is Borsh-serialized in declaration
//! order, so any change corrupts deserialization for every unchanged
//! client.
//!
//! Breaking, with `allow-ix-arg-change` for the case where the author
//! has flag-dayed every caller in the same release.

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::{ArgDef, ProgramSurface};

pub const ID: &str = "R008";
pub const NAME: &str = "instruction-arg-change";
pub const DESCRIPTION: &str =
    "An instruction's argument signature changed; existing clients send bytes the program will misread.";

pub struct InstructionArgChange;

impl Rule for InstructionArgChange {
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
        for (name, old_ix) in &old.instructions {
            let Some(new_ix) = new.instructions.get(name) else {
                continue;
            };
            if args_equal(&old_ix.args, &new_ix.args) {
                continue;
            }
            findings.push(
                self.finding(Severity::Breaking)
                    .at([format!("ix:{name}"), "args".into()])
                    .message(format!(
                        "argument signature of `{name}` changed: ({}) → ({})",
                        render_args(&old_ix.args),
                        render_args(&new_ix.args)
                    ))
                    .old(render_args(&old_ix.args))
                    .new_value(render_args(&new_ix.args))
                    .allow_flag("allow-ix-arg-change")
                    .suggestion(
                        "Instruction data is Borsh-serialized in declaration order. Prefer a \
                         new instruction name rather than reshaping an existing one, or update \
                         every caller in the same release.",
                    ),
            );
        }
        findings
    }
}

fn args_equal(a: &[ArgDef], b: &[ArgDef]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.name == y.name && x.ty == y.ty)
}

fn render_args(args: &[ArgDef]) -> String {
    args.iter()
        .map(|a| format!("{}: {}", a.name, a.ty))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{InstructionDef, PrimitiveType, TypeRef};

    fn arg(name: &str, ty: PrimitiveType) -> ArgDef {
        ArgDef {
            name: name.into(),
            ty: TypeRef::primitive(ty),
        }
    }

    fn ix(name: &str, args: Vec<ArgDef>) -> InstructionDef {
        InstructionDef {
            name: name.into(),
            discriminator: [0; 8],
            args,
            accounts: vec![],
        }
    }

    fn surface(i: InstructionDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.instructions.insert(i.name.clone(), i);
        s
    }

    #[test]
    fn identical_args_no_finding() {
        let s = surface(ix("deposit", vec![arg("amount", PrimitiveType::U64)]));
        assert!(InstructionArgChange
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn arg_reorder_is_breaking() {
        let old = surface(ix(
            "deposit",
            vec![
                arg("amount", PrimitiveType::U64),
                arg("memo", PrimitiveType::String),
            ],
        ));
        let new = surface(ix(
            "deposit",
            vec![
                arg("memo", PrimitiveType::String),
                arg("amount", PrimitiveType::U64),
            ],
        ));
        let findings = InstructionArgChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, ID);
        assert_eq!(findings[0].severity, Severity::Breaking);
        assert_eq!(
            findings[0].allow_flag.as_deref(),
            Some("allow-ix-arg-change")
        );
        assert_eq!(
            findings[0].old.as_deref(),
            Some("amount: u64, memo: string")
        );
    }

    #[test]
    fn arg_retype_is_breaking() {
        let old = surface(ix("deposit", vec![arg("amount", PrimitiveType::U32)]));
        let new = surface(ix("deposit", vec![arg("amount", PrimitiveType::U64)]));
        let findings = InstructionArgChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn arg_append_is_breaking() {
        let old = surface(ix("deposit", vec![arg("amount", PrimitiveType::U64)]));
        let new = surface(ix(
            "deposit",
            vec![
                arg("amount", PrimitiveType::U64),
                arg("memo", PrimitiveType::String),
            ],
        ));
        let findings = InstructionArgChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn new_instructions_are_out_of_scope() {
        let old = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        let new = surface(ix("deposit", vec![arg("amount", PrimitiveType::U64)]));
        assert!(InstructionArgChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}

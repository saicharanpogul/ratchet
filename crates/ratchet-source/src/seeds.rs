//! Convert Rust seed expressions (`b"vault"`, `user.key().as_ref()`,
//! `mint.key().to_bytes()`, …) into [`Seed`] values.
//!
//! The parser is a deliberate best-effort: Anchor users write seeds in a
//! wide variety of styles and we'd rather surface a structured
//! [`Seed::Unknown`] than crash on an expression we can't flatten. The
//! downstream R013 rule treats `Unknown` seeds as opaque but still
//! comparable by source text, so a seed-expression change is still
//! caught even when we can't name its components.

use proc_macro2::Span;
use quote::ToTokens;
use ratchet_core::Seed;
use syn::{Expr, ExprLit, Lit};

/// The high-level categories a seed component can fall into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedExpr {
    Const(Vec<u8>),
    Account { name: String, field: Option<String> },
    Arg(String),
    Unknown(String),
}

impl SeedExpr {
    pub fn into_seed(self) -> Seed {
        match self {
            SeedExpr::Const(bytes) => Seed::Const { bytes },
            SeedExpr::Account { name, field } => Seed::Account { name, field },
            SeedExpr::Arg(name) => Seed::Arg { name },
            SeedExpr::Unknown(raw) => Seed::Unknown { raw },
        }
    }
}

/// Interpret a single expression inside a `seeds = [...]` literal.
///
/// `known_accounts` is the set of account names from the surrounding
/// `#[derive(Accounts)]` struct. Any expression whose root identifier
/// matches one of those names is treated as an `Account`; otherwise it
/// falls to `Arg` (instruction argument) or `Unknown`.
pub fn parse_seed_expr(expr: &Expr, known_accounts: &[String]) -> SeedExpr {
    // Peel `&` borrows — they are common in `&foo.key()`.
    let expr = strip_borrow(expr);

    // Byte-string literal: `b"vault"`.
    if let Expr::Lit(ExprLit {
        lit: Lit::ByteStr(bs),
        ..
    }) = expr
    {
        return SeedExpr::Const(bs.value());
    }

    // `b"vault".as_ref()` / `b"vault".as_bytes()` / `x.to_le_bytes()` — walk
    // through method-call chains and try to find the base identifier.
    if let Some((root, field)) = walk_method_chain(expr) {
        if known_accounts.iter().any(|a| a == &root) {
            return SeedExpr::Account {
                name: root,
                field,
            };
        } else if field.is_none() {
            return SeedExpr::Arg(root);
        }
    }

    // Fall back to the raw token text so the finding still shows something
    // sensible and diffs stay stable.
    SeedExpr::Unknown(token_text(expr))
}

fn strip_borrow(mut expr: &Expr) -> &Expr {
    while let Expr::Reference(r) = expr {
        expr = &r.expr;
    }
    expr
}

/// Walk through `a.b.c.method(...)` style chains, returning
/// `(root_ident, deepest_field)`. Method calls (`.key()`, `.to_bytes()`)
/// are ignored so we see past Anchor's encoding boilerplate.
fn walk_method_chain(expr: &Expr) -> Option<(String, Option<String>)> {
    let mut path: Vec<String> = Vec::new();
    let mut cursor = expr;
    loop {
        let cursor_stripped = strip_borrow(cursor);
        match cursor_stripped {
            Expr::Path(p) => {
                let ident = p.path.segments.last()?.ident.to_string();
                path.push(ident);
                break;
            }
            Expr::Field(f) => {
                if let syn::Member::Named(ident) = &f.member {
                    path.push(ident.to_string());
                }
                cursor = &f.base;
            }
            Expr::MethodCall(m) => {
                // Skip the method; descend into the receiver.
                cursor = &m.receiver;
            }
            _ => return None,
        }
    }
    path.reverse();
    if path.is_empty() {
        return None;
    }
    let root = path.remove(0);
    let field = if path.is_empty() {
        None
    } else {
        Some(path.join("."))
    };
    Some((root, field))
}

fn token_text(expr: &Expr) -> String {
    // Use quote!'s token stream; normalizes spacing.
    let tokens = expr.to_token_stream();
    tokens.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn accounts() -> Vec<String> {
        vec!["user".into(), "vault".into(), "mint".into()]
    }

    #[test]
    fn byte_string_literal_becomes_const() {
        let e: Expr = parse_quote!(b"vault");
        assert_eq!(parse_seed_expr(&e, &accounts()), SeedExpr::Const(b"vault".to_vec()));
    }

    #[test]
    fn borrowed_byte_string_still_const() {
        let e: Expr = parse_quote!(&b"vault");
        assert_eq!(parse_seed_expr(&e, &accounts()), SeedExpr::Const(b"vault".to_vec()));
    }

    #[test]
    fn account_key_call() {
        let e: Expr = parse_quote!(user.key());
        assert_eq!(
            parse_seed_expr(&e, &accounts()),
            SeedExpr::Account {
                name: "user".into(),
                field: None,
            }
        );
    }

    #[test]
    fn account_key_as_ref_chain() {
        let e: Expr = parse_quote!(user.key().as_ref());
        assert_eq!(
            parse_seed_expr(&e, &accounts()),
            SeedExpr::Account {
                name: "user".into(),
                field: None,
            }
        );
    }

    #[test]
    fn account_field_extracted() {
        let e: Expr = parse_quote!(vault.config.admin.to_bytes());
        assert_eq!(
            parse_seed_expr(&e, &accounts()),
            SeedExpr::Account {
                name: "vault".into(),
                field: Some("config.admin".into()),
            }
        );
    }

    #[test]
    fn arg_without_field_becomes_arg() {
        let e: Expr = parse_quote!(amount.to_le_bytes());
        assert_eq!(parse_seed_expr(&e, &accounts()), SeedExpr::Arg("amount".into()));
    }

    #[test]
    fn unknown_expressions_round_trip_as_raw() {
        let e: Expr = parse_quote!(Clock::get()?.unix_timestamp.to_le_bytes());
        match parse_seed_expr(&e, &accounts()) {
            SeedExpr::Unknown(raw) => assert!(raw.contains("Clock")),
            other => panic!("expected unknown, got {other:?}"),
        }
    }

    #[test]
    fn into_seed_preserves_classification() {
        let s = SeedExpr::Const(b"x".to_vec()).into_seed();
        assert!(matches!(s, Seed::Const { .. }));
        let s = SeedExpr::Account {
            name: "a".into(),
            field: None,
        }
        .into_seed();
        assert!(matches!(s, Seed::Account { .. }));
    }

    #[allow(dead_code)]
    fn _ensure_span_accessible() {
        // proc_macro2::Span is used via to_token_stream; verify the import
        // chain stays linked.
        let _: Span = Span::call_site();
    }
}

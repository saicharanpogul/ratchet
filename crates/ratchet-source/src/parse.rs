//! Parse Anchor program source files with `syn` and extract PDA seed
//! information into a [`SourcePatch`].
//!
//! Algorithm:
//!
//! 1. Walk every `.rs` file under the supplied root.
//! 2. Build two tables per file:
//!    - `ix_name_of[StructName] = ix_name` — populated from
//!      `#[program] mod { pub fn <ix>(ctx: Context<StructName>, ...) }`.
//!    - `accounts_of[StructName] = [(field_name, seeds_expr)]` —
//!      populated from `#[derive(Accounts)] struct StructName { ... }`.
//! 3. Join the two: for each Accounts struct, emit `SourcePatch` entries
//!    keyed by `(ix_name, field_name)`.
//!
//! Unresolved structs (no matching `Context<X>` in a `#[program]` mod)
//! are skipped silently — they are often helper composites that belong
//! to the IDL's flattened account list already.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ratchet_core::PdaSpec;
use syn::visit::Visit;
use syn::{
    Attribute, Expr, ExprLit, ExprPath, ImplItem, ItemFn, ItemImpl, ItemMod, ItemStruct,
    Lit, Meta, Pat,
};
use walkdir::WalkDir;

use crate::patch::SourcePatch;
use crate::seeds::parse_seed_expr;

/// Diagnostic breadcrumb collected while scanning a directory.
#[derive(Debug, Default, Clone)]
pub struct SourceScan {
    pub files_parsed: usize,
    pub structs_scanned: usize,
    pub pdas_extracted: usize,
    pub unresolved_structs: Vec<String>,
    pub patch: SourcePatch,
}

/// Parse a single file and append everything found to `scan`.
pub fn parse_file(path: &Path, scan: &mut SourceScan) -> Result<()> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading source file {}", path.display()))?;
    let file = syn::parse_file(&content)
        .with_context(|| format!("parsing {}", path.display()))?;

    let mut v = FileVisitor::default();
    v.visit_file(&file);

    for (struct_name, fields) in v.accounts_of {
        scan.structs_scanned += 1;
        let Some(ix_name) = v.ix_name_of.get(&struct_name) else {
            scan.unresolved_structs.push(struct_name);
            continue;
        };
        let known_accounts: Vec<String> = fields.iter().map(|f| f.0.clone()).collect();
        for (field, seeds_expr) in &fields {
            let Some(seeds_expr) = seeds_expr else {
                continue;
            };
            let seeds = seeds_expr
                .iter()
                .map(|e| parse_seed_expr(e, &known_accounts).into_seed())
                .collect();
            let pda = PdaSpec {
                seeds,
                program_id: None,
            };
            scan.patch.insert(ix_name, field, pda);
            scan.pdas_extracted += 1;
        }
    }
    scan.files_parsed += 1;
    Ok(())
}

/// Walk `root` recursively for `.rs` files and parse each. Files inside
/// `target/` and other build artifact directories are skipped.
pub fn parse_dir(root: &Path) -> Result<SourceScan> {
    let mut scan = SourceScan::default();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !is_build_artifact(e.path()))
    {
        let entry = entry.with_context(|| format!("walking {}", root.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        parse_file(path, &mut scan)
            .with_context(|| format!("scanning {}", path.display()))?;
    }
    Ok(scan)
}

fn is_build_artifact(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("target") | Some(".git") | Some("node_modules")
        )
    })
}

#[derive(Default)]
struct FileVisitor {
    /// `ix_name_of[AccountsStructName] = instruction_name`
    ix_name_of: HashMap<String, String>,
    /// `accounts_of[AccountsStructName] = [(field_name, seeds_expr)]`
    accounts_of: HashMap<String, Vec<(String, Option<Vec<Expr>>)>>,
    inside_program_mod: bool,
}

impl<'ast> Visit<'ast> for FileVisitor {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        let was_in_program = self.inside_program_mod;
        if has_program_attr(&node.attrs) {
            self.inside_program_mod = true;
        }
        syn::visit::visit_item_mod(self, node);
        self.inside_program_mod = was_in_program;
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        // Inside a program mod, impl blocks contain the instruction fns in
        // older Anchor styles; visit to pick up its inner fn signatures.
        if self.inside_program_mod {
            for item in &node.items {
                if let ImplItem::Fn(m) = item {
                    self.capture_fn_sig(&m.sig);
                }
            }
        }
        syn::visit::visit_item_impl(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if self.inside_program_mod {
            self.capture_fn_sig(&node.sig);
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        if !has_derive_accounts(&node.attrs) {
            return;
        }
        let struct_name = node.ident.to_string();
        let mut fields = Vec::new();
        if let syn::Fields::Named(named) = &node.fields {
            for field in &named.named {
                let Some(ident) = &field.ident else { continue };
                let seeds_expr = extract_seeds_from_attrs(&field.attrs);
                fields.push((ident.to_string(), seeds_expr));
            }
        }
        self.accounts_of.insert(struct_name, fields);
    }
}

impl FileVisitor {
    fn capture_fn_sig(&mut self, sig: &syn::Signature) {
        let ix_name = sig.ident.to_string();
        // First argument is `ctx: Context<X>` (or &mut variant).
        let Some(first) = sig.inputs.first() else {
            return;
        };
        let syn::FnArg::Typed(pat_type) = first else {
            return;
        };
        let Pat::Ident(_) = &*pat_type.pat else {
            return;
        };
        let Some(accounts_struct) = extract_context_type(&pat_type.ty) else {
            return;
        };
        self.ix_name_of.insert(accounts_struct, ix_name);
    }
}

fn has_program_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| path_matches(&a.meta, "program"))
}

fn has_derive_accounts(attrs: &[Attribute]) -> bool {
    for attr in attrs {
        if !path_is(attr.path(), "derive") {
            continue;
        }
        if let Meta::List(list) = &attr.meta {
            let tokens = list.tokens.to_string();
            if tokens.contains("Accounts") {
                return true;
            }
        }
    }
    false
}

fn path_matches(meta: &Meta, name: &str) -> bool {
    let path = match meta {
        Meta::Path(p) => p,
        Meta::List(l) => &l.path,
        Meta::NameValue(nv) => &nv.path,
    };
    path_is(path, name)
}

fn path_is(path: &syn::Path, name: &str) -> bool {
    path.segments.last().is_some_and(|s| s.ident == name)
}

/// Given `Context<X>` / `Context<'info, X>` / `&mut Context<X>`, return
/// `Some("X".into())`.
fn extract_context_type(ty: &syn::Type) -> Option<String> {
    let ty = strip_ty_wrappers(ty);
    let syn::Type::Path(tp) = ty else {
        return None;
    };
    let last = tp.path.segments.last()?;
    if last.ident != "Context" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(ab) = &last.arguments else {
        return None;
    };
    for arg in &ab.args {
        if let syn::GenericArgument::Type(syn::Type::Path(inner)) = arg {
            let ident = inner.path.segments.last()?.ident.to_string();
            return Some(ident);
        }
    }
    None
}

fn strip_ty_wrappers(ty: &syn::Type) -> &syn::Type {
    match ty {
        syn::Type::Reference(r) => strip_ty_wrappers(&r.elem),
        syn::Type::Paren(p) => strip_ty_wrappers(&p.elem),
        _ => ty,
    }
}

/// Walk a field's attribute list and return the seed expressions inside a
/// `#[account(seeds = [...])]` attribute, if present.
fn extract_seeds_from_attrs(attrs: &[Attribute]) -> Option<Vec<Expr>> {
    for attr in attrs {
        if !path_is(attr.path(), "account") {
            continue;
        }
        let Meta::List(list) = &attr.meta else {
            continue;
        };
        let parsed = list
            .parse_args_with(
                syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated,
            )
            .ok()?;
        for item in parsed {
            if let Some(seeds) = expr_as_seeds_assign(&item) {
                return Some(seeds);
            }
        }
    }
    None
}

/// Match `seeds = [a, b, c]` (a `syn::ExprAssign` with an ident LHS and an
/// array RHS).
fn expr_as_seeds_assign(expr: &Expr) -> Option<Vec<Expr>> {
    let Expr::Assign(assign) = expr else {
        return None;
    };
    let ident = match &*assign.left {
        Expr::Path(ExprPath { path, .. }) => path.segments.last()?.ident.to_string(),
        _ => return None,
    };
    if ident != "seeds" {
        return None;
    }
    // RHS may be wrapped in an `&` or a paren.
    let mut rhs = &*assign.right;
    loop {
        match rhs {
            Expr::Reference(r) => rhs = &r.expr,
            Expr::Paren(p) => rhs = &p.expr,
            _ => break,
        }
    }
    let Expr::Array(arr) = rhs else {
        // Sometimes the RHS is a literal single value (unusual for seeds);
        // treat it as a one-element array.
        if let Expr::Lit(ExprLit {
            lit: Lit::ByteStr(_),
            ..
        }) = rhs
        {
            return Some(vec![rhs.clone()]);
        }
        return None;
    };
    Some(arr.elems.iter().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM_SAMPLE: &str = r#"
        use anchor_lang::prelude::*;

        #[program]
        pub mod my_program {
            use super::*;

            pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
                Ok(())
            }

            pub fn withdraw(ctx: Context<Withdraw>) -> Result<()> {
                Ok(())
            }
        }

        #[derive(Accounts)]
        pub struct Deposit<'info> {
            #[account(mut)]
            pub user: Signer<'info>,

            #[account(
                init,
                payer = user,
                seeds = [b"vault", user.key().as_ref()],
                bump,
                space = 8 + 64,
            )]
            pub vault: Account<'info, Vault>,

            pub system_program: Program<'info, System>,
        }

        #[derive(Accounts)]
        pub struct Withdraw<'info> {
            pub user: Signer<'info>,
        }

        #[account]
        pub struct Vault {
            pub owner: Pubkey,
            pub balance: u64,
        }
    "#;

    #[test]
    fn parse_extracts_pda_seeds() {
        use std::io::Write;
        let mut path = std::env::temp_dir();
        path.push(format!(
            "ratchet-source-sample-{}-{}.rs",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(PROGRAM_SAMPLE.as_bytes()).unwrap();

        let mut scan = SourceScan::default();
        parse_file(&path, &mut scan).unwrap();

        assert_eq!(scan.files_parsed, 1);
        assert_eq!(scan.structs_scanned, 2);

        let pda = scan
            .patch
            .get("deposit", "vault")
            .expect("Deposit::vault should have seeds extracted");
        assert_eq!(pda.seeds.len(), 2);
        match &pda.seeds[0] {
            ratchet_core::Seed::Const { bytes } => assert_eq!(bytes, &b"vault".to_vec()),
            other => panic!("expected const seed, got {other:?}"),
        }
        match &pda.seeds[1] {
            ratchet_core::Seed::Account { name, .. } => assert_eq!(name, "user"),
            other => panic!("expected account seed, got {other:?}"),
        }

        // Withdraw has no PDA accounts → no entries for withdraw in the patch.
        assert!(scan.patch.get("withdraw", "user").is_none());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn struct_without_context_binding_is_unresolved() {
        let src = r#"
            #[derive(Accounts)]
            pub struct Orphan<'info> {
                #[account(seeds = [b"x"], bump)]
                pub pda: UncheckedAccount<'info>,
            }
        "#;
        let mut path = std::env::temp_dir();
        path.push(format!(
            "ratchet-source-orphan-{}-{}.rs",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, src).unwrap();
        let mut scan = SourceScan::default();
        parse_file(&path, &mut scan).unwrap();
        assert_eq!(scan.structs_scanned, 1);
        assert_eq!(scan.pdas_extracted, 0);
        assert!(scan.unresolved_structs.contains(&"Orphan".to_string()));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn parse_dir_skips_target_and_git() {
        let tmp = std::env::temp_dir().join(format!(
            "ratchet-source-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(tmp.join("src")).unwrap();
        fs::create_dir_all(tmp.join("target/debug")).unwrap();
        fs::write(tmp.join("src/lib.rs"), PROGRAM_SAMPLE).unwrap();
        fs::write(
            tmp.join("target/debug/garbage.rs"),
            "fn not_even_valid() {{ @@@",
        )
        .unwrap();

        let scan = parse_dir(&tmp).unwrap();
        assert_eq!(scan.files_parsed, 1);
        assert_eq!(scan.pdas_extracted, 1);

        let _ = fs::remove_dir_all(&tmp);
    }
}

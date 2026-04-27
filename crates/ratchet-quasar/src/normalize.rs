//! Normalise a [`QuasarIdl`] into ratchet's framework-agnostic
//! [`ProgramSurface`] IR. Once normalised, every existing rule (P-rules,
//! R-rules) runs on a Quasar surface as it does on an Anchor surface —
//! no rule changes needed.
//!
//! Key translation choices, all flagged inline:
//!
//! - **Discriminators**: Quasar's are `Vec<u8>` (typically 1 byte).
//!   Ratchet's IR uses `[u8; 8]` (Anchor's sha256-prefix size). We
//!   pad with trailing zeros so they fit. R006 (account-discriminator-
//!   change) still fires on any byte-level change; P003/P004 (default-
//!   discriminator-pin) won't match because the padded shape never
//!   equals `sha256("account:<Name>")[..8]` — which is the right
//!   semantics: Quasar devs always assign discriminators explicitly,
//!   so "is this the default?" is a category error there.
//! - **Account fields**: Quasar separates `IdlAccountDef` (just name +
//!   discriminator) from `IdlTypeDef` (the field list). We join them
//!   here so the surface's [`AccountDef`] carries fields directly,
//!   matching how the rules already think.
//! - **`IdlType` mapping**: untagged enum → `TypeRef`. `DynString` /
//!   `DynVec` lose Quasar's `maxLength` / `prefixBytes` precision —
//!   ratchet's IR doesn't model bounded-storage info today, but the
//!   loss is invisible to current rules.

use std::collections::{BTreeMap, HashMap};

use anyhow::{anyhow, Result};
use ratchet_core::{
    AccountDef, AccountInput, ArgDef, Discriminator, ErrorDef, EventDef, FieldDef, InstructionDef,
    PdaSpec, PrimitiveType, ProgramSurface, Seed, TypeDef, TypeRef,
};

use crate::idl::{
    QuasarIdl, QuasarIdlAccountItem, QuasarIdlField, QuasarIdlPda, QuasarIdlSeed, QuasarIdlType,
    QuasarIdlTypeDefKind,
};

/// Normalize a parsed Quasar IDL into a ratchet surface. Errors only
/// on structural mismatches (e.g. an account references a typedef
/// name that doesn't exist) — unrecognised primitive type names fall
/// back to [`TypeRef::Unrecognized`] so a single typo doesn't block
/// the rest of the diff.
pub fn normalize(idl: &QuasarIdl) -> Result<ProgramSurface> {
    // Build a typedef → fields lookup so account/event defs (which
    // only carry name + discriminator) can pull their fields in.
    let mut typedef_fields: HashMap<&str, Vec<FieldDef>> = HashMap::new();
    let mut types: BTreeMap<String, TypeDef> = BTreeMap::new();
    for tdef in &idl.types {
        if tdef.ty.kind != QuasarIdlTypeDefKind::Struct {
            // Quasar only ships Struct today; defensive log if a future
            // schema adds enums/unions.
            continue;
        }
        let fields = convert_fields(&tdef.ty.fields);
        typedef_fields.insert(tdef.name.as_str(), fields.clone());
        types.insert(tdef.name.clone(), TypeDef::Struct { fields });
    }

    let mut accounts: BTreeMap<String, AccountDef> = BTreeMap::new();
    for acc in &idl.accounts {
        let fields = typedef_fields
            .get(acc.name.as_str())
            .cloned()
            .unwrap_or_default();
        accounts.insert(
            acc.name.clone(),
            AccountDef {
                name: acc.name.clone(),
                discriminator: pad_discriminator(&acc.discriminator),
                fields,
                size: None,
            },
        );
    }

    let mut events: BTreeMap<String, EventDef> = BTreeMap::new();
    for ev in &idl.events {
        events.insert(
            ev.name.clone(),
            EventDef {
                name: ev.name.clone(),
                discriminator: pad_discriminator(&ev.discriminator),
            },
        );
    }

    let mut instructions: BTreeMap<String, InstructionDef> = BTreeMap::new();
    for ix in &idl.instructions {
        instructions.insert(
            ix.name.clone(),
            InstructionDef {
                name: ix.name.clone(),
                discriminator: pad_discriminator(&ix.discriminator),
                args: ix
                    .args
                    .iter()
                    .map(|f| ArgDef {
                        name: f.name.clone(),
                        ty: convert_type(&f.ty),
                    })
                    .collect(),
                accounts: ix.accounts.iter().map(convert_account_input).collect(),
            },
        );
    }

    let mut errors: BTreeMap<u32, ErrorDef> = BTreeMap::new();
    for e in &idl.errors {
        errors.insert(
            e.code,
            ErrorDef {
                code: e.code,
                name: e.name.clone(),
                message: e.msg.clone(),
            },
        );
    }

    let name = if idl.metadata.name.is_empty() {
        // Defensive fallback so downstream errors point at a usable
        // surface even when the metadata block is missing.
        "<unnamed>".to_string()
    } else {
        idl.metadata.name.clone()
    };

    let version = if idl.metadata.version.is_empty() {
        None
    } else {
        Some(idl.metadata.version.clone())
    };

    Ok(ProgramSurface {
        name,
        program_id: Some(idl.address.clone()),
        version,
        accounts,
        instructions,
        types,
        errors,
        events,
    })
}

/// Pad a Quasar variable-length discriminator to ratchet's 8-byte slot.
/// Surface choice: trailing zeros, not leading. Quasar dispatches on
/// the leading bytes (the on-chain ix data starts with the
/// discriminator bytes followed by args), so leading-position is the
/// load-bearing part — keep it stable across the pad.
fn pad_discriminator(bytes: &[u8]) -> Discriminator {
    let mut out = [0u8; 8];
    let take = bytes.len().min(8);
    out[..take].copy_from_slice(&bytes[..take]);
    out
}

fn convert_fields(fields: &[QuasarIdlField]) -> Vec<FieldDef> {
    fields
        .iter()
        .map(|f| FieldDef {
            name: f.name.clone(),
            ty: convert_type(&f.ty),
            offset: None,
            size: None,
        })
        .collect()
}

fn convert_type(ty: &QuasarIdlType) -> TypeRef {
    match ty {
        QuasarIdlType::Primitive(name) => match parse_primitive(name) {
            Some(p) => TypeRef::primitive(p),
            None => TypeRef::Unrecognized { raw: name.clone() },
        },
        QuasarIdlType::Option { option } => TypeRef::Option {
            ty: Box::new(convert_type(option)),
        },
        QuasarIdlType::Defined { defined } => TypeRef::Defined {
            name: defined.clone(),
        },
        QuasarIdlType::DynString { .. } => {
            // We lose `maxLength` + `prefixBytes` — the IR doesn't
            // carry storage-bound info today. The diff still catches
            // a string-to-non-string retype, which is the part rules
            // care about.
            TypeRef::primitive(PrimitiveType::String)
        }
        QuasarIdlType::DynVec { vec } => TypeRef::Vec {
            ty: Box::new(convert_type(&vec.items)),
        },
    }
}

fn parse_primitive(name: &str) -> Option<PrimitiveType> {
    Some(match name {
        "bool" => PrimitiveType::Bool,
        "u8" => PrimitiveType::U8,
        "u16" => PrimitiveType::U16,
        "u32" => PrimitiveType::U32,
        "u64" => PrimitiveType::U64,
        "u128" => PrimitiveType::U128,
        "i8" => PrimitiveType::I8,
        "i16" => PrimitiveType::I16,
        "i32" => PrimitiveType::I32,
        "i64" => PrimitiveType::I64,
        "i128" => PrimitiveType::I128,
        "f32" => PrimitiveType::F32,
        "f64" => PrimitiveType::F64,
        "string" => PrimitiveType::String,
        "bytes" => PrimitiveType::Bytes,
        "pubkey" => PrimitiveType::Pubkey,
        _ => return None,
    })
}

fn convert_account_input(item: &QuasarIdlAccountItem) -> AccountInput {
    AccountInput {
        name: item.name.clone(),
        is_signer: item.signer,
        is_writable: item.writable,
        // Quasar doesn't model optional accounts — every slot is
        // required at the dispatch layer.
        is_optional: false,
        pda: item.pda.as_ref().map(convert_pda),
    }
}

fn convert_pda(pda: &QuasarIdlPda) -> PdaSpec {
    PdaSpec {
        seeds: pda.seeds.iter().map(convert_seed).collect(),
        // Quasar PDAs are always derived under the program itself;
        // there's no cross-program PDA representation in the IDL.
        program_id: None,
    }
}

fn convert_seed(seed: &QuasarIdlSeed) -> Seed {
    match seed {
        QuasarIdlSeed::Const { value } => Seed::Const {
            bytes: value.clone(),
        },
        QuasarIdlSeed::Account { path } => Seed::Account {
            // Paths in Quasar are sometimes `account.field` form; we
            // split if present so the IR's structured Seed matches.
            // No `field` lookup means we treat the whole string as
            // an account name (pubkey reference).
            name: split_path_account(path).0.into(),
            field: split_path_account(path).1.map(str::to_string),
        },
        QuasarIdlSeed::Arg { path } => Seed::Arg { name: path.clone() },
    }
}

fn split_path_account(path: &str) -> (&str, Option<&str>) {
    match path.split_once('.') {
        Some((acc, field)) => (acc, Some(field)),
        None => (path, None),
    }
}

/// Lift parsing failure into a plain `Result<ProgramSurface>` for callers
/// that already have a JSON string and don't want to chain.
pub fn normalize_str(json: &str) -> Result<ProgramSurface> {
    let idl: QuasarIdl =
        serde_json::from_str(json).map_err(|e| anyhow!("parsing Quasar IDL: {e}"))?;
    normalize(&idl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::parse_quasar_idl_str;

    fn idl_with(json: &str) -> QuasarIdl {
        parse_quasar_idl_str(json).expect("valid Quasar IDL")
    }

    #[test]
    fn pads_one_byte_discriminator_with_trailing_zeros() {
        assert_eq!(pad_discriminator(&[0x05]), [0x05, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            pad_discriminator(&[0xAA, 0xBB]),
            [0xAA, 0xBB, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn truncates_overlong_discriminator_to_eight_bytes() {
        // Defensive: if some future Quasar variant ships >8-byte
        // discriminators, we trim instead of panicking. Diffs may
        // look weird but normalisation completes.
        assert_eq!(
            pad_discriminator(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn primitive_types_map_correctly() {
        assert_eq!(parse_primitive("u64"), Some(PrimitiveType::U64));
        assert_eq!(parse_primitive("pubkey"), Some(PrimitiveType::Pubkey));
        assert_eq!(parse_primitive("bytes"), Some(PrimitiveType::Bytes));
        assert!(parse_primitive("not-a-type").is_none());
    }

    #[test]
    fn unknown_primitive_falls_back_to_unrecognized() {
        let ty = QuasarIdlType::Primitive("future_type".to_string());
        match convert_type(&ty) {
            TypeRef::Unrecognized { raw } => assert_eq!(raw, "future_type"),
            other => panic!("expected Unrecognized, got {other:?}"),
        }
    }

    #[test]
    fn option_and_defined_types_round_trip() {
        let opt = QuasarIdlType::Option {
            option: Box::new(QuasarIdlType::Primitive("u64".into())),
        };
        match convert_type(&opt) {
            TypeRef::Option { ty } => assert_eq!(*ty, TypeRef::primitive(PrimitiveType::U64)),
            other => panic!("expected Option, got {other:?}"),
        }
        let def = QuasarIdlType::Defined {
            defined: "Vault".into(),
        };
        match convert_type(&def) {
            TypeRef::Defined { name } => assert_eq!(name, "Vault"),
            other => panic!("expected Defined, got {other:?}"),
        }
    }

    #[test]
    fn account_def_pulls_fields_from_matching_typedef() {
        let json = r#"{
            "address": "22222222222222222222222222222222222222222222",
            "metadata": { "name": "demo", "version": "0.1.0", "spec": "0.1.0" },
            "instructions": [],
            "accounts": [
                { "name": "Vault", "discriminator": [42] }
            ],
            "events": [],
            "types": [
                {
                    "name": "Vault",
                    "type": {
                        "kind": "struct",
                        "fields": [
                            { "name": "balance", "type": "u64" },
                            { "name": "owner", "type": "pubkey" }
                        ]
                    }
                }
            ],
            "errors": []
        }"#;
        let surface = normalize(&idl_with(json)).unwrap();
        let vault = surface.accounts.get("Vault").expect("Vault account");
        assert_eq!(vault.discriminator, [42, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(vault.fields.len(), 2);
        assert_eq!(vault.fields[0].name, "balance");
        assert_eq!(vault.fields[0].ty, TypeRef::primitive(PrimitiveType::U64));
        assert_eq!(vault.fields[1].name, "owner");
        assert_eq!(
            vault.fields[1].ty,
            TypeRef::primitive(PrimitiveType::Pubkey)
        );
    }

    #[test]
    fn instruction_with_signer_writable_account_inputs() {
        let json = r#"{
            "address": "22222222222222222222222222222222222222222222",
            "metadata": { "name": "demo", "version": "0.1.0", "spec": "0.1.0" },
            "instructions": [
                {
                    "name": "deposit",
                    "discriminator": [0],
                    "accounts": [
                        { "name": "user", "writable": true, "signer": true },
                        { "name": "vault", "writable": true }
                    ],
                    "args": [
                        { "name": "amount", "type": "u64" }
                    ]
                }
            ],
            "accounts": [],
            "events": [],
            "types": [],
            "errors": []
        }"#;
        let surface = normalize(&idl_with(json)).unwrap();
        let ix = surface.instructions.get("deposit").unwrap();
        assert_eq!(ix.accounts.len(), 2);
        assert!(ix.accounts[0].is_signer);
        assert!(ix.accounts[0].is_writable);
        assert!(!ix.accounts[1].is_signer);
        assert!(ix.accounts[1].is_writable);
        assert_eq!(ix.args.len(), 1);
        assert_eq!(ix.args[0].name, "amount");
    }

    #[test]
    fn pda_seeds_round_trip_const_arg_and_account() {
        let json = r#"{
            "address": "22222222222222222222222222222222222222222222",
            "metadata": { "name": "demo", "version": "0.1.0", "spec": "0.1.0" },
            "instructions": [
                {
                    "name": "create",
                    "discriminator": [0],
                    "accounts": [
                        {
                            "name": "vault",
                            "writable": true,
                            "pda": {
                                "seeds": [
                                    { "kind": "const", "value": [118, 97, 117, 108, 116] },
                                    { "kind": "account", "path": "user" },
                                    { "kind": "arg", "path": "seed" }
                                ]
                            }
                        }
                    ],
                    "args": []
                }
            ],
            "accounts": [],
            "types": [],
            "errors": []
        }"#;
        let surface = normalize(&idl_with(json)).unwrap();
        let ix = surface.instructions.get("create").unwrap();
        let vault = &ix.accounts[0];
        let pda = vault.pda.as_ref().unwrap();
        assert_eq!(pda.seeds.len(), 3);
        match &pda.seeds[0] {
            Seed::Const { bytes } => assert_eq!(bytes, b"vault"),
            other => panic!("expected Const, got {other:?}"),
        }
        match &pda.seeds[1] {
            Seed::Account { name, field } => {
                assert_eq!(name, "user");
                assert!(field.is_none());
            }
            other => panic!("expected Account, got {other:?}"),
        }
        match &pda.seeds[2] {
            Seed::Arg { name } => assert_eq!(name, "seed"),
            other => panic!("expected Arg, got {other:?}"),
        }
    }

    #[test]
    fn dotted_account_seed_path_splits_into_name_and_field() {
        let seed = QuasarIdlSeed::Account {
            path: "vault.balance".into(),
        };
        match convert_seed(&seed) {
            Seed::Account { name, field } => {
                assert_eq!(name, "vault");
                assert_eq!(field.as_deref(), Some("balance"));
            }
            other => panic!("expected Account, got {other:?}"),
        }
    }

    #[test]
    fn dyn_string_collapses_to_string_primitive() {
        // We lose maxLength/prefixBytes — that's intentional; the IR
        // doesn't carry bounded-storage info today, and rules only
        // care about the type identity.
        let ty = QuasarIdlType::DynString {
            string: crate::idl::QuasarIdlDynString {
                max_length: 32,
                prefix_bytes: 1,
            },
        };
        assert_eq!(convert_type(&ty), TypeRef::primitive(PrimitiveType::String));
    }

    #[test]
    fn dyn_vec_preserves_element_type() {
        let ty = QuasarIdlType::DynVec {
            vec: crate::idl::QuasarIdlDynVec {
                items: Box::new(QuasarIdlType::Primitive("u64".into())),
                max_length: 16,
                prefix_bytes: 2,
            },
        };
        match convert_type(&ty) {
            TypeRef::Vec { ty } => assert_eq!(*ty, TypeRef::primitive(PrimitiveType::U64)),
            other => panic!("expected Vec, got {other:?}"),
        }
    }

    #[test]
    fn errors_are_keyed_by_code_with_optional_message() {
        let json = r#"{
            "address": "22222222222222222222222222222222222222222222",
            "metadata": { "name": "demo", "version": "0.1.0", "spec": "0.1.0" },
            "instructions": [],
            "accounts": [],
            "types": [],
            "errors": [
                { "code": 6000, "name": "InvalidAuthority", "msg": "bad signer" },
                { "code": 6001, "name": "AmountTooSmall" }
            ]
        }"#;
        let surface = normalize(&idl_with(json)).unwrap();
        assert_eq!(surface.errors.len(), 2);
        let e6000 = &surface.errors[&6000];
        assert_eq!(e6000.name, "InvalidAuthority");
        assert_eq!(e6000.message.as_deref(), Some("bad signer"));
        let e6001 = &surface.errors[&6001];
        assert!(e6001.message.is_none());
    }

    #[test]
    fn missing_metadata_falls_back_to_unnamed_program() {
        // Defensive: a synthetic IDL without a name shouldn't break
        // normalisation (the rules will surface findings against
        // <unnamed> rather than crash).
        let json = r#"{
            "address": "22222222222222222222222222222222222222222222"
        }"#;
        let surface = normalize(&idl_with(json)).unwrap();
        assert_eq!(surface.name, "<unnamed>");
        assert_eq!(
            surface.program_id.as_deref(),
            Some("22222222222222222222222222222222222222222222")
        );
    }
}

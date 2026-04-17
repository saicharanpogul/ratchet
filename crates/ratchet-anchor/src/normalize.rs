//! Lower a parsed [`AnchorIdl`] into the framework-agnostic
//! [`ProgramSurface`].
//!
//! The Anchor IDL has two awkward corners:
//!
//! - account discriminators are optional (pre-0.30 IDLs omit them), so the
//!   normalizer computes the Anchor defaults when they are missing:
//!   `sha256("account:<Name>")[..8]` for accounts and
//!   `sha256("global:<snake_case_name>")[..8]` for instructions.
//! - type references are preserved as raw JSON in [`AnchorIdlType`], either a
//!   primitive string or a single-key object. The normalizer resolves those
//!   into [`ratchet_core::TypeRef`].

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use ratchet_core::{
    AccountDef, AccountInput, ArgDef, Discriminator, EnumVariant, EnumVariantFields, ErrorDef,
    EventDef, FieldDef, InstructionDef, PdaSpec, PrimitiveType, ProgramSurface, Seed, TypeDef,
    TypeRef,
};
use sha2::{Digest, Sha256};

use crate::idl::{
    AnchorIdl, AnchorIdlAccountItem, AnchorIdlEnumVariant, AnchorIdlField, AnchorIdlPda,
    AnchorIdlSeed, AnchorIdlStructFields, AnchorIdlType, AnchorIdlTypeDefKind,
};

/// Lower an [`AnchorIdl`] into a [`ProgramSurface`].
pub fn normalize(idl: &AnchorIdl) -> Result<ProgramSurface> {
    let mut surface = ProgramSurface {
        program_id: idl.address.clone(),
        ..Default::default()
    };

    if let Some(meta) = &idl.metadata {
        surface.name = meta.name.clone();
        surface.version = meta.version.clone();
    }

    // Types first, so accounts and instructions can resolve their fields by name.
    let types = idl
        .types
        .iter()
        .map(|td| {
            let def = match &td.def {
                AnchorIdlTypeDefKind::Struct { fields } => TypeDef::Struct {
                    fields: parse_fields(fields.as_ref())?,
                },
                AnchorIdlTypeDefKind::Enum { variants } => TypeDef::Enum {
                    variants: parse_variants(variants)?,
                },
                AnchorIdlTypeDefKind::Type { alias } => TypeDef::Alias {
                    target: parse_type_ref(alias)?,
                },
            };
            Ok((td.name.clone(), def))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    surface.types = types;

    // Accounts: header (name + optional disc) plus fields resolved from types.
    for header in &idl.accounts {
        let discriminator: Discriminator = header
            .discriminator
            .unwrap_or_else(|| default_account_discriminator(&header.name));
        let fields = match surface.types.get(&header.name) {
            Some(TypeDef::Struct { fields }) => fields.clone(),
            Some(_) => bail!(
                "account `{}` refers to a non-struct type in `types`",
                header.name
            ),
            None => Vec::new(), // account with no fields defined in types — rare but permitted
        };
        surface.accounts.insert(
            header.name.clone(),
            AccountDef {
                name: header.name.clone(),
                discriminator,
                fields,
                size: None,
            },
        );
    }

    // Instructions
    for ix in &idl.instructions {
        let discriminator = ix
            .discriminator
            .unwrap_or_else(|| default_instruction_discriminator(&ix.name));

        let mut accounts = Vec::new();
        for item in &ix.accounts {
            flatten_account(item, &mut accounts, "")?;
        }

        let args = ix
            .args
            .iter()
            .map(|a| {
                Ok(ArgDef {
                    name: a.name.clone(),
                    ty: parse_type_ref(&a.ty)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        surface.instructions.insert(
            ix.name.clone(),
            InstructionDef {
                name: ix.name.clone(),
                discriminator,
                args,
                accounts,
            },
        );
    }

    for e in &idl.errors {
        surface.errors.insert(
            e.code,
            ErrorDef {
                code: e.code,
                name: e.name.clone(),
                message: e.msg.clone(),
            },
        );
    }

    // Events. Discriminator defaults to sha256("event:<Name>")[..8] when
    // the IDL omits it (pre-0.30 IDLs strip the field).
    for event in &idl.events {
        let discriminator = event
            .discriminator
            .unwrap_or_else(|| default_event_discriminator(&event.name));
        surface.events.insert(
            event.name.clone(),
            EventDef {
                name: event.name.clone(),
                discriminator,
            },
        );
    }

    Ok(surface)
}

/// Default Anchor event discriminator: `sha256("event:<Name>")[..8]`.
pub fn default_event_discriminator(name: &str) -> Discriminator {
    let digest = Sha256::digest(format!("event:{name}").as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

fn parse_fields(fields: Option<&AnchorIdlStructFields>) -> Result<Vec<FieldDef>> {
    let Some(fields) = fields else {
        return Ok(Vec::new());
    };
    match fields {
        AnchorIdlStructFields::Named(named) => named
            .iter()
            .map(parse_named_field)
            .collect::<Result<Vec<_>>>(),
        AnchorIdlStructFields::Tuple(types) => types
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                Ok(FieldDef {
                    name: i.to_string(),
                    ty: parse_type_ref(ty)?,
                    offset: None,
                    size: None,
                })
            })
            .collect(),
    }
}

fn parse_named_field(f: &AnchorIdlField) -> Result<FieldDef> {
    Ok(FieldDef {
        name: f.name.clone(),
        ty: parse_type_ref(&f.ty)?,
        offset: None,
        size: None,
    })
}

fn parse_variants(variants: &[AnchorIdlEnumVariant]) -> Result<Vec<EnumVariant>> {
    variants
        .iter()
        .map(|v| {
            let fields = match &v.fields {
                None => EnumVariantFields::Unit,
                Some(AnchorIdlStructFields::Named(named)) => EnumVariantFields::Named {
                    fields: named.iter().map(parse_named_field).collect::<Result<_>>()?,
                },
                Some(AnchorIdlStructFields::Tuple(types)) => EnumVariantFields::Tuple {
                    types: types
                        .iter()
                        .map(parse_type_ref)
                        .collect::<Result<Vec<_>>>()?,
                },
            };
            Ok(EnumVariant {
                name: v.name.clone(),
                fields,
            })
        })
        .collect()
}

fn parse_type_ref(ty: &AnchorIdlType) -> Result<TypeRef> {
    match ty {
        AnchorIdlType::Primitive(s) => parse_primitive(s),
        AnchorIdlType::Object(v) => parse_complex(v),
    }
}

fn parse_primitive(s: &str) -> Result<TypeRef> {
    let prim = match s {
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
        "pubkey" | "publicKey" => PrimitiveType::Pubkey,
        other => {
            // Unknown string in primitive position. Could be:
            // (a) a user-defined type referenced by bare name (rare but
            //     accepted by older Anchor IDLs) — will be resolved by
            //     ProgramSurface.types if present.
            // (b) a future Anchor primitive we don't model yet.
            // We return `Unrecognized` rather than `Defined` so the
            //     diff engine surfaces the raw string explicitly and
            //     validate.rs knows not to assume a size.
            return Ok(TypeRef::Unrecognized { raw: other.into() });
        }
    };
    Ok(TypeRef::primitive(prim))
}

fn parse_complex(v: &serde_json::Value) -> Result<TypeRef> {
    let obj = v
        .as_object()
        .ok_or_else(|| anyhow!("type object expected, got {v}"))?;
    if let Some(inner) = obj.get("vec") {
        return Ok(TypeRef::Vec {
            ty: Box::new(parse_value_as_type_ref(inner)?),
        });
    }
    if let Some(inner) = obj.get("option") {
        return Ok(TypeRef::Option {
            ty: Box::new(parse_value_as_type_ref(inner)?),
        });
    }
    if let Some(inner) = obj.get("array") {
        let arr = inner.as_array().context("array type must be [T, N]")?;
        if arr.len() != 2 {
            bail!("array type expects 2 elements, got {}", arr.len());
        }
        let element = parse_value_as_type_ref(&arr[0])?;
        let len = arr[1].as_u64().context("array length must be a number")?;
        return Ok(TypeRef::Array {
            ty: Box::new(element),
            len: len as usize,
        });
    }
    if let Some(inner) = obj.get("defined") {
        let name = match inner {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(o) => o
                .get("name")
                .and_then(|n| n.as_str())
                .context("`defined` object requires a `name`")?
                .to_string(),
            _ => bail!("unexpected shape for `defined` type: {inner}"),
        };
        return Ok(TypeRef::Defined { name });
    }
    // Unknown type constructors (coption, hashMap, generics, ...) are
    // preserved verbatim — the raw JSON is kept so a retype like
    // `coption<u64> → coption<u32>` still differs when the inner type
    // changes. `Unrecognized` keeps these distinct from user-defined
    // `Defined` references in downstream tooling and min-size accounting.
    let raw = serde_json::to_string(v).unwrap_or_else(|_| String::from("<unserializable>"));
    Ok(TypeRef::Unrecognized { raw })
}

fn parse_value_as_type_ref(v: &serde_json::Value) -> Result<TypeRef> {
    match v {
        serde_json::Value::String(s) => parse_primitive(s),
        serde_json::Value::Object(_) => parse_complex(v),
        _ => bail!("expected a type reference, got {v}"),
    }
}

fn flatten_account(
    item: &AnchorIdlAccountItem,
    out: &mut Vec<AccountInput>,
    prefix: &str,
) -> Result<()> {
    match item {
        AnchorIdlAccountItem::Single(a) => {
            let name = if prefix.is_empty() {
                a.name.clone()
            } else {
                format!("{prefix}.{}", a.name)
            };
            out.push(AccountInput {
                name,
                is_signer: a.signer,
                is_writable: a.writable,
                is_optional: a.optional,
                pda: a.pda.as_ref().map(normalize_pda).transpose()?,
            });
            Ok(())
        }
        AnchorIdlAccountItem::Composite(c) => {
            let next_prefix = if prefix.is_empty() {
                c.name.clone()
            } else {
                format!("{prefix}.{}", c.name)
            };
            for inner in &c.accounts {
                flatten_account(inner, out, &next_prefix)?;
            }
            Ok(())
        }
    }
}

fn normalize_pda(pda: &AnchorIdlPda) -> Result<PdaSpec> {
    let seeds = pda
        .seeds
        .iter()
        .map(normalize_seed)
        .collect::<Result<Vec<_>>>()?;
    // If the IDL encodes the PDA as being derived off another program,
    // try to resolve a literal pubkey. For Arg / Account-referenced
    // program ids we keep the information but can't render a concrete
    // base58 — fall back to a printable raw form so R013 can still diff
    // "the program target changed" without invoking curve math.
    let program_id = pda.program.as_ref().map(|p| render_program_seed(p));
    Ok(PdaSpec { seeds, program_id })
}

fn render_program_seed(seed: &AnchorIdlSeed) -> String {
    match seed {
        AnchorIdlSeed::Const { value } => {
            // 32-byte literal program id: encode as base58 so diffs show
            // a human-comparable pubkey. Non-32-byte literals fall back
            // to a hex rendering.
            if value.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(value);
                bs58::encode(arr).into_string()
            } else {
                let hex: String = value.iter().map(|b| format!("{b:02x}")).collect();
                format!("const:0x{hex}")
            }
        }
        AnchorIdlSeed::Arg { path } => format!("arg:{path}"),
        AnchorIdlSeed::Account { path, account } => match account {
            Some(a) => format!("account:{path}::{a}"),
            None => format!("account:{path}"),
        },
    }
}

fn normalize_seed(seed: &AnchorIdlSeed) -> Result<Seed> {
    Ok(match seed {
        AnchorIdlSeed::Const { value } => Seed::Const {
            bytes: value.clone(),
        },
        AnchorIdlSeed::Arg { path } => Seed::Arg { name: path.clone() },
        AnchorIdlSeed::Account { path, account: _ } => Seed::Account {
            name: path.clone(),
            field: None,
        },
    })
}

/// Default Anchor account discriminator: `sha256("account:<Name>")[..8]`.
pub fn default_account_discriminator(name: &str) -> Discriminator {
    let digest = Sha256::digest(format!("account:{name}").as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

/// Default Anchor instruction discriminator:
/// `sha256("global:<snake_case_name>")[..8]`.
pub fn default_instruction_discriminator(name: &str) -> Discriminator {
    let digest = Sha256::digest(format!("global:{name}").as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_IDL: &str = r#"{
        "address": "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS",
        "metadata": { "name": "vault", "version": "0.1.0" },
        "instructions": [
            {
                "name": "deposit",
                "discriminator": [242, 35, 198, 137, 82, 225, 242, 182],
                "accounts": [
                    { "name": "user", "writable": true, "signer": true },
                    {
                        "name": "vault",
                        "writable": true,
                        "pda": {
                            "seeds": [
                                { "kind": "const", "value": [118, 97, 117, 108, 116] },
                                { "kind": "account", "path": "user" }
                            ]
                        }
                    }
                ],
                "args": [
                    { "name": "amount", "type": "u64" },
                    { "name": "memo", "type": { "option": "string" } },
                    { "name": "counts", "type": { "array": ["u8", 4] } },
                    { "name": "tags", "type": { "vec": "string" } }
                ]
            }
        ],
        "accounts": [
            { "name": "Vault", "discriminator": [211, 8, 232, 43, 2, 152, 117, 119] }
        ],
        "types": [
            {
                "name": "Vault",
                "type": {
                    "kind": "struct",
                    "fields": [
                        { "name": "owner", "type": "pubkey" },
                        { "name": "balance", "type": "u64" },
                        { "name": "nested", "type": { "defined": { "name": "Metadata" } } }
                    ]
                }
            },
            {
                "name": "Metadata",
                "type": {
                    "kind": "struct",
                    "fields": [
                        { "name": "kind", "type": { "defined": "Side" } }
                    ]
                }
            },
            {
                "name": "Side",
                "type": {
                    "kind": "enum",
                    "variants": [
                        { "name": "Bid" },
                        { "name": "Ask" }
                    ]
                }
            }
        ],
        "errors": [
            { "code": 6000, "name": "InsufficientBalance", "msg": "balance too low" }
        ]
    }"#;

    #[test]
    fn normalizes_sample_idl() {
        let idl: AnchorIdl = serde_json::from_str(SAMPLE_IDL).unwrap();
        let surface = normalize(&idl).unwrap();

        assert_eq!(surface.name, "vault");
        assert_eq!(surface.version.as_deref(), Some("0.1.0"));
        assert_eq!(surface.accounts.len(), 1);
        assert_eq!(surface.types.len(), 3);

        let vault = &surface.accounts["Vault"];
        assert_eq!(vault.discriminator, [211, 8, 232, 43, 2, 152, 117, 119]);
        assert_eq!(vault.fields.len(), 3);
        assert_eq!(vault.fields[0].name, "owner");
        assert_eq!(
            vault.fields[0].ty,
            TypeRef::primitive(PrimitiveType::Pubkey)
        );
        assert_eq!(
            vault.fields[2].ty,
            TypeRef::Defined {
                name: "Metadata".into()
            }
        );

        let deposit = &surface.instructions["deposit"];
        assert_eq!(deposit.accounts.len(), 2);
        assert_eq!(deposit.args.len(), 4);
        assert_eq!(deposit.args[0].ty, TypeRef::primitive(PrimitiveType::U64));
        match &deposit.args[1].ty {
            TypeRef::Option { ty } => {
                assert_eq!(**ty, TypeRef::primitive(PrimitiveType::String));
            }
            other => panic!("expected option, got {other:?}"),
        }
        match &deposit.args[2].ty {
            TypeRef::Array { ty, len } => {
                assert_eq!(**ty, TypeRef::primitive(PrimitiveType::U8));
                assert_eq!(*len, 4);
            }
            other => panic!("expected array, got {other:?}"),
        }
        match &deposit.args[3].ty {
            TypeRef::Vec { ty } => {
                assert_eq!(**ty, TypeRef::primitive(PrimitiveType::String));
            }
            other => panic!("expected vec, got {other:?}"),
        }

        let vault_input = &deposit.accounts[1];
        assert_eq!(vault_input.name, "vault");
        assert!(vault_input.is_writable);
        let pda = vault_input.pda.as_ref().unwrap();
        assert_eq!(pda.seeds.len(), 2);
        match &pda.seeds[0] {
            Seed::Const { bytes } => assert_eq!(bytes, &b"vault".to_vec()),
            _ => panic!("expected const seed"),
        }
    }

    #[test]
    fn discriminator_defaults_when_missing() {
        let idl: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": { "name": "legacy" },
                "instructions": [
                    { "name": "do_thing", "accounts": [], "args": [] }
                ],
                "accounts": [
                    { "name": "State" }
                ],
                "types": [
                    {
                        "name": "State",
                        "type": { "kind": "struct", "fields": [] }
                    }
                ]
            }"#,
        )
        .unwrap();
        let surface = normalize(&idl).unwrap();
        assert_eq!(
            surface.accounts["State"].discriminator,
            default_account_discriminator("State")
        );
        assert_eq!(
            surface.instructions["do_thing"].discriminator,
            default_instruction_discriminator("do_thing")
        );
    }

    #[test]
    fn default_discriminator_sha256_global_prefix() {
        // `global:initialize` → sha256 first 8 bytes.
        let expected: [u8; 8] = {
            let d = sha2::Sha256::digest(b"global:initialize");
            [d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
        };
        assert_eq!(default_instruction_discriminator("initialize"), expected);
    }

    #[test]
    fn events_are_normalized_with_defaults() {
        let idl: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": { "name": "p" },
                "instructions": [],
                "accounts": [],
                "types": [],
                "events": [
                    { "name": "Deposited", "discriminator": [1,2,3,4,5,6,7,8] },
                    { "name": "Withdrawn" }
                ]
            }"#,
        )
        .unwrap();
        let surface = normalize(&idl).unwrap();
        assert_eq!(surface.events.len(), 2);
        assert_eq!(
            surface.events["Deposited"].discriminator,
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(
            surface.events["Withdrawn"].discriminator,
            default_event_discriminator("Withdrawn")
        );
    }

    #[test]
    fn default_event_discriminator_uses_event_prefix() {
        let expected: [u8; 8] = {
            let d = sha2::Sha256::digest(b"event:Deposited");
            [d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]
        };
        assert_eq!(default_event_discriminator("Deposited"), expected);
    }

    #[test]
    fn unknown_primitive_becomes_unrecognized() {
        let idl: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": { "name": "p" },
                "instructions": [
                    {
                        "name": "doit",
                        "accounts": [],
                        "args": [{ "name": "amt", "type": "u256" }]
                    }
                ],
                "accounts": [],
                "types": []
            }"#,
        )
        .unwrap();
        let surface = normalize(&idl).unwrap();
        match &surface.instructions["doit"].args[0].ty {
            TypeRef::Unrecognized { raw } => assert_eq!(raw, "u256"),
            other => panic!("expected Unrecognized, got {other:?}"),
        }
    }

    #[test]
    fn unknown_complex_preserves_inner_type() {
        // Two IDLs that differ only in the inner type of a coption
        // constructor. Under the old "unsupported:coption" flattening
        // they'd compare equal; with Unrecognized their raw JSON
        // differs and the diff engine notices.
        let old: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": {"name":"p"},
                "instructions":[{
                    "name":"doit","accounts":[],
                    "args":[{"name":"x","type":{"coption":"u64"}}]
                }],
                "accounts":[],"types":[]
            }"#,
        )
        .unwrap();
        let new: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": {"name":"p"},
                "instructions":[{
                    "name":"doit","accounts":[],
                    "args":[{"name":"x","type":{"coption":"u32"}}]
                }],
                "accounts":[],"types":[]
            }"#,
        )
        .unwrap();
        let old_s = normalize(&old).unwrap();
        let new_s = normalize(&new).unwrap();
        let old_ty = &old_s.instructions["doit"].args[0].ty;
        let new_ty = &new_s.instructions["doit"].args[0].ty;
        assert_ne!(
            old_ty, new_ty,
            "coption<u64> and coption<u32> must not compare equal"
        );
        match (old_ty, new_ty) {
            (TypeRef::Unrecognized { raw: a }, TypeRef::Unrecognized { raw: b }) => {
                assert!(a.contains("u64"));
                assert!(b.contains("u32"));
            }
            _ => panic!("expected both Unrecognized"),
        }
    }

    #[test]
    fn pda_program_preserved_as_base58() {
        let idl: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": { "name": "p" },
                "instructions": [
                    {
                        "name": "doit",
                        "accounts": [
                            {
                                "name": "acct",
                                "pda": {
                                    "seeds": [{ "kind": "const", "value": [1, 2, 3] }],
                                    "program": {
                                        "kind": "const",
                                        "value": [6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133, 237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169]
                                    }
                                }
                            }
                        ],
                        "args": []
                    }
                ],
                "accounts": [],
                "types": []
            }"#,
        )
        .unwrap();
        let surface = normalize(&idl).unwrap();
        let pda = surface.instructions["doit"].accounts[0]
            .pda
            .as_ref()
            .unwrap();
        // Base58 of the 32 literal bytes above — this happens to be the
        // SPL Token program id; what matters for the test is that the
        // normalizer round-trips bytes → base58 losslessly.
        assert_eq!(
            pda.program_id.as_deref(),
            Some("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        );
    }

    #[test]
    fn pda_program_from_account_seed_uses_raw_label() {
        let idl: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": { "name": "p" },
                "instructions": [
                    {
                        "name": "doit",
                        "accounts": [
                            {
                                "name": "acct",
                                "pda": {
                                    "seeds": [{ "kind": "const", "value": [1] }],
                                    "program": { "kind": "account", "path": "remote_program" }
                                }
                            }
                        ],
                        "args": []
                    }
                ],
                "accounts": [],
                "types": []
            }"#,
        )
        .unwrap();
        let surface = normalize(&idl).unwrap();
        let pda = surface.instructions["doit"].accounts[0]
            .pda
            .as_ref()
            .unwrap();
        assert_eq!(pda.program_id.as_deref(), Some("account:remote_program"));
    }

    #[test]
    fn composite_accounts_are_flattened_with_prefixed_names() {
        let idl: AnchorIdl = serde_json::from_str(
            r#"{
                "metadata": { "name": "nest" },
                "instructions": [
                    {
                        "name": "doit",
                        "accounts": [
                            {
                                "name": "group",
                                "accounts": [
                                    { "name": "a", "signer": true },
                                    { "name": "b", "writable": true }
                                ]
                            },
                            { "name": "c" }
                        ],
                        "args": []
                    }
                ],
                "accounts": [],
                "types": []
            }"#,
        )
        .unwrap();
        let surface = normalize(&idl).unwrap();
        let ix = &surface.instructions["doit"];
        assert_eq!(ix.accounts.len(), 3);
        assert_eq!(ix.accounts[0].name, "group.a");
        assert!(ix.accounts[0].is_signer);
        assert_eq!(ix.accounts[1].name, "group.b");
        assert!(ix.accounts[1].is_writable);
        assert_eq!(ix.accounts[2].name, "c");
    }
}

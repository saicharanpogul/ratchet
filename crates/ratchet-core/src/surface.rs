//! Framework-agnostic IR of a Solana program's public surface.
//!
//! A [`ProgramSurface`] is everything a rule needs to decide whether an upgrade
//! is safe: the accounts (with field order and discriminators), the
//! instructions (with argument order, account list, and signer/writable
//! flags), the user-defined types (structs and enums), and the errors.
//!
//! Both sides of a diff — the deployed program and the candidate program —
//! are first normalized into this IR. Rules then operate on `&ProgramSurface`
//! pairs without caring whether the input came from an Anchor IDL, a Quasar
//! schema, or a hand-annotated native program.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Eight-byte selector that Anchor uses to route deserialization and
/// dispatch. For accounts it is conventionally
/// `sha256("account:<StructName>")[..8]`; for instructions,
/// `sha256("global:<snake_case_name>")[..8]`. Both can be overridden
/// explicitly as of Anchor 0.31.
pub type Discriminator = [u8; 8];

/// A Solana program's public surface, normalized across frameworks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProgramSurface {
    /// Program name, e.g. `"my_program"`. Typically taken from the IDL
    /// metadata.
    pub name: String,
    /// Base58-encoded program id, if known. May be `None` when the surface
    /// comes from local source without a deployed address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Semver or free-form version string from IDL metadata, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Accounts owned by the program, keyed by account struct name. Field
    /// order inside each account is preserved (layout is ordinal).
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountDef>,
    /// Instructions exposed by the program, keyed by snake_case name.
    #[serde(default)]
    pub instructions: BTreeMap<String, InstructionDef>,
    /// User-defined types referenced by accounts or instructions.
    #[serde(default)]
    pub types: BTreeMap<String, TypeDef>,
    /// Program errors keyed by error code.
    #[serde(default)]
    pub errors: BTreeMap<u32, ErrorDef>,
}

/// A struct that the program stores on-chain (the `#[account]` shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDef {
    pub name: String,
    /// Eight-byte selector; pre-resolved by the loader.
    #[serde(with = "discriminator_hex")]
    pub discriminator: Discriminator,
    /// Fields in declaration order. Order is load-bearing: Borsh lays them
    /// out sequentially.
    pub fields: Vec<FieldDef>,
    /// Total serialized size in bytes, if statically knowable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// A single field inside an account or user-defined struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRef,
    /// Byte offset inside the containing struct, if resolvable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    /// Serialized size in bytes for fixed-size types, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// A program instruction (dispatch target).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionDef {
    pub name: String,
    #[serde(with = "discriminator_hex")]
    pub discriminator: Discriminator,
    /// Arguments passed in the instruction data, in declaration order.
    pub args: Vec<ArgDef>,
    /// Account inputs (the `Accounts` context), in declaration order.
    /// Order matters: the Solana runtime dispatches by index.
    pub accounts: Vec<AccountInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRef,
}

/// A single account slot in an instruction's `Accounts` context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInput {
    pub name: String,
    #[serde(default)]
    pub is_signer: bool,
    #[serde(default)]
    pub is_writable: bool,
    #[serde(default)]
    pub is_optional: bool,
    /// PDA seed expression, if the account is derived. Captured where the
    /// source (or IDL) makes it available; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pda: Option<PdaSpec>,
}

/// A PDA derivation specification for an account input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdaSpec {
    pub seeds: Vec<Seed>,
    /// Explicit program id for the PDA if it is derived off another program.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
}

/// One component of a PDA seed expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Seed {
    /// Literal bytes, e.g. `b"vault"`.
    Const { bytes: Vec<u8> },
    /// Reference to an instruction argument by name.
    Arg { name: String },
    /// Reference to another account (usually its pubkey) in the same
    /// instruction's `Accounts` context, optionally into a field of its data.
    Account {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        field: Option<String>,
    },
    /// The loader could not determine the seed statically.
    Unknown { raw: String },
}

/// A user-defined type referenced by accounts or instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeDef {
    Struct { fields: Vec<FieldDef> },
    Enum { variants: Vec<EnumVariant> },
    Alias { target: TypeRef },
}

/// A variant of a user-defined enum. Variant order is load-bearing because
/// Borsh tags variants by ordinal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub fields: EnumVariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnumVariantFields {
    Unit,
    Tuple { types: Vec<TypeRef> },
    Named { fields: Vec<FieldDef> },
}

/// An error code declared by the program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDef {
    pub code: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Primitive types recognized by the IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrimitiveType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    F32,
    F64,
    String,
    Bytes,
    Pubkey,
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "u8",
            PrimitiveType::U16 => "u16",
            PrimitiveType::U32 => "u32",
            PrimitiveType::U64 => "u64",
            PrimitiveType::U128 => "u128",
            PrimitiveType::I8 => "i8",
            PrimitiveType::I16 => "i16",
            PrimitiveType::I32 => "i32",
            PrimitiveType::I64 => "i64",
            PrimitiveType::I128 => "i128",
            PrimitiveType::F32 => "f32",
            PrimitiveType::F64 => "f64",
            PrimitiveType::String => "string",
            PrimitiveType::Bytes => "bytes",
            PrimitiveType::Pubkey => "pubkey",
        })
    }
}

/// Reference to a type — primitive, composite collection, or a named
/// user-defined type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeRef {
    Primitive {
        ty: PrimitiveType,
    },
    Array {
        #[serde(rename = "type")]
        ty: Box<TypeRef>,
        len: usize,
    },
    Vec {
        #[serde(rename = "type")]
        ty: Box<TypeRef>,
    },
    Option {
        #[serde(rename = "type")]
        ty: Box<TypeRef>,
    },
    /// Reference to a user-defined type (found in `ProgramSurface::types`).
    Defined {
        name: String,
    },
    /// The normalizer saw something it couldn't classify — a primitive
    /// string we don't recognize (future Anchor additions, typos) or a
    /// complex-type constructor we don't model (`coption`, `hashMap`,
    /// generics, …). `raw` captures the full JSON or primitive name so
    /// a diff can still tell `coption<u64> → coption<u32>` apart.
    Unrecognized {
        raw: String,
    },
}

impl TypeRef {
    pub const fn primitive(ty: PrimitiveType) -> Self {
        TypeRef::Primitive { ty }
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeRef::Primitive { ty } => write!(f, "{ty}"),
            TypeRef::Array { ty, len } => write!(f, "[{ty}; {len}]"),
            TypeRef::Vec { ty } => write!(f, "Vec<{ty}>"),
            TypeRef::Option { ty } => write!(f, "Option<{ty}>"),
            TypeRef::Defined { name } => write!(f, "{name}"),
            TypeRef::Unrecognized { raw } => write!(f, "unrecognized<{raw}>"),
        }
    }
}

mod discriminator_hex {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(disc: &[u8; 8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(16);
        for b in disc {
            out.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 8], D::Error> {
        let s = String::deserialize(d)?;
        if s.len() != 16 {
            return Err(serde::de::Error::custom(format!(
                "expected 16 hex chars, got {}",
                s.len()
            )));
        }
        let mut out = [0u8; 8];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_ref_display_primitive() {
        let t = TypeRef::primitive(PrimitiveType::U64);
        assert_eq!(t.to_string(), "u64");
    }

    #[test]
    fn type_ref_display_nested() {
        let t = TypeRef::Vec {
            ty: Box::new(TypeRef::Option {
                ty: Box::new(TypeRef::primitive(PrimitiveType::Pubkey)),
            }),
        };
        assert_eq!(t.to_string(), "Vec<Option<pubkey>>");
    }

    #[test]
    fn type_ref_display_array_and_defined() {
        let t = TypeRef::Array {
            ty: Box::new(TypeRef::Defined {
                name: "VaultState".into(),
            }),
            len: 4,
        };
        assert_eq!(t.to_string(), "[VaultState; 4]");
    }

    #[test]
    fn discriminator_hex_round_trip() {
        let acc = AccountDef {
            name: "Vault".into(),
            discriminator: [0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04],
            fields: vec![],
            size: None,
        };
        let j = serde_json::to_string(&acc).unwrap();
        assert!(j.contains("\"deadbeef01020304\""));
        let back: AccountDef = serde_json::from_str(&j).unwrap();
        assert_eq!(back.discriminator, acc.discriminator);
    }

    #[test]
    fn empty_surface_round_trips() {
        let s = ProgramSurface {
            name: "prog".into(),
            ..Default::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: ProgramSurface = serde_json::from_str(&j).unwrap();
        assert_eq!(back.name, "prog");
    }
}

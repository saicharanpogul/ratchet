//! Quasar IDL JSON shape, mirroring `blueshift-gg/quasar`'s
//! `schema/src/lib.rs`. Kept narrow: only the fields the normaliser
//! reads. New Quasar fields can be added behind `#[serde(default)]`
//! without breaking existing fixtures.
//!
//! Verified against the Quasar source at
//! `github.com/blueshift-gg/quasar/blob/master/schema/src/lib.rs` —
//! when their schema evolves, update here in lockstep.

use serde::Deserialize;

/// Top-level Quasar IDL document. Notable differences from Anchor:
///
/// - `address` is at the top level (Anchor nests program id in metadata).
/// - `discriminator` fields are `Vec<u8>` (typically 1 byte) rather
///   than `[u8; 8]` sha256 prefixes.
/// - `IdlType` is an untagged union with `option`/`defined`/string/vec
///   variants instead of Anchor's `{kind, type}` tagged shape.
/// - Type-defs only support `Struct` (no enums yet — see Quasar
///   `IdlTypeDefKind`).
/// - PDAs have no `program` field (Quasar does not model cross-program
///   PDAs).
#[derive(Debug, Deserialize)]
pub struct QuasarIdl {
    pub address: String,
    #[serde(default)]
    pub metadata: QuasarIdlMetadata,
    #[serde(default)]
    pub instructions: Vec<QuasarIdlInstruction>,
    #[serde(default)]
    pub accounts: Vec<QuasarIdlAccountDef>,
    #[serde(default)]
    pub events: Vec<QuasarIdlEventDef>,
    #[serde(default)]
    pub types: Vec<QuasarIdlTypeDef>,
    #[serde(default)]
    pub errors: Vec<QuasarIdlError>,
}

#[derive(Debug, Default, Deserialize)]
pub struct QuasarIdlMetadata {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub spec: String,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlInstruction {
    pub name: String,
    /// 1 byte by convention (`#[instruction(discriminator = N)]`), but
    /// modelled as a Vec to track Quasar's flexibility — the
    /// normaliser pads to ratchet's 8-byte `Discriminator`.
    pub discriminator: Vec<u8>,
    #[serde(default)]
    pub accounts: Vec<QuasarIdlAccountItem>,
    #[serde(default)]
    pub args: Vec<QuasarIdlField>,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlAccountItem {
    pub name: String,
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub signer: bool,
    #[serde(default)]
    pub pda: Option<QuasarIdlPda>,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlPda {
    pub seeds: Vec<QuasarIdlSeed>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
pub enum QuasarIdlSeed {
    #[serde(rename = "const")]
    Const { value: Vec<u8> },
    #[serde(rename = "account")]
    Account { path: String },
    #[serde(rename = "arg")]
    Arg { path: String },
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: QuasarIdlType,
}

/// Quasar's `IdlType` is an *untagged* serde union — order matters here.
/// Primitive matches first ("u64", "pubkey", "bool", …), then the
/// object-shaped variants by their unique key (`option`, `defined`,
/// `string`, `vec`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum QuasarIdlType {
    Primitive(String),
    Option { option: Box<QuasarIdlType> },
    Defined { defined: String },
    DynString { string: QuasarIdlDynString },
    DynVec { vec: QuasarIdlDynVec },
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlDynString {
    #[serde(rename = "maxLength", default)]
    pub max_length: usize,
    #[serde(rename = "prefixBytes", default)]
    pub prefix_bytes: usize,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlDynVec {
    pub items: Box<QuasarIdlType>,
    #[serde(rename = "maxLength", default)]
    pub max_length: usize,
    #[serde(rename = "prefixBytes", default)]
    pub prefix_bytes: usize,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlAccountDef {
    pub name: String,
    pub discriminator: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlEventDef {
    pub name: String,
    pub discriminator: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlTypeDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: QuasarIdlTypeDefType,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlTypeDefType {
    pub kind: QuasarIdlTypeDefKind,
    #[serde(default)]
    pub fields: Vec<QuasarIdlField>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QuasarIdlTypeDefKind {
    Struct,
}

#[derive(Debug, Deserialize)]
pub struct QuasarIdlError {
    pub code: u32,
    pub name: String,
    #[serde(default)]
    pub msg: Option<String>,
}

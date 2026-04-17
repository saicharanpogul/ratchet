//! Rust mirror of the Anchor IDL JSON schema (Anchor 0.30+).
//!
//! These types are `Deserialize`-only: they capture the on-disk / on-chain
//! shape of an Anchor IDL faithfully so a later normalizer can lower them
//! into [`ratchet_core::ProgramSurface`]. They intentionally do not try to
//! interpret squirrely bits like type references — those stay as
//! [`serde_json::Value`] and are resolved during normalization.

use serde::Deserialize;

/// A parsed Anchor IDL document.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AnchorIdl {
    /// Program address (base58), when embedded in the IDL.
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub metadata: Option<AnchorIdlMetadata>,
    #[serde(default)]
    pub instructions: Vec<AnchorIdlInstruction>,
    #[serde(default)]
    pub accounts: Vec<AnchorIdlAccountHeader>,
    #[serde(default)]
    pub types: Vec<AnchorIdlTypeDef>,
    #[serde(default)]
    pub errors: Vec<AnchorIdlError>,
    #[serde(default)]
    pub events: Vec<AnchorIdlEventHeader>,
    #[serde(default)]
    pub constants: Vec<AnchorIdlConstant>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlMetadata {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub spec: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlInstruction {
    pub name: String,
    /// Eight-byte discriminator. Optional because pre-0.30 IDLs omitted it;
    /// the normalizer computes the Anchor default when absent.
    #[serde(default)]
    pub discriminator: Option<[u8; 8]>,
    #[serde(default)]
    pub accounts: Vec<AnchorIdlAccountItem>,
    #[serde(default)]
    pub args: Vec<AnchorIdlField>,
    #[serde(default)]
    pub returns: Option<AnchorIdlType>,
}

/// An entry inside an instruction's `accounts` array. Anchor lets groups
/// of accounts be nested under a named composite (from a nested
/// `#[derive(Accounts)]` struct), so every slot is either a single account
/// or a composite.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AnchorIdlAccountItem {
    /// Composite must be tried first: it is distinguished by the required
    /// `accounts` child array.
    Composite(AnchorIdlAccountComposite),
    Single(AnchorIdlAccount),
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlAccount {
    pub name: String,
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub signer: bool,
    #[serde(default)]
    pub optional: bool,
    /// Explicit program-owned address (e.g. the System Program).
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub pda: Option<AnchorIdlPda>,
    #[serde(default)]
    pub docs: Option<Vec<String>>,
    #[serde(default)]
    pub relations: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlAccountComposite {
    pub name: String,
    pub accounts: Vec<AnchorIdlAccountItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlPda {
    pub seeds: Vec<AnchorIdlSeed>,
    #[serde(default)]
    pub program: Option<Box<AnchorIdlSeed>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AnchorIdlSeed {
    Const {
        value: Vec<u8>,
    },
    Arg {
        path: String,
    },
    Account {
        path: String,
        #[serde(default)]
        account: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: AnchorIdlType,
    #[serde(default)]
    pub docs: Option<Vec<String>>,
}

/// Anchor IDL type reference. Either a primitive string (`"u64"`, `"bool"`,
/// `"pubkey"`) or a single-key object (`{"vec": T}`, `{"option": T}`,
/// `{"array": [T, N]}`, `{"defined": {"name": "Foo"}}`, ...).
///
/// The JSON is preserved verbatim in the [`Object`](Self::Object) variant
/// and resolved by the normalizer.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AnchorIdlType {
    Primitive(String),
    Object(serde_json::Value),
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlAccountHeader {
    pub name: String,
    /// Optional: pre-0.30 IDLs omit it; normalizer defaults to
    /// `sha256("account:<Name>")[..8]`.
    #[serde(default)]
    pub discriminator: Option<[u8; 8]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlTypeDef {
    pub name: String,
    #[serde(rename = "type")]
    pub def: AnchorIdlTypeDefKind,
    #[serde(default)]
    pub docs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AnchorIdlTypeDefKind {
    Struct {
        #[serde(default)]
        fields: Option<AnchorIdlStructFields>,
    },
    Enum {
        variants: Vec<AnchorIdlEnumVariant>,
    },
    Type {
        alias: AnchorIdlType,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AnchorIdlStructFields {
    Named(Vec<AnchorIdlField>),
    Tuple(Vec<AnchorIdlType>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlEnumVariant {
    pub name: String,
    #[serde(default)]
    pub fields: Option<AnchorIdlStructFields>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlError {
    pub code: u32,
    pub name: String,
    #[serde(default)]
    pub msg: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlEventHeader {
    pub name: String,
    #[serde(default)]
    pub discriminator: Option<[u8; 8]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnchorIdlConstant {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: AnchorIdlType,
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_IDL: &str = r#"{
        "address": "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS",
        "metadata": {
            "name": "vault",
            "version": "0.1.0",
            "spec": "0.1.0"
        },
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
                    },
                    { "name": "system_program", "address": "11111111111111111111111111111111" }
                ],
                "args": [
                    { "name": "amount", "type": "u64" }
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
                        { "name": "balance", "type": "u64" }
                    ]
                }
            }
        ],
        "errors": [
            { "code": 6000, "name": "InsufficientBalance", "msg": "balance too low" }
        ]
    }"#;

    #[test]
    fn parses_anchor_030_idl() {
        let idl: AnchorIdl = serde_json::from_str(SAMPLE_IDL).unwrap();
        assert_eq!(
            idl.address.as_deref(),
            Some("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS")
        );
        assert_eq!(idl.metadata.as_ref().unwrap().name, "vault");
        assert_eq!(idl.instructions.len(), 1);
        assert_eq!(idl.accounts.len(), 1);
        assert_eq!(idl.types.len(), 1);
        assert_eq!(idl.errors.len(), 1);

        let ix = &idl.instructions[0];
        assert_eq!(ix.name, "deposit");
        assert_eq!(
            ix.discriminator,
            Some([242, 35, 198, 137, 82, 225, 242, 182])
        );
        assert_eq!(ix.accounts.len(), 3);
        assert_eq!(ix.args.len(), 1);

        match &ix.accounts[1] {
            AnchorIdlAccountItem::Single(a) => {
                assert_eq!(a.name, "vault");
                assert!(a.writable);
                assert!(!a.signer);
                let pda = a.pda.as_ref().unwrap();
                assert_eq!(pda.seeds.len(), 2);
                match &pda.seeds[0] {
                    AnchorIdlSeed::Const { value } => {
                        assert_eq!(value, &b"vault".to_vec());
                    }
                    _ => panic!("expected const seed"),
                }
            }
            _ => panic!("expected single account"),
        }
    }

    #[test]
    fn primitive_type_deserializes_from_string() {
        let t: AnchorIdlType = serde_json::from_str("\"u64\"").unwrap();
        match t {
            AnchorIdlType::Primitive(s) => assert_eq!(s, "u64"),
            _ => panic!("expected primitive"),
        }
    }

    #[test]
    fn complex_type_deserializes_as_object() {
        let t: AnchorIdlType = serde_json::from_str(r#"{"vec": "u8"}"#).unwrap();
        match t {
            AnchorIdlType::Object(v) => {
                assert_eq!(v["vec"], serde_json::json!("u8"));
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn enum_variants_parse() {
        let t: AnchorIdlTypeDef = serde_json::from_str(
            r#"{
                "name": "Side",
                "type": {
                    "kind": "enum",
                    "variants": [
                        { "name": "Bid" },
                        { "name": "Ask" }
                    ]
                }
            }"#,
        )
        .unwrap();
        match t.def {
            AnchorIdlTypeDefKind::Enum { variants } => {
                assert_eq!(variants.len(), 2);
                assert_eq!(variants[0].name, "Bid");
                assert_eq!(variants[1].name, "Ask");
            }
            _ => panic!("expected enum"),
        }
    }
}

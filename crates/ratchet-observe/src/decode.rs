//! Decode raw transactions into the shape the aggregator needs.
//!
//! Input: JSON-RPC `getTransaction` responses (keeping raw for RPC
//! portability — every provider wraps the same underlying transaction
//! format). Output: [`DecodedTx`], one per tx, with the matching ix
//! from *our* program resolved to its IDL name and the (optional)
//! error code pulled from `meta.err`.
//!
//! We only look at the top-level instruction array — inner ixs from
//! CPIs aren't attributed here. A future revision can walk
//! `meta.innerInstructions` for full CPI attribution.

use ratchet_anchor::{default_instruction_discriminator, AnchorIdl};
use serde::{Deserialize, Serialize};

/// Final per-tx row the aggregator consumes. Fields are sparse on
/// purpose — any observation that we couldn't resolve (unknown
/// discriminator, missing `meta`, malformed error enum) leaves its
/// field `None` so buckets that depend on it are simply excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedTx {
    /// Base58 signature.
    pub signature: String,
    /// Unix seconds; `None` when the RPC didn't return one.
    pub block_time: Option<i64>,
    /// IDL-resolved instruction name (e.g. `"deposit"`). `None` when
    /// no instruction in the tx belonged to our program OR when the
    /// discriminator didn't match any known ix.
    pub ix_name: Option<String>,
    /// Program custom-error code (`CustomError(n)`). `None` on success
    /// or when the error was `InstructionError` with a non-custom variant.
    pub error_code: Option<u32>,
    /// IDL-resolved error name (e.g. `"InvalidAuthority"`). `None` when
    /// the code doesn't resolve to an IDL entry.
    pub error_name: Option<String>,
    /// Fee payer pubkey (first signer of the tx).
    pub fee_payer: Option<String>,
    /// `meta.computeUnitsConsumed` when present. Used for CU
    /// percentiles — every recent Solana RPC returns it; older
    /// responses omit it.
    pub compute_units: Option<u64>,
}

/// Shape of the JSON-RPC `getTransaction` response with
/// `"jsonParsed"` encoding. We deliberately take a minimal slice of
/// the response — Solana tx JSON is huge and most fields don't matter
/// to us.
#[derive(Debug, Clone, Deserialize)]
pub struct RawTransaction {
    pub signature: String,
    #[serde(rename = "blockTime")]
    pub block_time: Option<i64>,
    pub transaction: RawTxBody,
    pub meta: Option<RawTxMeta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTxBody {
    pub message: RawMessage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawMessage {
    #[serde(rename = "accountKeys", default)]
    pub account_keys: Vec<serde_json::Value>,
    #[serde(default)]
    pub instructions: Vec<RawInstruction>,
}

/// Top-level instruction, possibly parsed (`jsonParsed` encoding) or
/// raw. We only care about the raw form — if the RPC decided to
/// pre-parse this instruction, it's not ours (we don't ship a parser
/// registry), so skip it.
#[derive(Debug, Clone, Deserialize)]
pub struct RawInstruction {
    /// Index into `accountKeys` identifying which program this ix
    /// belongs to.
    #[serde(rename = "programIdIndex")]
    pub program_id_index: Option<usize>,
    /// Base58 program id when the RPC uses `jsonParsed` + a known
    /// program — we don't parse these, just skip.
    #[serde(rename = "programId")]
    pub program_id: Option<String>,
    /// Base58 instruction data.
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTxMeta {
    pub err: Option<serde_json::Value>,
    /// Present on modern RPC responses; absent on legacy / older nodes.
    #[serde(rename = "computeUnitsConsumed")]
    pub compute_units_consumed: Option<u64>,
}

/// Decode a batch of transactions against `idl`. Unresolvable txs
/// still appear in the output so downstream totals match the sampled
/// count — their fields are `None` and buckets ignore them.
pub fn decode_all(idl: &AnchorIdl, txs: &[RawTransaction]) -> Vec<DecodedTx> {
    let discriminators = build_ix_discriminator_table(idl);
    txs.iter()
        .map(|tx| decode_one(idl, &discriminators, tx))
        .collect()
}

fn decode_one(
    idl: &AnchorIdl,
    discriminators: &[([u8; 8], String)],
    tx: &RawTransaction,
) -> DecodedTx {
    let (ix_name, fee_payer) = resolve_first_program_ix(tx, discriminators);
    let error_code = tx.meta.as_ref().and_then(extract_custom_error);
    let error_name = error_code.and_then(|code| {
        idl.errors
            .iter()
            .find(|e| e.code == code)
            .map(|e| e.name.clone())
    });
    DecodedTx {
        signature: tx.signature.clone(),
        block_time: tx.block_time,
        ix_name,
        error_code,
        error_name,
        fee_payer,
        compute_units: tx.meta.as_ref().and_then(|m| m.compute_units_consumed),
    }
}

fn build_ix_discriminator_table(idl: &AnchorIdl) -> Vec<([u8; 8], String)> {
    idl.instructions
        .iter()
        .map(|ix| {
            let disc = ix
                .discriminator
                .unwrap_or_else(|| default_instruction_discriminator(&ix.name));
            (disc, ix.name.clone())
        })
        .collect()
}

fn resolve_first_program_ix(
    tx: &RawTransaction,
    discriminators: &[([u8; 8], String)],
) -> (Option<String>, Option<String>) {
    let keys = &tx.transaction.message.account_keys;
    // Fee payer is the first account key in Solana's wire format.
    let fee_payer = keys.first().and_then(extract_pubkey);

    for ix in &tx.transaction.message.instructions {
        let Some(data) = ix.data.as_deref() else {
            continue;
        };
        let Ok(bytes) = bs58::decode(data).into_vec() else {
            continue;
        };
        if bytes.len() < 8 {
            continue;
        }
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&bytes[..8]);
        if let Some(name) = discriminators
            .iter()
            .find(|(d, _)| *d == prefix)
            .map(|(_, n)| n.clone())
        {
            return (Some(name), fee_payer);
        }
    }
    (None, fee_payer)
}

fn extract_pubkey(v: &serde_json::Value) -> Option<String> {
    // `accountKeys` is either Vec<String> (base encoding) or
    // Vec<{ pubkey, signer, writable, source }> on jsonParsed. Handle
    // both — we only need the string.
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    v.get("pubkey")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
}

/// The wire format of `meta.err` is polymorphic:
///
/// - `null` — success
/// - `{ "InstructionError": [ix_idx, { "Custom": code }] }` — most
///   Anchor / Quasar program errors bubble up this way
/// - `"InvalidAccountData"` and friends — runtime-level errors, no
///   custom code
/// - miscellaneous enum variants from the runtime
///
/// We only resolve the `Custom(code)` path — everything else returns
/// `None` and gets lumped into the "unresolved failures" bucket.
fn extract_custom_error(meta: &RawTxMeta) -> Option<u32> {
    let err = meta.err.as_ref()?;
    let ix_err = err.get("InstructionError")?;
    let tuple = ix_err.as_array()?;
    // Expected shape: [ix_index (int), detail (obj or string)].
    let detail = tuple.get(1)?;
    let custom = detail.get("Custom")?;
    custom.as_u64().map(|v| v as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratchet_anchor::AnchorIdl;
    use serde_json::json;

    fn bare_idl_with_ix(ix_name: &str, disc: [u8; 8]) -> AnchorIdl {
        serde_json::from_value(json!({
            "metadata": { "name": "t" },
            "instructions": [
                { "name": ix_name, "discriminator": disc.to_vec(), "accounts": [], "args": [] }
            ],
            "accounts": [],
            "types": [],
            "errors": [
                { "code": 6001, "name": "SomeError", "msg": "Something" }
            ]
        }))
        .unwrap()
    }

    fn mk_raw_tx(
        data_b58: &str,
        err: Option<serde_json::Value>,
        cu: Option<u64>,
    ) -> RawTransaction {
        RawTransaction {
            signature: "sig-1".into(),
            block_time: Some(1_700_000_000),
            transaction: RawTxBody {
                message: RawMessage {
                    account_keys: vec![json!("FeePayer111111111111111111111111111111111111")],
                    instructions: vec![RawInstruction {
                        program_id_index: Some(0),
                        program_id: None,
                        data: Some(data_b58.to_string()),
                    }],
                },
            },
            meta: Some(RawTxMeta {
                err,
                compute_units_consumed: cu,
            }),
        }
    }

    #[test]
    fn discriminator_table_uses_explicit_pin_when_present() {
        let idl = bare_idl_with_ix("deposit", [1, 2, 3, 4, 5, 6, 7, 8]);
        let table = build_ix_discriminator_table(&idl);
        assert_eq!(table, vec![([1, 2, 3, 4, 5, 6, 7, 8], "deposit".into())]);
    }

    #[test]
    fn custom_error_is_extracted_from_instruction_error_variant() {
        let meta = RawTxMeta {
            err: Some(json!({ "InstructionError": [0, { "Custom": 6001 }] })),
            compute_units_consumed: Some(10_000),
        };
        assert_eq!(extract_custom_error(&meta), Some(6001));
    }

    #[test]
    fn non_custom_runtime_errors_are_ignored() {
        let meta = RawTxMeta {
            err: Some(json!("InvalidAccountData")),
            compute_units_consumed: None,
        };
        assert_eq!(extract_custom_error(&meta), None);
    }

    #[test]
    fn successful_tx_has_no_error() {
        let meta = RawTxMeta {
            err: None,
            compute_units_consumed: Some(5_000),
        };
        assert_eq!(extract_custom_error(&meta), None);
    }

    #[test]
    fn decode_one_maps_discriminator_to_ix_name() {
        let disc = [11, 22, 33, 44, 55, 66, 77, 88];
        let idl = bare_idl_with_ix("deposit", disc);
        let table = build_ix_discriminator_table(&idl);
        // Ix data = discriminator + some args.
        let data = bs58::encode([&disc[..], &[0xAA, 0xBB, 0xCC]].concat()).into_string();
        let tx = mk_raw_tx(&data, None, Some(42_000));
        let decoded = decode_one(&idl, &table, &tx);
        assert_eq!(decoded.ix_name.as_deref(), Some("deposit"));
        assert_eq!(decoded.error_code, None);
        assert_eq!(decoded.compute_units, Some(42_000));
        assert_eq!(
            decoded.fee_payer.as_deref(),
            Some("FeePayer111111111111111111111111111111111111")
        );
    }

    #[test]
    fn decode_one_ignores_unknown_discriminator() {
        let idl = bare_idl_with_ix("deposit", [1, 2, 3, 4, 5, 6, 7, 8]);
        let table = build_ix_discriminator_table(&idl);
        let data = bs58::encode([99u8; 10]).into_string(); // not our discriminator
        let tx = mk_raw_tx(&data, None, None);
        let decoded = decode_one(&idl, &table, &tx);
        assert!(decoded.ix_name.is_none());
    }
}

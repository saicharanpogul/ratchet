//! Fetch an Anchor IDL account from a JSON-RPC endpoint.
//!
//! Two entry points:
//! - [`fetch_idl_account`] takes the IDL account pubkey directly. Use
//!   when the account was moved off the canonical Anchor slot, or for
//!   non-Anchor programs that piggy-back on the Anchor IDL layout.
//! - [`fetch_idl_for_program`] derives the IDL account address from
//!   just the program id via `anchor_idl_address` and then fetches.
//!   This is the path the CLI's `--program` flag uses.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::json;

use crate::decode::decode_idl_account;
use crate::idl::AnchorIdl;
use crate::pda::{anchor_idl_address, decode_pubkey, encode_pubkey};

/// Cluster shorthand. Accepts the three well-known clusters or an explicit
/// RPC URL.
#[derive(Debug, Clone)]
pub enum Cluster {
    Mainnet,
    Devnet,
    Testnet,
    Custom(String),
}

impl Cluster {
    pub fn parse(s: &str) -> Self {
        match s {
            "m" | "main" | "mainnet" | "mainnet-beta" => Cluster::Mainnet,
            "d" | "dev" | "devnet" => Cluster::Devnet,
            "t" | "test" | "testnet" => Cluster::Testnet,
            url => Cluster::Custom(url.to_string()),
        }
    }

    pub fn url(&self) -> &str {
        match self {
            Cluster::Mainnet => "https://api.mainnet-beta.solana.com",
            Cluster::Devnet => "https://api.devnet.solana.com",
            Cluster::Testnet => "https://api.testnet.solana.com",
            Cluster::Custom(u) => u.as_str(),
        }
    }
}

/// Fetch and parse the IDL stored on-chain at `idl_account_pubkey`.
///
/// Performs one `getAccountInfo` JSON-RPC call, base64-decodes the account
/// data, and hands the blob to [`decode_idl_account`].
pub fn fetch_idl_account(cluster: &Cluster, idl_account_pubkey: &str) -> Result<AnchorIdl> {
    let raw = rpc_get_account_data(cluster.url(), idl_account_pubkey)?;
    decode_idl_account(&raw)
}

/// Fetch the Anchor IDL for a program, auto-deriving the IDL account.
///
/// Equivalent to `fetch_idl_account(cluster, &anchor_idl_address(program_id))`
/// but takes a base58 program id for convenience.
pub fn fetch_idl_for_program(cluster: &Cluster, program_id_b58: &str) -> Result<AnchorIdl> {
    let program_id = decode_pubkey(program_id_b58)?;
    let idl_address = anchor_idl_address(&program_id);
    let idl_b58 = encode_pubkey(&idl_address);
    fetch_idl_account(cluster, &idl_b58)
}

/// Fetch raw account data for any Solana account via `getAccountInfo`.
/// Returns the base64-decoded data bytes.
pub fn fetch_account_data(cluster: &Cluster, pubkey: &str) -> Result<Vec<u8>> {
    rpc_get_account_data(cluster.url(), pubkey)
}

fn rpc_get_account_data(rpc_url: &str, pubkey: &str) -> Result<Vec<u8>> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [
            pubkey,
            { "encoding": "base64", "commitment": "confirmed" }
        ]
    });

    let response: serde_json::Value = ureq::post(rpc_url)
        .set("content-type", "application/json")
        .send_json(body)
        .context("calling getAccountInfo")?
        .into_json()
        .context("parsing JSON-RPC response")?;

    if let Some(err) = response.get("error") {
        bail!("RPC error: {err}");
    }

    let value = response
        .get("result")
        .and_then(|r| r.get("value"))
        .ok_or_else(|| anyhow::anyhow!("RPC response missing result.value"))?;

    if value.is_null() {
        bail!("account {pubkey} not found on {rpc_url}");
    }

    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("RPC response missing account data"))?;

    BASE64.decode(data).context("base64-decoding account data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_shorthands() {
        assert!(matches!(Cluster::parse("mainnet"), Cluster::Mainnet));
        assert!(matches!(Cluster::parse("m"), Cluster::Mainnet));
        assert!(matches!(Cluster::parse("devnet"), Cluster::Devnet));
        assert!(matches!(Cluster::parse("testnet"), Cluster::Testnet));
        match Cluster::parse("https://my-custom-rpc.example") {
            Cluster::Custom(u) => assert_eq!(u, "https://my-custom-rpc.example"),
            _ => panic!("expected custom cluster"),
        }
    }

    #[test]
    fn mainnet_url() {
        assert_eq!(
            Cluster::Mainnet.url(),
            "https://api.mainnet-beta.solana.com"
        );
    }
}

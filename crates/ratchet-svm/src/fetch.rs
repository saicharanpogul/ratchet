//! JSON-RPC helper to sample program-owned accounts.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use ratchet_anchor::Cluster;
use serde::Deserialize;
use serde_json::json;

/// A single program-owned account: pubkey plus the raw data blob.
#[derive(Debug, Clone)]
pub struct ProgramAccount {
    pub pubkey: String,
    pub data: Vec<u8>,
}

/// Fetch up to `limit` program-owned accounts via `getProgramAccounts`.
/// `data_slice_len` caps how many bytes of each account's data are
/// returned; the first 8 bytes (Anchor discriminator) are always
/// retrieved so callers can classify accounts cheaply.
pub fn fetch_program_accounts(
    cluster: &Cluster,
    program_id_b58: &str,
    limit: usize,
) -> Result<Vec<ProgramAccount>> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            program_id_b58,
            {
                "encoding": "base64",
                "commitment": "confirmed",
                "dataSlice": { "offset": 0, "length": 4096 },
                "withContext": false,
            }
        ]
    });

    let response: serde_json::Value = ureq::post(cluster.url())
        .set("content-type", "application/json")
        .send_json(body)
        .context("getProgramAccounts RPC call")?
        .into_json()
        .context("parsing getProgramAccounts response")?;

    if let Some(err) = response.get("error") {
        bail!("RPC error: {err}");
    }

    let Some(result) = response.get("result").and_then(|r| r.as_array()) else {
        bail!("missing result array from getProgramAccounts");
    };

    let mut out = Vec::with_capacity(result.len().min(limit));
    for entry in result.iter().take(limit) {
        let pubkey = entry
            .get("pubkey")
            .and_then(|p| p.as_str())
            .context("account entry missing pubkey")?
            .to_string();
        let data_arr = entry
            .get("account")
            .and_then(|a| a.get("data"))
            .and_then(|d| d.as_array())
            .context("account entry missing data array")?;
        let encoded = data_arr
            .first()
            .and_then(|v| v.as_str())
            .context("data entry missing base64 payload")?;
        let data = BASE64
            .decode(encoded)
            .context("base64 decoding account data")?;
        out.push(ProgramAccount { pubkey, data });
    }
    Ok(out)
}

/// Deserialize-only helper for tests that don't want to hit a network.
pub(crate) fn parse_program_accounts_json(raw: &str) -> Result<Vec<ProgramAccount>> {
    #[derive(Deserialize)]
    struct Rpc {
        result: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        pubkey: String,
        account: AccountData,
    }
    #[derive(Deserialize)]
    struct AccountData {
        data: (String, String),
    }

    let rpc: Rpc = serde_json::from_str(raw)?;
    rpc.result
        .into_iter()
        .map(|e| {
            Ok(ProgramAccount {
                pubkey: e.pubkey,
                data: BASE64.decode(&e.account.data.0)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_program_accounts_json_happy_path() {
        // getProgramAccounts with base64 encoding returns a (data, encoding) tuple
        // per account. Encode four bytes to `AQIDBA==`.
        let raw = r#"{
            "result": [
                {
                    "pubkey": "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS",
                    "account": {
                        "data": ["AQIDBA==", "base64"]
                    }
                }
            ]
        }"#;
        let accs = parse_program_accounts_json(raw).unwrap();
        assert_eq!(accs.len(), 1);
        assert_eq!(accs[0].pubkey, "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");
        assert_eq!(accs[0].data, vec![1, 2, 3, 4]);
    }
}

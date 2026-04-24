//! Redaction for RPC URLs that carry API keys.
//!
//! Any display path — CLI human output, error messages, HTML export,
//! log lines — runs caller-provided URLs through [`redact_rpc_url`]
//! before surfacing them, so a screenshotted terminal or a copy-
//! pasted error doesn't leak the provider key.
//!
//! Covers three provider shapes:
//!
//! - **Query-param keys** (Helius, custom RPCs) → value replaced
//!   with `***`.
//! - **Path-segment keys** (QuickNode, Alchemy) → segments ≥ 24 chars
//!   of `[A-Za-z0-9_-]` replaced with `***`.
//! - **Public endpoints** (stock mainnet/devnet/testnet) pass through
//!   unchanged.

/// Replace API-key-bearing parts of `url` with `***`. The result is
/// safe to include in a screenshot, error message, or shared log.
pub fn redact_rpc_url(url: &str) -> String {
    let (base, rest) = match url.find("://").map(|i| i + 3) {
        Some(after_scheme) => match url[after_scheme..].find('/') {
            Some(slash) => url.split_at(after_scheme + slash),
            None => return url.to_string(),
        },
        None => return url.to_string(),
    };

    let (path, query) = match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };

    let redacted_path: String = path
        .split('/')
        .map(redact_path_segment)
        .collect::<Vec<_>>()
        .join("/");

    let mut out = format!("{base}{redacted_path}");
    if let Some(q) = query {
        let redacted_q = q
            .split('&')
            .map(|kv| {
                if let Some(eq) = kv.find('=') {
                    let key = &kv[..eq];
                    let key_lower = key.to_lowercase();
                    if matches!(
                        key_lower.as_str(),
                        "api-key" | "apikey" | "api_key" | "token" | "key"
                    ) {
                        format!("{key}=***")
                    } else {
                        kv.to_string()
                    }
                } else {
                    kv.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("&");
        out = format!("{out}?{redacted_q}");
    }
    out
}

fn redact_path_segment(seg: &str) -> String {
    if seg.len() >= 24
        && seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        "***".to_string()
    } else {
        seg.to_string()
    }
}

/// Scrub URL-shaped substrings out of free-form strings (e.g. ureq
/// error messages that concatenate the URL into a sentence). Splits
/// on whitespace / quotes / brackets to find URL-looking tokens,
/// redacts each, stitches back together.
pub fn redact_error_message(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut i = 0;
    let bytes = msg.as_bytes();
    while i < bytes.len() {
        if bytes[i..].starts_with(b"http://") || bytes[i..].starts_with(b"https://") {
            let rest = &msg[i..];
            let end = rest
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']')
                })
                .unwrap_or(rest.len());
            let url = &rest[..end];
            let (url_clean, trailing) = trim_trailing_punct(url);
            out.push_str(&redact_rpc_url(url_clean));
            out.push_str(trailing);
            i += end;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn trim_trailing_punct(s: &str) -> (&str, &str) {
    let mut end = s.len();
    for (idx, c) in s.char_indices().rev() {
        if matches!(c, ':' | ',' | '.' | ';') {
            end = idx;
        } else {
            break;
        }
    }
    (&s[..end], &s[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_helius_api_key_query_param() {
        assert_eq!(
            redact_rpc_url("https://mainnet.helius-rpc.com/?api-key=abcdef1234567890"),
            "https://mainnet.helius-rpc.com/?api-key=***"
        );
    }

    #[test]
    fn masks_quicknode_path_token() {
        assert_eq!(
            redact_rpc_url(
                "https://cool-name.solana-mainnet.quiknode.pro/abcdef0123456789abcdef0123456789/"
            ),
            "https://cool-name.solana-mainnet.quiknode.pro/***/"
        );
    }

    #[test]
    fn masks_alchemy_path_key() {
        assert_eq!(
            redact_rpc_url("https://solana-mainnet.g.alchemy.com/v2/abcdefghijklmnopqrstuvwx"),
            "https://solana-mainnet.g.alchemy.com/v2/***"
        );
    }

    #[test]
    fn leaves_public_endpoints_alone() {
        assert_eq!(
            redact_rpc_url("https://api.mainnet-beta.solana.com"),
            "https://api.mainnet-beta.solana.com"
        );
    }

    #[test]
    fn preserves_short_path_segments() {
        assert_eq!(
            redact_rpc_url("https://solana-mainnet.g.alchemy.com/v2/"),
            "https://solana-mainnet.g.alchemy.com/v2/"
        );
    }

    #[test]
    fn masks_uuid_shaped_helius_keys() {
        // Shape of an actual Helius key — uuidv4 with dashes. Any
        // real secret would never appear in this test fixture.
        assert_eq!(
            redact_rpc_url(
                "https://devnet.helius-rpc.com/?api-key=00000000-0000-0000-0000-000000000000"
            ),
            "https://devnet.helius-rpc.com/?api-key=***"
        );
    }

    #[test]
    fn redacts_any_occurrence_in_a_free_text_error_message() {
        let msg = "https://devnet.helius-rpc.com/?api-key=placeholder-key-value: status code 429";
        assert_eq!(
            redact_error_message(msg),
            "https://devnet.helius-rpc.com/?api-key=***: status code 429"
        );
    }
}

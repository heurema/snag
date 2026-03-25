#![allow(dead_code)]
use std::collections::HashMap;

/// Allowlisted env var names.
const ALLOWED_ENV: &[&str] = &[
    "PATH",
    "SHELL",
    "TERM",
    "LANG",
    "HOME",
    "USER",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "CARGO_PKG_VERSION",
];

/// Collect only allowlisted env vars, with HOME replaced by dirname.
pub fn redact_env() -> HashMap<String, String> {
    let mut out = HashMap::new();
    for &key in ALLOWED_ENV {
        if let Ok(val) = std::env::var(key) {
            let val = if key == "HOME" {
                // Return dirname only (parent dir of home)
                std::path::Path::new(&val)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| val.clone())
            } else {
                val
            };
            out.insert(key.to_string(), val);
        }
    }
    out
}

/// Strip home directory prefix from paths in a backtrace string.
pub fn redact_backtrace(s: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return s.to_string();
    }
    s.lines()
        .map(|line| strip_home_prefix(line, &home))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_home_prefix(line: &str, home: &str) -> String {
    // Replace any occurrence of the home path with a relative marker
    if line.contains(home) {
        line.replace(home, "~")
    } else {
        line.to_string()
    }
}

/// Redact secrets from a message string.
/// Patterns: Bearer tokens, sk-, ghp_, gho_, AKIA, base64 blobs >40 chars, -----BEGIN
pub fn redact_message(s: &str) -> String {
    let mut result = s.to_string();

    // -----BEGIN ... (PEM headers)
    result = replace_pattern_line(&result, "-----BEGIN");

    // Bearer <token>
    result = replace_after_keyword(&result, "Bearer ", true);

    // token=<value>
    result = replace_after_keyword(&result, "token=", false);

    // sk- prefix (OpenAI keys etc)
    result = replace_prefixed_token(&result, "sk-");

    // GitHub tokens
    result = replace_prefixed_token(&result, "ghp_");
    result = replace_prefixed_token(&result, "gho_");

    // AWS access keys
    result = replace_prefixed_token(&result, "AKIA");

    // Base64 blobs >40 chars
    result = replace_base64_blobs(&result, 40);

    result
}

/// Replace the rest of a line containing the given prefix with [REDACTED].
fn replace_pattern_line(s: &str, pattern: &str) -> String {
    s.lines()
        .map(|line| {
            if line.contains(pattern) {
                "[REDACTED]".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace keyword + following token with [REDACTED].
/// If `space_sep` is true, the token is after a space (Bearer <token>).
/// If false, the token immediately follows (token=<value>).
fn replace_after_keyword(s: &str, keyword: &str, space_sep: bool) -> String {
    let mut result = String::with_capacity(s.len());
    let mut remaining = s;
    while let Some(pos) = remaining.find(keyword) {
        result.push_str(&remaining[..pos]);
        let after_keyword = &remaining[pos + keyword.len()..];
        if space_sep {
            // Skip any spaces, then take until whitespace
            let token_start = after_keyword.len() - after_keyword.trim_start().len();
            let rest = &after_keyword[token_start..];
            let token_end = rest
                .find(|c: char| c.is_whitespace())
                .unwrap_or(rest.len());
            if token_end > 0 {
                result.push_str(keyword);
                result.push_str(&" ".repeat(token_start));
                result.push_str("[REDACTED]");
                remaining = &rest[token_end..];
            } else {
                result.push_str(keyword);
                remaining = after_keyword;
            }
        } else {
            // token immediately follows
            let token_end = after_keyword
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '&' || c == ',')
                .unwrap_or(after_keyword.len());
            if token_end > 0 {
                result.push_str(keyword);
                result.push_str("[REDACTED]");
                remaining = &after_keyword[token_end..];
            } else {
                result.push_str(keyword);
                remaining = after_keyword;
            }
        }
    }
    result.push_str(remaining);
    result
}

/// Replace tokens that start with a given prefix (e.g. "sk-", "ghp_").
fn replace_prefixed_token(s: &str, prefix: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut remaining = s;
    while let Some(pos) = remaining.find(prefix) {
        // Only treat as a secret if preceded by start-of-string or non-alphanumeric
        let safe_prefix = if pos == 0 {
            true
        } else {
            let prev = remaining.as_bytes()[pos - 1];
            !prev.is_ascii_alphanumeric() && prev != b'_'
        };

        if safe_prefix {
            result.push_str(&remaining[..pos]);
            let after = &remaining[pos + prefix.len()..];
            // Consume alphanumeric + _ chars as the token body
            let token_end = after
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                .unwrap_or(after.len());
            result.push_str("[REDACTED]");
            remaining = &after[token_end..];
        } else {
            // Not a token, advance past this occurrence
            result.push_str(&remaining[..pos + prefix.len()]);
            remaining = &remaining[pos + prefix.len()..];
        }
    }
    result.push_str(remaining);
    result
}

/// Replace base64 blobs longer than `min_len` characters.
fn replace_base64_blobs(s: &str, min_len: usize) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_blob = false;
    let mut blob_start = 0;
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if is_base64_char(c) {
            if !in_blob {
                blob_start = i;
                in_blob = true;
            }
        } else {
            if in_blob {
                let blob_len = i - blob_start;
                if blob_len > min_len {
                    result.push_str("[REDACTED]");
                } else {
                    // Not long enough to be suspicious — keep
                    for j in blob_start..i {
                        result.push(chars[j]);
                    }
                }
                in_blob = false;
            }
            result.push(c);
        }
        i += 1;
    }

    if in_blob {
        let blob_len = n - blob_start;
        if blob_len > min_len {
            result.push_str("[REDACTED]");
        } else {
            for j in blob_start..n {
                result.push(chars[j]);
            }
        }
    }

    result
}

fn is_base64_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_env_only_allowlist() {
        let env = redact_env();
        for key in env.keys() {
            assert!(
                ALLOWED_ENV.contains(&key.as_str()),
                "unexpected key: {key}"
            );
        }
    }

    #[test]
    fn test_redact_backtrace_strips_home() {
        let fake_home = "/Users/testuser";
        // Temporarily can't override HOME, so test strip_home_prefix directly
        let line = "  at /Users/testuser/personal/mycel/src/foo.rs:42";
        let result = strip_home_prefix(line, fake_home);
        assert!(!result.contains("/Users/testuser"), "home not stripped: {result}");
        assert!(result.contains("src/foo.rs:42"), "path lost: {result}");
    }

    #[test]
    fn test_redact_message_bearer() {
        let out = redact_message("Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.payload rest");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("eyJhbGciOiJSUzI1NiJ9"));
    }

    #[test]
    fn test_redact_message_sk_prefix() {
        let out = redact_message("key=sk-abc123XYZ456 done");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("sk-abc123XYZ456"));
    }

    #[test]
    fn test_redact_message_ghp() {
        let out = redact_message("token ghp_1234567890abcdef end");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("ghp_"));
    }

    #[test]
    fn test_redact_message_pem() {
        let out = redact_message("-----BEGIN RSA PRIVATE KEY-----");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("RSA PRIVATE KEY"));
    }

    #[test]
    fn test_redact_message_akia() {
        let out = redact_message("aws key: AKIAIOSFODNN7EXAMPLE");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn test_redact_message_base64_long() {
        // 48-char base64 blob
        let blob = "dGhpcyBpcyBhIHZlcnkgbG9uZyBiYXNlNjQgYmxvYiBoZQ==";
        assert!(blob.len() > 40);
        let out = redact_message(&format!("data: {blob} end"));
        assert!(out.contains("[REDACTED]"), "long base64 not redacted: {out}");
    }

    #[test]
    fn test_redact_message_short_base64_kept() {
        // Short base64 (less than 40 chars) should NOT be redacted
        let short = "dGVzdA=="; // "test" in base64 — 8 chars
        let out = redact_message(&format!("value={short}"));
        assert!(out.contains(short), "short base64 should not be redacted: {out}");
    }

    #[test]
    fn test_redact_message_clean_unchanged() {
        let msg = "thread 'main' panicked at index out of bounds";
        let out = redact_message(msg);
        assert_eq!(out, msg);
    }
}

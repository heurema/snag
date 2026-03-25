use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

// ── Schema version ─────────────────────────────────────────────────────────

pub const SCHEMA_VERSION: u32 = 1;

// ── ErrorKind ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Panic,
    Handled,
    AgentFailure,
    CiFailure,
    UserReport,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::Panic => write!(f, "panic"),
            ErrorKind::Handled => write!(f, "handled"),
            ErrorKind::AgentFailure => write!(f, "agent_failure"),
            ErrorKind::CiFailure => write!(f, "ci_failure"),
            ErrorKind::UserReport => write!(f, "user_report"),
        }
    }
}

// ── Sub-structs ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductInfo {
    pub name: String,
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReporterInfo {
    pub name: String,
    pub version: String,
    /// "standalone" | "embedded" | "agent"
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvironmentInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Allowlisted env vars only
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    pub ts: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breadcrumbs: Vec<Breadcrumb>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub created_at: String,
    pub bundle_path: String,
    pub submitted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_url: Option<String>,
}

// ── ReportBundle ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportBundle {
    pub schema_version: u32,
    pub fingerprint: String,
    pub product: ProductInfo,
    pub reporter: ReporterInfo,
    pub environment: EnvironmentInfo,
    pub error: ErrorInfo,
    pub context: ContextInfo,
    pub user: UserInfo,
    pub metadata: Metadata,
}

impl ReportBundle {
    /// Compute deterministic fingerprint per spec §Fingerprint.
    pub fn fingerprint(
        product_name: &str,
        error_kind: &ErrorKind,
        error_message: &str,
        location: Option<&str>,
        version: Option<&str>,
    ) -> String {
        let norm_msg = normalize_message(error_message);
        let first_line = norm_msg.lines().next().unwrap_or("").trim();

        let loc = location.unwrap_or("unknown");
        // Keep file:line, drop column
        let loc_trimmed = if loc.rsplit_once(':').is_some() {
            // Only strip column if both remaining parts look like file:line
            // simple heuristic: if loc has exactly 2+ colons, strip last
            let parts: Vec<&str> = loc.splitn(3, ':').collect();
            if parts.len() == 3 {
                // file:line:col
                &loc[..loc.rfind(':').unwrap()]
            } else {
                loc
            }
        } else {
            loc
        };

        // major.minor only
        let ver_short = version
            .map(|v| {
                let parts: Vec<&str> = v.splitn(3, '.').collect();
                if parts.len() >= 2 {
                    format!("{}.{}", parts[0], parts[1])
                } else {
                    v.to_string()
                }
            })
            .unwrap_or_default();

        let input = format!(
            "{}\n{}\n{}\n{}\n{}",
            product_name,
            error_kind,
            first_line,
            loc_trimmed,
            ver_short
        );

        let hash = Sha256::digest(input.as_bytes());
        let hex = format!("{:x}", hash);
        hex[..12].to_string()
    }

    /// Render the GitHub issue body per spec §Issue Body Format.
    pub fn to_issue_body(&self) -> String {
        let fp = &self.fingerprint;
        let title = self
            .user
            .title
            .as_deref()
            .unwrap_or_else(|| self.error.message.lines().next().unwrap_or(""));

        // Environment table
        let product_cell = {
            let ver = self.product.version.as_deref().unwrap_or("");
            let commit = self
                .product
                .commit
                .as_deref()
                .map(|c| format!(" ({})", c))
                .unwrap_or_default();
            format!("{} {}{}", self.product.name, ver, commit)
        };
        let os_cell = {
            let os = self.environment.os.as_deref().unwrap_or("");
            let arch = self.environment.arch.as_deref().unwrap_or("");
            format!("{} {}", os, arch).trim().to_string()
        };
        let reporter_cell = format!(
            "{} {} ({})",
            self.reporter.name, self.reporter.version, self.reporter.mode
        );

        // Error block
        let error_block = &self.error.message;

        // Backtrace details
        let backtrace_section = match &self.error.backtrace {
            Some(bt) if !bt.is_empty() => format!(
                "\n<details>\n<summary>Backtrace</summary>\n\n```\n{}\n```\n\n</details>\n",
                bt
            ),
            _ => String::new(),
        };

        // Context
        let branch_line = match (&self.context.git_branch, &self.context.git_commit) {
            (Some(b), Some(c)) => format!("Branch: {} ({})", b, c),
            (Some(b), None) => format!("Branch: {}", b),
            _ => String::new(),
        };
        let command_line = self
            .context
            .command
            .as_deref()
            .map(|c| format!("Command: `{}`", c))
            .unwrap_or_default();

        let context_lines: Vec<&str> = [&branch_line, &command_line]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
            .collect();

        let breadcrumbs_section = if !self.context.breadcrumbs.is_empty() {
            let items: Vec<String> = self
                .context
                .breadcrumbs
                .iter()
                .map(|b| {
                    // Extract HH:MM:SS from ISO timestamp
                    let time = b.ts.split('T').nth(1).unwrap_or(&b.ts);
                    let time = time.trim_end_matches('Z').split('.').next().unwrap_or(time);
                    format!("- {} {}", time, b.action)
                })
                .collect();
            format!(
                "\n<details>\n<summary>Breadcrumbs</summary>\n\n{}\n\n</details>\n",
                items.join("\n")
            )
        } else {
            String::new()
        };

        let reproduction = self
            .user
            .body
            .as_deref()
            .unwrap_or("(not provided)");

        let mut body = format!(
            "<!-- snag:fp:{fp} -->\n<!-- snag:schema:{schema} -->\n\n## Summary\n\n{title}\n\n## Environment\n\n| | |\n|---|---|\n| Product | {product_cell} |\n| OS | {os_cell} |\n| Reporter | {reporter_cell} |\n\n## Error\n\n```\n{error_block}\n```\n",
            fp = fp,
            schema = SCHEMA_VERSION,
            title = title,
            product_cell = product_cell,
            os_cell = os_cell,
            reporter_cell = reporter_cell,
            error_block = error_block,
        );

        body.push_str(&backtrace_section);

        if !context_lines.is_empty() {
            body.push_str("\n## Context\n\n");
            body.push_str(&context_lines.join("\n"));
            body.push('\n');
        }
        body.push_str(&breadcrumbs_section);

        body.push_str(&format!(
            "\n## Reproduction\n\n{}\n",
            reproduction
        ));

        body
    }

    /// Save bundle to disk as JSON with 0600 permissions.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {e}"))?;
        std::fs::write(path, &json).map_err(|e| format!("Write error: {e}"))?;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("chmod error: {e}"))?;
        Ok(())
    }

    /// Load bundle from disk.
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Read error: {e}"))?;
        serde_json::from_str(&content).map_err(|e| format!("Parse error: {e}"))
    }
}

/// Normalize error message for fingerprinting.
/// - lowercase
/// - replace hex addresses (0x[0-9a-f]+) with 0xXXXX
/// - replace UUIDs with UUID
/// - strip trailing whitespace per line
pub fn normalize_message(msg: &str) -> String {
    let lower = msg.to_lowercase();
    // Replace UUIDs: 8-4-4-4-12 hex groups
    let after_uuid = replace_pattern_uuid(&lower);
    // Replace 0x[hex]
    let after_hex = replace_hex_addresses(&after_uuid);
    // Strip trailing whitespace per line
    after_hex
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_hex_addresses(s: &str) -> String {
    // Replace 0x followed by one or more hex digits
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'0' && bytes[i + 1] == b'x' {
            // Check if followed by hex digits
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j > i + 2 {
                result.push_str("0xXXXX");
                i = j;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

fn replace_pattern_uuid(s: &str) -> String {
    // UUID pattern: 8hex-4hex-4hex-4hex-12hex
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        // Try to match UUID starting at i
        if let Some(end) = match_uuid(&chars, i) {
            result.push_str("UUID");
            i = end;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn match_uuid(chars: &[char], start: usize) -> Option<usize> {
    // 8-4-4-4-12 hex with dashes
    let groups = [8, 4, 4, 4, 12];
    let mut i = start;
    for (gi, &g) in groups.iter().enumerate() {
        let mut count = 0;
        while count < g {
            if i >= chars.len() {
                return None;
            }
            if !chars[i].is_ascii_hexdigit() {
                return None;
            }
            count += 1;
            i += 1;
        }
        if gi < groups.len() - 1 {
            if i >= chars.len() || chars[i] != '-' {
                return None;
            }
            i += 1;
        }
    }
    Some(i)
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Current UTC timestamp in ISO 8601.
pub fn now_utc() -> String {
    // Use std only — no chrono dependency
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix_to_iso8601(secs)
}

fn unix_to_iso8601(secs: u64) -> String {
    // Days since epoch
    let s = secs;
    let sec_of_day = (s % 86400) as u32;
    let days = (s / 86400) as u32;

    let h = sec_of_day / 3600;
    let m = (sec_of_day % 3600) / 60;
    let sec = sec_of_day % 60;

    let (y, mo, d) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, sec)
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_year(y: u32) -> u32 {
    if is_leap(y) { 366 } else { 365 }
}

fn days_to_ymd(mut days: u32) -> (u32, u32, u32) {
    let mut y = 1970u32;
    loop {
        let dy = days_in_year(y);
        if days < dy {
            break;
        }
        days -= dy;
        y += 1;
    }
    let months = if is_leap(y) {
        [31u32, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 1u32;
    for &dm in &months {
        if days < dm {
            break;
        }
        days -= dm;
        mo += 1;
    }
    (y, mo, days + 1)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_message_lowercase() {
        let out = normalize_message("THREAD PANICKED at 'IndexError'");
        assert_eq!(out, "thread panicked at 'indexerror'");
    }

    #[test]
    fn test_normalize_message_hex() {
        let out = normalize_message("address 0xdeadbeef in stack");
        assert_eq!(out, "address 0xXXXX in stack");
    }

    #[test]
    fn test_normalize_message_multiple_hex() {
        let out = normalize_message("ptr 0x1a2b at 0xFF00");
        assert_eq!(out, "ptr 0xXXXX at 0xXXXX");
    }

    #[test]
    fn test_normalize_message_uuid() {
        let out = normalize_message("session 550e8400-e29b-41d4-a716-446655440000 failed");
        assert_eq!(out, "session UUID failed");
    }

    #[test]
    fn test_normalize_message_trailing_whitespace() {
        let out = normalize_message("error   \nfoo  ");
        assert_eq!(out, "error\nfoo");
    }

    #[test]
    fn test_fingerprint_deterministic() {
        let fp1 = ReportBundle::fingerprint(
            "mycel",
            &ErrorKind::Panic,
            "index out of bounds",
            Some("src/foo.rs:42"),
            Some("0.3.2"),
        );
        let fp2 = ReportBundle::fingerprint(
            "mycel",
            &ErrorKind::Panic,
            "index out of bounds",
            Some("src/foo.rs:42"),
            Some("0.3.2"),
        );
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 12);
    }

    #[test]
    fn test_fingerprint_patch_invariant() {
        // Different patch versions → same fingerprint
        let fp1 = ReportBundle::fingerprint(
            "mycel",
            &ErrorKind::Panic,
            "index out of bounds",
            Some("src/foo.rs:42"),
            Some("0.3.2"),
        );
        let fp2 = ReportBundle::fingerprint(
            "mycel",
            &ErrorKind::Panic,
            "index out of bounds",
            Some("src/foo.rs:42"),
            Some("0.3.9"),
        );
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_fingerprint_different_product() {
        let fp1 = ReportBundle::fingerprint(
            "mycel",
            &ErrorKind::Panic,
            "error",
            None,
            None,
        );
        let fp2 = ReportBundle::fingerprint(
            "signum",
            &ErrorKind::Panic,
            "error",
            None,
            None,
        );
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_fingerprint_12_chars() {
        let fp = ReportBundle::fingerprint("x", &ErrorKind::UserReport, "msg", None, None);
        assert_eq!(fp.len(), 12);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_to_issue_body_contains_fp() {
        let bundle = make_test_bundle();
        let body = bundle.to_issue_body();
        assert!(body.contains(&format!("<!-- snag:fp:{} -->", bundle.fingerprint)));
        assert!(body.contains("<!-- snag:schema:1 -->"));
    }

    #[test]
    fn test_to_issue_body_sections() {
        let bundle = make_test_bundle();
        let body = bundle.to_issue_body();
        assert!(body.contains("## Summary"));
        assert!(body.contains("## Environment"));
        assert!(body.contains("## Error"));
        assert!(body.contains("## Reproduction"));
    }

    #[test]
    fn test_save_load_roundtrip() {
        let bundle = make_test_bundle();
        let dir = std::env::temp_dir();
        let path = dir.join("snag-test-bundle-roundtrip.json");
        bundle.save(&path).unwrap();

        // Check permissions
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        let loaded = ReportBundle::load(&path).unwrap();
        assert_eq!(loaded.fingerprint, bundle.fingerprint);
        assert_eq!(loaded.product.name, bundle.product.name);

        let _ = std::fs::remove_file(&path);
    }

    fn make_test_bundle() -> ReportBundle {
        let fp = ReportBundle::fingerprint(
            "mycel",
            &ErrorKind::Panic,
            "index out of bounds",
            Some("src/foo.rs:42"),
            Some("0.3.2"),
        );
        ReportBundle {
            schema_version: SCHEMA_VERSION,
            fingerprint: fp,
            product: ProductInfo {
                name: "mycel".to_string(),
                repo: "heurema/mycel".to_string(),
                version: Some("0.3.2".to_string()),
                commit: Some("abc123d".to_string()),
            },
            reporter: ReporterInfo {
                name: "snag".to_string(),
                version: "0.1.0".to_string(),
                mode: "standalone".to_string(),
            },
            environment: EnvironmentInfo {
                os: Some("Darwin 25.3.0".to_string()),
                arch: Some("aarch64".to_string()),
                shell: Some("zsh".to_string()),
                env: Default::default(),
            },
            error: ErrorInfo {
                kind: ErrorKind::Panic,
                message: "index out of bounds".to_string(),
                location: Some("src/foo.rs:42".to_string()),
                backtrace: None,
                exit_code: None,
            },
            context: ContextInfo {
                git_branch: Some("main".to_string()),
                git_commit: Some("def456".to_string()),
                command: Some("mycel send alice hello".to_string()),
                breadcrumbs: vec![],
            },
            user: UserInfo {
                title: None,
                body: Some("Steps to reproduce...".to_string()),
                labels: vec!["bug".to_string()],
            },
            metadata: Metadata {
                created_at: "2026-03-25T15:00:03Z".to_string(),
                bundle_path: "/tmp/snag-test.json".to_string(),
                submitted: false,
                issue_url: None,
            },
        }
    }
}

# ReportBundle v1 Specification

Status: DRAFT
Date: 2026-03-25

## Overview

ReportBundle is the universal exchange format between crash capture, user reporting,
and issue submission. All paths (panic hook, `<product> report`, `snag file`, agent wrapper)
produce the same bundle. The bundle is a JSON file saved to disk first, submitted second.

## Principles

1. **Save first, submit later.** No network calls during crash capture.
2. **Redact before write.** Secrets never touch disk unredacted.
3. **Explicit identity.** Embedded callers pass ProductId; standalone uses heuristics.
4. **Versioned schema.** Bundle includes schema_version; consumers must handle unknown fields gracefully.
5. **No auto-submit.** Default = save + print instructions. Agent/CI must confirm before submit.

## Schema

```json
{
  "schema_version": 1,
  "fingerprint": "<deterministic SHA-256, see §Fingerprint>",

  "product": {
    "name": "mycel",
    "repo": "heurema/mycel",
    "version": "0.3.2",
    "commit": "abc123d"
  },

  "reporter": {
    "name": "snag",
    "version": "0.1.0",
    "mode": "standalone | embedded | agent"
  },

  "environment": {
    "os": "Darwin 25.3.0",
    "arch": "aarch64",
    "shell": "zsh"
  },

  "error": {
    "kind": "panic | handled | agent_failure | ci_failure | user_report",
    "message": "thread 'main' panicked at 'index out of bounds'",
    "location": "src/transport.rs:42",
    "backtrace": "<redacted backtrace, see §Redaction>",
    "exit_code": 101
  },

  "context": {
    "git_branch": "main",
    "git_commit": "def456",
    "command": "mycel send alice hello",
    "breadcrumbs": [
      {"ts": "2026-03-25T15:00:01Z", "action": "config loaded"},
      {"ts": "2026-03-25T15:00:02Z", "action": "relay connect wss://relay.mycel.run"}
    ]
  },

  "user": {
    "title": "<user-provided or auto-generated>",
    "body": "<user-provided description>",
    "labels": ["bug"]
  },

  "metadata": {
    "created_at": "2026-03-25T15:00:03Z",
    "bundle_path": "/tmp/snag-bundle-42.json",
    "submitted": false,
    "issue_url": null
  }
}
```

### Required Fields

| Field | Required by | Notes |
|-------|-------------|-------|
| schema_version | all | Always 1 |
| fingerprint | all | Computed after all other fields |
| product.name | all | Explicit in embedded, detected in standalone |
| product.repo | all | `<org>/<name>` |
| reporter.mode | all | How the bundle was created |
| error.kind | all | Enum, see below |
| error.message | all | Short, first line only |
| metadata.created_at | all | ISO 8601 UTC |

### Optional Fields

Everything else. Missing fields = null/empty, not schema violation.

### Error Kinds

| Kind | Source | Typical producer |
|------|--------|-----------------|
| `panic` | Rust panic hook | Embedded library |
| `handled` | Caught error the user wants to report | `<product> report` subcommand |
| `agent_failure` | AI agent encountered product bug | snag via SKILL.md |
| `ci_failure` | CI pipeline failure | snag in CI mode |
| `user_report` | Manual report, no crash | `snag file` interactive |

## Fingerprint

Deterministic hash for deduplication. Computed from stable fields only (not timestamps, not user text).

```
fingerprint = SHA-256(
  product.name + "\n" +
  error.kind + "\n" +
  error.message (first line, normalized: lowercase, strip numbers/hashes) + "\n" +
  error.location (file:line, or "unknown") + "\n" +
  product.version (major.minor only, not patch)
)[:12]
```

### Normalization Rules

- `error.message`: lowercase, replace hex addresses (`0x[0-9a-f]+`) with `0xXXXX`,
  replace UUIDs with `UUID`, strip trailing whitespace
- `error.location`: keep `file:line`, drop column
- `product.version`: `1.2.3` → `1.2` (patch changes don't change fingerprint)

### Usage in GitHub Issues

Fingerprint is embedded as HTML comment in issue body:

```markdown
<!-- snag:fp:a1b2c3d4e5f6 -->
<!-- snag:schema:1 -->
```

Dedup check: `gh search issues "snag:fp:a1b2c3d4e5f6" --repo <repo> --state open`

Falls back to title Jaccard similarity if no fingerprint match found (for manually-created issues).

## Detection Modes

### Embedded (in Rust CLIs)

Product identity is known at compile time. No heuristics.

```rust
// In mycel's main.rs
snag_lib::init(ProductId {
    name: "mycel",
    repo: "heurema/mycel",
    version: env!("CARGO_PKG_VERSION"),
    commit: option_env!("GIT_HASH"),
});
```

The library:
1. Registers panic hook (save-only, no network)
2. Provides `report` subcommand integration
3. Collects breadcrumbs via `snag_lib::breadcrumb("action")`

### Standalone (snag CLI)

Product identity detected via heuristic chain:

| Priority | Signal | Confidence |
|----------|--------|------------|
| 1 | `--product X` flag | 100% |
| 2 | git remote URL | 95% |
| 3 | manifest file name | 85% |
| 4 | CWD path segment | 60% |
| 5 | no match | refuse |

### Agent/Plugin Wrapper

Wrappers MUST pass explicit `--product` and `--version`. Never rely on CWD detection
for plugins — the plugin runs in the user's repo, not the plugin's repo.

```
snag file --product signum --version 4.16.1 --title "..." --body "..."
```

## Panic Hook Contract

The panic hook MUST:
- Be synchronous, no async
- Do zero network calls
- Do zero `gh` / `git` subprocess calls
- Allocate minimally (pre-allocate buffer at init)
- Apply redaction before writing
- Write bundle to `$TMPDIR/snag-bundle-<pid>.json` with mode 0600
- Print to stderr:

```
snag: crash report saved to /tmp/snag-bundle-42.json
snag: run `snag submit /tmp/snag-bundle-42.json` to review and file
```

The panic hook MUST NOT:
- Call `std::panic::set_hook` unconditionally (chain with existing hook)
- Auto-submit anything
- Capture full env vars (allowlist only)
- Block on I/O beyond the temp file write

### Hook Chaining

```rust
let prev = std::panic::take_hook();
std::panic::set_hook(Box::new(move |info| {
    // Our handler first (save bundle)
    save_crash_bundle(info);
    // Then previous handler (human-panic, color-eyre, etc.)
    prev(info);
}));
```

## Redaction Pipeline

Applied before any write to disk.

### Env Var Allowlist

Only these env vars are captured:

```
PATH, SHELL, TERM, LANG, HOME (dirname only), USER,
RUST_BACKTRACE, RUST_LOG, CARGO_PKG_VERSION
```

Everything else is stripped. No blocklist approach — allowlist only.

### Backtrace Redaction

- Strip absolute paths: `/Users/vitaly/personal/mycel/src/foo.rs` → `src/foo.rs`
- Strip home directory prefix globally
- Preserve crate-relative paths

### Message Redaction

- Scan for patterns: `Bearer `, `token=`, `sk-`, `ghp_`, `gho_`, `AKIA`,
  base64 blobs >40 chars, `-----BEGIN`
- Replace matches with `[REDACTED]`

### Body Redaction (user-provided)

User-provided text is NOT redacted (they chose to write it). But auto-collected
context (breadcrumbs, env, backtrace) always goes through redaction.

## Submit Flow

```
snag submit <bundle-path>
```

1. Read bundle from disk
2. Display preview (title, body, fingerprint, product, env)
3. Prompt: `Submit to heurema/mycel? [y/N/edit]`
   - `y` → proceed
   - `N` → abort, bundle stays on disk
   - `edit` → open $EDITOR with body, reload
4. Fingerprint dedup check: `gh search issues "snag:fp:<hash>" --repo <repo>`
   - Match found → show existing issue URL, ask "Still file? [y/N]"
5. Title dedup check (fallback): Jaccard > 0.8
6. Create issue: `gh issue create -R <repo> --title <title> --body-file <tmp>`
7. Update bundle: `submitted: true, issue_url: <url>`
8. Print issue URL

### Session Limits

- Max 5 issues per `snag submit` session without `--force`
- Enforced via counter file: `$TMPDIR/snag-session-<date>.count`

## Issue Body Format

```markdown
<!-- snag:fp:a1b2c3d4e5f6 -->
<!-- snag:schema:1 -->

## Summary

<user.title or error.message first line>

## Environment

| | |
|---|---|
| Product | mycel 0.3.2 (abc123d) |
| OS | Darwin 25.3.0 aarch64 |
| Reporter | snag 0.1.0 (standalone) |

## Error

```
<error.message>
```

<details>
<summary>Backtrace</summary>

```
<redacted backtrace>
```

</details>

## Context

Branch: main (def456)
Command: `mycel send alice hello`

<details>
<summary>Breadcrumbs</summary>

- 15:00:01 config loaded
- 15:00:02 relay connect wss://relay.mycel.run

</details>

## Reproduction

<user.body or "(not provided)">
```

## Library Crate API (sketch)

```rust
// heurema-report (or snag-lib)
pub struct ProductId {
    pub name: &'static str,
    pub repo: &'static str,
    pub version: &'static str,
    pub commit: Option<&'static str>,
}

/// Initialize: register panic hook, set product identity
pub fn init(product: ProductId);

/// Add breadcrumb (ring buffer, max 50)
pub fn breadcrumb(action: &str);

/// Create bundle from handled error (not panic)
pub fn report_error(err: &dyn std::error::Error) -> ReportBundle;

/// Create bundle interactively (user-initiated report)
pub fn report_interactive() -> ReportBundle;

/// Submit bundle to GitHub (with confirmation)
pub fn submit(bundle: &ReportBundle, confirm: bool) -> Result<String, SubmitError>;

/// Detect product from environment (standalone mode)
pub fn detect(config: &Config) -> Option<Detection>;
```

## Version Compatibility

| Schema | snag CLI | Library | Notes |
|--------|----------|---------|-------|
| v1 | 0.1.x | 0.1.x | Initial release |

Rules:
- snag CLI MUST accept bundles with schema_version <= its own
- snag CLI MUST reject bundles with schema_version > its own (suggest upgrade)
- Unknown fields MUST be preserved (forward compat), not stripped
- schema_version bump = breaking change in required fields or fingerprint algorithm

## Crate Split (future)

For now, single crate. If it grows beyond ~1000 LOC:

```
snag-core     — bundle, schema, redaction, fingerprint (no I/O)
snag-detect   — product detection (filesystem + git)
snag-gh       — gh CLI integration (submit, dedup)
snag          — CLI binary
```

```
   _________  ____ _____ _
  / ___/ __ \/ __ `/ __ `/
 (__  ) / / / /_/ / /_/ /
/____/_/ /_/\__,_/\__, /
                 /____/
```

**Hit a snag? File it.**

[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> Auto-detect which project you're in, check for duplicates, file a GitHub issue on the right repo. Works with any org via config. Integrates with AI agents via thin SKILL.md.

---

## Install

```bash
cargo install --git https://github.com/heurema/snag.git --locked
```

## Quick Start

```bash
# Create config for your org
snag init
# Edit ~/.config/snag/config.toml — set org and products

# Detect which product you're working in
snag detect
# → signum (confidence: 95%, signal: git-remote, repo: heurema/signum)

# Check if a similar issue already exists
snag check "panic in JSON parser"

# File a bug report (interactive)
snag file --title "gemini output not parsed"

# File from an AI agent (non-interactive)
snag file --auto --product signum --title "fence stripping missing"
```

## How It Works

### Detection Chain

| Priority | Signal | Confidence | Method |
|----------|--------|------------|--------|
| 1 | `--product` flag | 100% | Explicit override |
| 2 | Git remote URL | 95% | `git remote get-url origin` → match against config |
| 3 | Manifest file | 85% | Walk up from CWD, match `Cargo.toml`/`plugin.json` names |
| 4 | CWD path | 60% | Directory name matches product name |

No match = no filing. Wrong repo is worse than no report.

### Dedup Check

Before creating an issue, `snag` searches for similar open issues:

```bash
gh search issues "keywords" --repo org/product --match title --state open
```

If Jaccard similarity > 80% with an existing title → blocks creation (use `--force` to override).

### Config

```toml
# ~/.config/snag/config.toml

[org]
name = "myorg"
github = "myorg"

[[products]]
name = "myproject"
repo = "myproject"
markers = ["Cargo.toml:myproject", ".myproject/"]

[settings]
max_issues_per_session = 5
```

**Markers** tell snag how to detect a product from the filesystem:
- `"Cargo.toml:name"` — file contains string "name"
- `".signum/"` — directory exists
- `"plugin.json"` — file exists

## AI Agent Integration

snag works with any AI agent that can run shell commands. Drop a SKILL.md into your agent's skill directory:

```yaml
---
name: snag
description: |
  Auto-detect and report bugs. Use when: a CLI tool crashes,
  a workflow fails, or you encounter unexpected behavior.
  Runs `snag` CLI binary.
---
```

Works with: Claude Code, Codex CLI, Gemini CLI, Cursor, and [30+ other agents](https://agentskills.io).

## Commands

| Command | Description |
|---------|-------------|
| `snag detect` | Show detected product + confidence |
| `snag check "title"` | Search for similar open issues |
| `snag file` | Interactive: detect → check → create issue |
| `snag file --auto --title "..."` | Non-interactive: for agent use |
| `snag init` | Create config template |

## Requirements

- `gh` CLI (authenticated)
- `git`

## License

[MIT](LICENSE)

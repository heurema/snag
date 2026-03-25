---
name: snag
description: |
  Auto-detect and report bugs in configured products.
  Use when: a CLI tool crashes, a plugin workflow fails, or you encounter
  unexpected behavior in a product from your org. Runs `snag` CLI binary.
  Checks for duplicate issues before filing. Requires `snag` in PATH and
  `~/.config/snag/config.toml` configured.
allowed-tools: Bash(snag *)
---

## Commands

```bash
# Detect which product you're working in
snag detect

# Check if similar issue already exists
snag check "issue title or keywords"

# File a bug (non-interactive, for agent use)
snag file --auto --title "concise bug title" --body "reproduction steps, expected vs actual, error output"

# File with explicit product (skip auto-detect)
snag file --auto --product myproject --title "title" --body "body"

# File interactively (asks for confirmation)
snag file --title "title"
```

## When to Use

- A CLI tool from the configured org crashes or returns an unexpected error
- A plugin workflow produces wrong results
- You discover a bug during development

## Rules

- Always ask the user for confirmation before running `snag file`
- Use `snag detect` first to verify the right product is detected
- Use `snag check "title"` to avoid creating duplicates
- Include reproduction steps, expected vs actual behavior, and error output in the body

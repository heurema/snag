use crate::{bundle::ReportBundle, check};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

/// snag submit <bundle-path> [--force]
pub fn run(bundle_path: &str, force: bool, _config_path: Option<&str>) -> i32 {
    let path = Path::new(bundle_path);

    // 1. Load bundle
    let mut bundle = match ReportBundle::load(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Cannot load bundle: {e}");
            return 1;
        }
    };

    // Schema version check
    if bundle.schema_version > crate::bundle::SCHEMA_VERSION {
        eprintln!(
            "Bundle schema version {} > supported {}. Please upgrade snag.",
            bundle.schema_version,
            crate::bundle::SCHEMA_VERSION
        );
        return 1;
    }

    if bundle.metadata.submitted {
        if let Some(url) = &bundle.metadata.issue_url {
            eprintln!("Bundle already submitted: {url}");
        } else {
            eprintln!("Bundle already submitted.");
        }
        return 0;
    }

    // 2. Session rate-limit (max 5 without --force)
    if !force {
        let count = session_count();
        if count >= 5 {
            eprintln!(
                "Session limit reached ({count}/5). Use --force to override."
            );
            return 1;
        }
    }

    // 3. Display preview
    display_preview(&bundle);

    // 4. Confirm (unless --force)
    if !force {
        eprint!("\nSubmit to {}? [y/N] ", bundle.product.repo);
        io::stdout().flush().ok();
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).unwrap_or(0);
        match answer.trim().to_lowercase().as_str() {
            "y" | "yes" => {}
            _ => {
                eprintln!("Cancelled. Bundle saved at: {bundle_path}");
                return 0;
            }
        }
    }

    let repo = &bundle.product.repo.clone();
    let fp = bundle.fingerprint.clone();

    // 5. Fingerprint dedup check
    if let Some(existing) = check::find_by_fingerprint(repo, &fp) {
        eprintln!(
            "Duplicate found: #{} — {}",
            existing.number, existing.title
        );
        eprintln!("  https://github.com/{repo}/issues/{}", existing.number);
        if !force {
            eprint!("Still file? [y/N] ");
            io::stdout().flush().ok();
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).unwrap_or(0);
            if !answer.trim().eq_ignore_ascii_case("y") {
                eprintln!("Cancelled.");
                return 0;
            }
        }
    } else {
        // 6. Title Jaccard fallback
        let title = bundle
            .user
            .title
            .as_deref()
            .unwrap_or_else(|| bundle.error.message.lines().next().unwrap_or(""))
            .to_string();
        if check::has_duplicate(repo, &title) {
            eprintln!("Similar issue found in {repo}. Use --force to file anyway.");
            if !force {
                return 2;
            }
        }
    }

    // 7. Create issue
    let title = bundle
        .user
        .title
        .clone()
        .unwrap_or_else(|| {
            bundle
                .error
                .message
                .lines()
                .next()
                .unwrap_or("Bug report")
                .to_string()
        });
    let body = bundle.to_issue_body();
    let labels = bundle.user.labels.clone();

    match create_issue(repo, &title, &body, &labels) {
        Ok(url) => {
            // 8. Update bundle on disk
            bundle.metadata.submitted = true;
            bundle.metadata.issue_url = Some(url.clone());
            if let Err(e) = bundle.save(path) {
                eprintln!("Warning: could not update bundle on disk: {e}");
            }

            // Increment session counter
            increment_session_count();

            println!("{url}");
            0
        }
        Err(e) => {
            eprintln!("Failed to create issue: {e}");
            1
        }
    }
}

fn display_preview(bundle: &ReportBundle) {
    let title = bundle
        .user
        .title
        .as_deref()
        .unwrap_or_else(|| bundle.error.message.lines().next().unwrap_or(""));
    eprintln!("\n--- Bundle Preview ---");
    eprintln!("Product:     {} ({})", bundle.product.name, bundle.product.repo);
    if let Some(ver) = &bundle.product.version {
        eprintln!("Version:     {ver}");
    }
    eprintln!(
        "Error:       [{:?}] {}",
        bundle.error.kind, bundle.error.message
    );
    eprintln!("Fingerprint: {}", bundle.fingerprint);
    eprintln!("Title:       {title}");
    if let Some(os) = &bundle.environment.os {
        eprintln!("OS:          {os}");
    }
    eprintln!("--- End Preview ---");
}

fn create_issue(repo: &str, title: &str, body: &str, labels: &[String]) -> Result<String, String> {
    let tmp = std::env::temp_dir().join(format!("snag-submit-body-{}.md", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| format!("Cannot write temp file: {e}"))?;

    let mut args = vec![
        "issue".to_string(),
        "create".to_string(),
        "-R".to_string(),
        repo.to_string(),
        "--title".to_string(),
        title.to_string(),
        "--body-file".to_string(),
        tmp.to_string_lossy().to_string(),
    ];
    for label in labels {
        args.push("--label".to_string());
        args.push(label.clone());
    }

    let output = Command::new("gh")
        .args(&args)
        .output()
        .map_err(|e| format!("gh not found: {e}"))?;

    let _ = std::fs::remove_file(&tmp);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Retry without labels if label doesn't exist
        if stderr.contains("label") && !labels.is_empty() {
            let tmp2 =
                std::env::temp_dir().join(format!("snag-submit-body-retry-{}.md", std::process::id()));
            std::fs::write(&tmp2, body).map_err(|e| format!("Cannot write temp file: {e}"))?;
            let output2 = Command::new("gh")
                .args([
                    "issue",
                    "create",
                    "-R",
                    repo,
                    "--title",
                    title,
                    "--body-file",
                    &tmp2.to_string_lossy(),
                ])
                .output()
                .map_err(|e| format!("gh retry failed: {e}"))?;
            let _ = std::fs::remove_file(&tmp2);
            if !output2.status.success() {
                return Err(String::from_utf8_lossy(&output2.stderr).to_string());
            }
            return Ok(String::from_utf8_lossy(&output2.stdout).trim().to_string());
        }
        return Err(stderr.to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ── Session counter ────────────────────────────────────────────────────────

fn session_file() -> std::path::PathBuf {
    use crate::bundle::now_utc;
    let date = &now_utc()[..10]; // YYYY-MM-DD
    std::env::temp_dir().join(format!("snag-session-{date}.count"))
}

fn session_count() -> u32 {
    let path = session_file();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn increment_session_count() {
    let path = session_file();
    let count = session_count() + 1;
    let _ = std::fs::write(&path, count.to_string());
}

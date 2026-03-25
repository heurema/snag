use crate::{check, detect, registry};
use std::io::{self, Read};
use std::process::Command;

pub fn run(
    auto: bool,
    product: Option<&str>,
    title: Option<&str>,
    body: Option<&str>,
    no_check: bool,
    force: bool,
    config_path: Option<&str>,
) -> i32 {
    let config = match registry::load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    // Resolve product → repo
    let (product_name, repo) = match resolve_product(&config, product) {
        Some(r) => r,
        None => {
            eprintln!("Cannot determine target repo. Use --product or cd into the project.");
            return 1;
        }
    };

    // Resolve title
    let title = match title {
        Some(t) => t.to_string(),
        None if auto => {
            eprintln!("--auto requires --title (auto-title from agent context not yet implemented)");
            return 1;
        }
        None => {
            eprint!("Issue title: ");
            let mut t = String::new();
            io::stdin().read_line(&mut t).unwrap_or(0);
            let t = t.trim().to_string();
            if t.is_empty() {
                eprintln!("Title cannot be empty.");
                return 1;
            }
            t
        }
    };

    // Dedup check
    if !no_check && !force {
        if check::has_duplicate(&repo, &title) {
            eprintln!("Similar issue already exists in {repo}. Use --force to file anyway.");
            // Show the duplicates
            let _ = check::run(&title, Some(&product_name), config_path);
            return 2;
        }
    }

    // Resolve body
    let body = match body {
        Some(b) => b.to_string(),
        None if auto => collect_auto_context(),
        None => {
            eprintln!("Issue body (end with Ctrl-D):");
            let mut b = String::new();
            io::stdin().read_to_string(&mut b).unwrap_or(0);
            b
        }
    };

    // Confirm (unless --auto with agent calling)
    if !auto {
        eprintln!("\n--- Preview ---");
        eprintln!("Repo:  {repo}");
        eprintln!("Title: {title}");
        eprintln!("Body:\n{body}");
        eprintln!("--- End Preview ---\n");
        eprint!("Submit? [y/N] ");
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).unwrap_or(0);
        if !answer.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return 0;
        }
    }

    // Create issue via gh
    match create_issue(&repo, &title, &body) {
        Ok(url) => {
            println!("{url}");
            0
        }
        Err(e) => {
            eprintln!("Failed to create issue: {e}");
            1
        }
    }
}

fn resolve_product(
    config: &registry::Config,
    product_override: Option<&str>,
) -> Option<(String, String)> {
    if let Some(name) = product_override {
        let p = config.products.iter().find(|p| p.name == name)?;
        return Some((p.name.clone(), p.full_repo(&config.org)));
    }

    let d = detect::detect(config)?;
    Some((d.product, d.repo))
}

fn collect_auto_context() -> String {
    let mut parts = Vec::new();

    // OS info
    if let Ok(output) = Command::new("uname").args(["-s", "-r"]).output() {
        if output.status.success() {
            parts.push(format!(
                "OS: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
    }

    // Git branch
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
    {
        if output.status.success() {
            parts.push(format!(
                "Branch: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
    }

    // Last commit
    if let Ok(output) = Command::new("git")
        .args(["log", "--oneline", "-1"])
        .output()
    {
        if output.status.success() {
            parts.push(format!(
                "Last commit: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
    }

    if parts.is_empty() {
        "(no context collected)".to_string()
    } else {
        format!("## Environment\n\n{}", parts.join("\n"))
    }
}

fn create_issue(repo: &str, title: &str, body: &str) -> Result<String, String> {
    // Write body to temp file to avoid shell quoting issues
    let tmp = std::env::temp_dir().join(format!("snag-body-{}.md", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| format!("Cannot write temp file: {e}"))?;

    let output = Command::new("gh")
        .args([
            "issue",
            "create",
            "-R",
            repo,
            "--title",
            title,
            "--body-file",
            &tmp.to_string_lossy(),
            "--label",
            "bug",
        ])
        .output()
        .map_err(|e| format!("gh not found: {e}"))?;

    let _ = std::fs::remove_file(&tmp);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Retry without --label if label doesn't exist
        if stderr.contains("label") {
            let output2 = Command::new("gh")
                .args([
                    "issue",
                    "create",
                    "-R",
                    repo,
                    "--title",
                    title,
                    "--body-file",
                    &tmp.to_string_lossy(),
                ])
                .output()
                .map_err(|e| format!("gh retry failed: {e}"))?;

            if !output2.status.success() {
                return Err(String::from_utf8_lossy(&output2.stderr).to_string());
            }
            return Ok(String::from_utf8_lossy(&output2.stdout).trim().to_string());
        }
        return Err(stderr.to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

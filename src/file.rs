use crate::{
    bundle::{
        ContextInfo, EnvironmentInfo, ErrorInfo, ErrorKind, Metadata, ProductInfo, ReportBundle,
        ReporterInfo, UserInfo, SCHEMA_VERSION, now_utc,
    },
    check, detect, redact, registry,
};
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

    // Resolve product → (name, repo, version)
    let (product_name, product_repo, product_version) =
        match resolve_product(&config, product) {
            Some(r) => r,
            None => {
                eprintln!(
                    "Cannot determine target repo. Use --product or cd into the project."
                );
                return 1;
            }
        };

    // Resolve title
    let title = match title {
        Some(t) => t.to_string(),
        None if auto => {
            eprintln!(
                "--auto requires --title (auto-title from agent context not yet implemented)"
            );
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

    // Resolve body
    let body = match body {
        Some(b) => b.to_string(),
        None if auto => String::new(),
        None => {
            eprintln!("Issue body (end with Ctrl-D):");
            let mut b = String::new();
            io::stdin().read_to_string(&mut b).unwrap_or(0);
            b
        }
    };

    // Collect environment + context (redacted)
    let environment = collect_environment();
    let context = collect_context();

    // Build error info
    let error = ErrorInfo {
        kind: ErrorKind::UserReport,
        message: title.clone(),
        location: None,
        backtrace: None,
        exit_code: None,
    };

    // Compute fingerprint
    let fp = ReportBundle::fingerprint(
        &product_name,
        &ErrorKind::UserReport,
        &title,
        None,
        product_version.as_deref(),
    );

    // Dedup check (fingerprint-first)
    if !no_check && !force {
        if check::has_duplicate_with_fp(&product_repo, &title, Some(&fp)) {
            eprintln!(
                "Similar issue already exists in {product_repo}. Use --force to file anyway."
            );
            let _ = check::run(&title, Some(&product_name), config_path);
            return 2;
        }
    }

    let now = now_utc();
    let bundle_path =
        std::env::temp_dir().join(format!("snag-bundle-{}.json", std::process::id()));

    let bundle = ReportBundle {
        schema_version: SCHEMA_VERSION,
        fingerprint: fp,
        product: ProductInfo {
            name: product_name.clone(),
            repo: product_repo.clone(),
            version: product_version,
            commit: None,
        },
        reporter: ReporterInfo {
            name: "snag".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            mode: "standalone".to_string(),
        },
        environment,
        error,
        context,
        user: UserInfo {
            title: Some(title.clone()),
            body: if body.is_empty() { None } else { Some(body) },
            labels: vec!["bug".to_string()],
        },
        metadata: Metadata {
            created_at: now,
            bundle_path: bundle_path.to_string_lossy().to_string(),
            submitted: false,
            issue_url: None,
        },
    };

    // Save bundle to disk
    if let Err(e) = bundle.save(&bundle_path) {
        eprintln!("Warning: could not save bundle: {e}");
    } else {
        eprintln!("Bundle saved: {}", bundle_path.display());
    }

    // Auto-submit (or prompt)
    crate::submit::run(
        &bundle_path.to_string_lossy(),
        auto || force,
        config_path,
    )
}

fn resolve_product(
    config: &registry::Config,
    product_override: Option<&str>,
) -> Option<(String, String, Option<String>)> {
    if let Some(name) = product_override {
        let p = config.products.iter().find(|p| p.name == name)?;
        return Some((p.name.clone(), p.full_repo(&config.org), None));
    }

    let d = detect::detect(config)?;
    Some((d.product, d.repo, None))
}

/// Collect environment info using the redaction allowlist.
pub fn collect_environment() -> EnvironmentInfo {
    let env = redact::redact_env();

    // OS + arch via uname
    let os = Command::new("uname")
        .args(["-s", "-r"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let arch = Command::new("uname")
        .arg("-m")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let shell = env.get("SHELL").cloned();

    EnvironmentInfo {
        os,
        arch,
        shell,
        env,
    }
}

/// Collect context info: git branch, commit, last command.
pub fn collect_context() -> ContextInfo {
    let git_branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let git_commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    ContextInfo {
        git_branch,
        git_commit,
        command: None,
        breadcrumbs: vec![],
    }
}

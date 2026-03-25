use crate::registry;
use std::process::Command;

pub struct Detection {
    pub product: String,
    pub repo: String,
    pub confidence: u8,
    pub signal: &'static str,
}

pub fn run(config_path: Option<&str>) -> i32 {
    let config = match registry::load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    match detect(&config) {
        Some(d) => {
            println!(
                "{} (confidence: {}%, signal: {}, repo: {})",
                d.product, d.confidence, d.signal, d.repo
            );
            0
        }
        None => {
            eprintln!("No product detected. Checked: git-remote, manifest, cwd-path.");
            1
        }
    }
}

pub fn detect(config: &registry::Config) -> Option<Detection> {
    // Signal 1: git remote URL (95% confidence)
    if let Some(remote) = git_remote_url() {
        if let Some(product) = registry::find_by_remote(config, &remote) {
            return Some(Detection {
                product: product.name.clone(),
                repo: product.full_repo(&config.org),
                confidence: 95,
                signal: "git-remote",
            });
        }
    }

    // Signal 2: manifest file walk (85% confidence)
    let cwd = std::env::current_dir().ok()?;
    if let Some((product, _dir)) = registry::find_by_manifest(config, &cwd) {
        return Some(Detection {
            product: product.name.clone(),
            repo: product.full_repo(&config.org),
            confidence: 85,
            signal: "manifest",
        });
    }

    // Signal 3: CWD path segment (60% confidence)
    if let Some(product) = registry::find_by_path(config, &cwd) {
        return Some(Detection {
            product: product.name.clone(),
            repo: product.full_repo(&config.org),
            confidence: 60,
            signal: "cwd-path",
        });
    }

    None
}

fn git_remote_url() -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() { None } else { Some(url) }
}

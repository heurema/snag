use crate::{detect, registry};
use std::process::Command;

pub fn run(title: &str, product: Option<&str>, config_path: Option<&str>) -> i32 {
    let config = match registry::load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    let repo = match resolve_repo(&config, product) {
        Some(r) => r,
        None => {
            eprintln!("Cannot determine target repo. Use --product or cd into the project.");
            return 1;
        }
    };

    match find_similar(&repo, title) {
        Ok(matches) => {
            if matches.is_empty() {
                println!("No similar open issues found in {repo}.");
                0
            } else {
                println!("Similar open issues in {repo}:");
                for m in &matches {
                    let sim = jaccard_similarity(title, &m.title);
                    println!(
                        "  #{} (similarity: {:.0}%) {}",
                        m.number,
                        sim * 100.0,
                        m.title
                    );
                    println!("    https://github.com/{repo}/issues/{}", m.number);
                }
                2 // exit code 2 = duplicates found
            }
        }
        Err(e) => {
            eprintln!("Search failed: {e}");
            1
        }
    }
}

fn resolve_repo(config: &registry::Config, product_override: Option<&str>) -> Option<String> {
    if let Some(name) = product_override {
        let p = config.products.iter().find(|p| p.name == name)?;
        return Some(p.full_repo(&config.org));
    }

    let d = detect::detect(config)?;
    Some(d.repo)
}

pub struct IssueMatch {
    pub number: u64,
    pub title: String,
}

pub fn find_similar(repo: &str, title: &str) -> Result<Vec<IssueMatch>, String> {
    // Extract keywords: take words ≥4 chars, up to 5
    let keywords: Vec<&str> = title
        .split_whitespace()
        .filter(|w| w.len() >= 4)
        .take(5)
        .collect();

    if keywords.is_empty() {
        return Ok(vec![]);
    }

    let query = keywords.join(" ");
    let output = Command::new("gh")
        .args([
            "search",
            "issues",
            &query,
            "--repo",
            repo,
            "--match",
            "title",
            "--state",
            "open",
            "--json",
            "number,title",
            "--limit",
            "5",
        ])
        .output()
        .map_err(|e| format!("gh not found or failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh search failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<GhIssue> =
        serde_json::from_str(&stdout).map_err(|e| format!("JSON parse error: {e}"))?;

    Ok(items
        .into_iter()
        .map(|i| IssueMatch {
            number: i.number,
            title: i.title,
        })
        .collect())
}

/// Check if any existing issue is a likely duplicate (Jaccard > 0.8).
pub fn has_duplicate(repo: &str, title: &str) -> bool {
    if let Ok(matches) = find_similar(repo, title) {
        matches
            .iter()
            .any(|m| jaccard_similarity(title, &m.title) > 0.8)
    } else {
        false // on error, don't block filing
    }
}

fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let set_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

#[derive(serde::Deserialize)]
struct GhIssue {
    number: u64,
    title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jaccard_identical() {
        assert!((jaccard_similarity("hello world", "hello world") - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_jaccard_different() {
        assert!(jaccard_similarity("hello world", "foo bar") < 0.01);
    }

    #[test]
    fn test_jaccard_partial() {
        let sim = jaccard_similarity("fix gemini JSON parse", "fix gemini JSON fence strip");
        assert!(sim > 0.3 && sim < 0.8);
    }
}

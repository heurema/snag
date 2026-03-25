use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub org: Org,
    #[serde(default)]
    pub products: Vec<Product>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Org {
    pub name: String,
    pub github: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Product {
    pub name: String,
    /// GitHub repo name, defaults to product name
    pub repo: Option<String>,
    /// Detection markers: "Cargo.toml:name", ".signum/", "plugin.json:name"
    #[serde(default)]
    pub markers: Vec<String>,
}

impl Product {
    pub fn repo_name(&self) -> &str {
        self.repo.as_deref().unwrap_or(&self.name)
    }

    pub fn full_repo(&self, org: &Org) -> String {
        format!("{}/{}", org.github, self.repo_name())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    #[serde(default = "default_max_issues")]
    pub max_issues_per_session: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_issues_per_session: default_max_issues(),
        }
    }
}

fn default_max_issues() -> u32 {
    5
}

pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("snag")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("snag")
    } else {
        PathBuf::from(".config").join("snag")
    }
}

pub fn load_config(override_path: Option<&str>) -> Result<Config, String> {
    let path = match override_path {
        Some(p) => PathBuf::from(p),
        None => config_dir().join("config.toml"),
    };

    if !path.exists() {
        return Err(format!(
            "Config not found: {}. Run `snag init` first.",
            path.display()
        ));
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;

    parse_toml(&content).map_err(|e| format!("Invalid config {}: {e}", path.display()))
}

/// Minimal TOML parser — handles [org], [[products]], [settings].
/// Avoids adding toml crate dependency for a simple config format.
fn parse_toml(input: &str) -> Result<Config, String> {
    let mut org_name = String::new();
    let mut org_github = String::new();
    let mut products = Vec::new();
    let mut max_issues: u32 = 5;

    let mut current_section = String::new();
    let mut current_product: Option<ProductBuilder> = None;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed == "[[products]]" {
            if let Some(p) = current_product.take() {
                products.push(p.build()?);
            }
            current_product = Some(ProductBuilder::default());
            current_section = "products".to_string();
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(p) = current_product.take() {
                products.push(p.build()?);
            }
            current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
            continue;
        }

        if let Some((key, val)) = parse_kv(trimmed) {
            match current_section.as_str() {
                "org" => match key {
                    "name" => org_name = val,
                    "github" => org_github = val,
                    _ => {}
                },
                "settings" => {
                    if key == "max_issues_per_session" {
                        max_issues = val.parse().unwrap_or(5);
                    }
                }
                "products" => {
                    if let Some(ref mut p) = current_product {
                        match key {
                            "name" => p.name = Some(val),
                            "repo" => p.repo = Some(val),
                            "markers" => p.markers = parse_string_array(&val),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(p) = current_product.take() {
        products.push(p.build()?);
    }

    if org_github.is_empty() {
        return Err("Missing [org].github".to_string());
    }
    if org_name.is_empty() {
        org_name = org_github.clone();
    }

    Ok(Config {
        org: Org {
            name: org_name,
            github: org_github,
        },
        products,
        settings: Settings {
            max_issues_per_session: max_issues,
        },
    })
}

fn parse_kv(line: &str) -> Option<(&str, String)> {
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    let val = rest.trim().trim_matches('"');
    Some((key, val.to_string()))
}

fn parse_string_array(val: &str) -> Vec<String> {
    let s = val.trim();
    if s.starts_with('[') && s.ends_with(']') {
        s[1..s.len() - 1]
            .split(',')
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|v| !v.is_empty())
            .collect()
    } else {
        vec![s.to_string()]
    }
}

#[derive(Default)]
struct ProductBuilder {
    name: Option<String>,
    repo: Option<String>,
    markers: Vec<String>,
}

impl ProductBuilder {
    fn build(self) -> Result<Product, String> {
        let name = self.name.ok_or("[[products]] missing name")?;
        Ok(Product {
            name,
            repo: self.repo,
            markers: self.markers,
        })
    }
}

/// Find a product by matching git remote URL against known org repos.
pub fn find_by_remote<'a>(config: &'a Config, remote_url: &str) -> Option<&'a Product> {
    let repo_name = extract_repo_name(remote_url)?;
    config
        .products
        .iter()
        .find(|p| p.repo_name() == repo_name || p.name == repo_name)
}

/// Extract repo name from git remote URL.
/// Handles: https://github.com/org/repo.git, git@github.com:org/repo.git
fn extract_repo_name(url: &str) -> Option<String> {
    let url = url.trim();
    // SSH: git@github.com:org/repo.git
    if let Some(rest) = url.strip_prefix("git@") {
        let path = rest.split_once(':')?.1;
        let repo = path.trim_end_matches(".git").rsplit('/').next()?;
        return Some(repo.to_string());
    }
    // HTTPS: https://github.com/org/repo.git
    let repo = url.trim_end_matches(".git").rsplit('/').next()?;
    if repo.is_empty() {
        return None;
    }
    Some(repo.to_string())
}

/// Find a product by scanning manifest files upward from CWD.
pub fn find_by_manifest<'a>(config: &'a Config, start_dir: &Path) -> Option<(&'a Product, PathBuf)> {
    let mut dir = start_dir.to_path_buf();
    loop {
        for product in &config.products {
            for marker in &product.markers {
                if let Some(colon_pos) = marker.find(':') {
                    // "Cargo.toml:name" — check file contains package name
                    let file = &marker[..colon_pos];
                    let expected_name = &marker[colon_pos + 1..];
                    let path = dir.join(file);
                    if path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if content.contains(expected_name) {
                                return Some((product, dir.clone()));
                            }
                        }
                    }
                } else if marker.ends_with('/') {
                    // ".signum/" — check directory exists
                    if dir.join(marker.trim_end_matches('/')).is_dir() {
                        return Some((product, dir.clone()));
                    }
                } else {
                    // "some-file" — check file exists
                    if dir.join(marker).is_file() {
                        return Some((product, dir.clone()));
                    }
                }
            }
        }

        if !dir.pop() {
            break;
        }
    }
    None
}

/// Find a product by matching CWD path segments against product names.
pub fn find_by_path<'a>(config: &'a Config, cwd: &Path) -> Option<&'a Product> {
    let cwd_str = cwd.to_string_lossy();
    config.products.iter().find(|p| {
        cwd_str.contains(&format!("/{}/", p.name))
            || cwd_str.ends_with(&format!("/{}", p.name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_name_https() {
        assert_eq!(
            extract_repo_name("https://github.com/heurema/signum.git"),
            Some("signum".to_string())
        );
    }

    #[test]
    fn test_extract_repo_name_ssh() {
        assert_eq!(
            extract_repo_name("git@github.com:heurema/signum.git"),
            Some("signum".to_string())
        );
    }

    #[test]
    fn test_extract_repo_name_no_git_suffix() {
        assert_eq!(
            extract_repo_name("https://github.com/heurema/mycel"),
            Some("mycel".to_string())
        );
    }

    #[test]
    fn test_parse_config() {
        let toml = r#"
[org]
name = "heurema"
github = "heurema"

[[products]]
name = "signum"
markers = [".signum/", "plugin.json:signum"]

[[products]]
name = "mycel"
repo = "mycel"
markers = ["Cargo.toml:mycel"]

[settings]
max_issues_per_session = 10
"#;
        let config = parse_toml(toml).unwrap();
        assert_eq!(config.org.github, "heurema");
        assert_eq!(config.products.len(), 2);
        assert_eq!(config.products[0].name, "signum");
        assert_eq!(config.products[0].markers.len(), 2);
        assert_eq!(config.products[1].repo_name(), "mycel");
        assert_eq!(config.settings.max_issues_per_session, 10);
    }
}

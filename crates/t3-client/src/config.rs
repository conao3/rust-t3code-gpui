use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub label: String,
    pub http_base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub environments: Vec<Environment>,
}

impl Config {
    pub fn default_path() -> Result<PathBuf> {
        let base = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => {
                let home = std::env::var_os("HOME").context("HOME is not set")?;
                PathBuf::from(home).join(".config")
            }
        };
        Ok(base.join("t3code-gpui").join("environments.json"))
    }

    pub fn load(path: &Path) -> Result<Config> {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("cannot read the environments file {}", path.display()))?;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!(
                "{} holds tokens and must not be readable by group or others (chmod 600)",
                path.display()
            );
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read the environments file {}", path.display()))?;
        Config::parse(&text)
            .with_context(|| format!("invalid environments file {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        let config: Config = serde_json::from_str(text)?;
        if config.environments.is_empty() {
            bail!("no environments are configured");
        }
        for (i, environment) in config.environments.iter().enumerate() {
            if environment.label.trim().is_empty() {
                bail!("environment {i} has an empty label");
            }
            if !environment.http_base_url.starts_with("http://")
                && !environment.http_base_url.starts_with("https://")
            {
                bail!(
                    "environment {} has an httpBaseUrl that is not http(s)",
                    environment.label
                );
            }
            if environment.token.trim().is_empty() {
                bail!("environment {} has an empty token", environment.label);
            }
            if config.environments[..i]
                .iter()
                .any(|other| other.label == environment.label)
            {
                bail!("environment label {} appears twice", environment.label);
            }
        }
        Ok(config)
    }

    pub fn find(&self, label: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.label == label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const VALID: &str =
        r#"{"environments":[{"label":"eos","httpBaseUrl":"http://127.0.0.1:3774","token":"abc"}]}"#;

    #[test]
    fn parses_a_valid_file() {
        let config = Config::parse(VALID).unwrap();
        assert_eq!(config.environments[0].label, "eos");
        assert_eq!(config.find("eos").unwrap().token, "abc");
    }

    #[test]
    fn rejects_duplicate_labels() {
        let text = r#"{"environments":[
            {"label":"eos","httpBaseUrl":"http://a","token":"x"},
            {"label":"eos","httpBaseUrl":"http://b","token":"y"}]}"#;
        assert!(Config::parse(text).is_err());
    }

    #[test]
    fn rejects_non_http_urls_and_empty_tokens() {
        let bad_url = r#"{"environments":[{"label":"a","httpBaseUrl":"ws://a","token":"x"}]}"#;
        let no_token = r#"{"environments":[{"label":"a","httpBaseUrl":"http://a","token":" "}]}"#;
        assert!(Config::parse(bad_url).is_err());
        assert!(Config::parse(no_token).is_err());
        assert!(Config::parse(r#"{"environments":[]}"#).is_err());
    }

    #[test]
    fn refuses_a_file_readable_by_others() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(VALID.as_bytes()).unwrap();
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Config::load(file.path()).is_err());
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Config::load(file.path()).is_ok());
    }
}

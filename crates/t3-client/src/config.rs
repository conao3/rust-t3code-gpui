use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub label: String,
    pub http_base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub environments: Vec<Environment>,
}

/// The server origin and pairing code of a pairing URL (`<origin>/pair#token=<code>`, or the
/// same with `?token=`), as `t3 pair` prints it.
pub fn pairing_url(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (base, code) = url
        .split_once("#token=")
        .or_else(|| url.split_once("?token="))?;
    let origin = base
        .strip_suffix("/pair")
        .unwrap_or(base)
        .trim_end_matches('/');
    let code = code.split(['&', '#']).next()?.trim();
    (origin.starts_with("http") && !code.is_empty()).then(|| (origin.to_string(), code.to_string()))
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

    /// Writes the environments to `path` readable only by the owner, replacing the file in one step.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        Config::parse(&text)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, format!("{text}\n"))
            .with_context(|| format!("cannot write {}", partial.display()))?;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&partial, path)
            .with_context(|| format!("cannot replace {}", path.display()))
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

    #[test]
    fn a_saved_config_loads_back_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t3code-gpui/environments.json");
        let config = Config {
            environments: vec![Environment {
                label: "eos".into(),
                http_base_url: "http://127.0.0.1:3773".into(),
                token: "secret".into(),
            }],
        };
        config.save(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(Config::load(&path).unwrap(), config);
        let empty = Config {
            environments: vec![],
        };
        assert!(empty.save(&path).is_err());
    }

    #[test]
    fn pairing_urls_give_the_origin_and_code() {
        assert_eq!(
            pairing_url("http://127.0.0.1:38792/pair#token=3FLT3TD7BAWT"),
            Some(("http://127.0.0.1:38792".into(), "3FLT3TD7BAWT".into()))
        );
        assert_eq!(
            pairing_url(" https://h.ts.net:8443/pair?token=ABC&x=1 "),
            Some(("https://h.ts.net:8443".into(), "ABC".into()))
        );
        assert_eq!(pairing_url("3FLT3TD7BAWT"), None);
    }
}

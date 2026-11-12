use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};

/// T3's limits for a sent image and file (`PROVIDER_SEND_TURN_MAX_IMAGE_BYTES` and
/// `PROVIDER_SEND_TURN_MAX_FILE_BYTES`).
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub name: String,
    pub mime_type: String,
    pub bytes: Arc<Vec<u8>>,
}

impl Pending {
    pub fn new(name: String, mime_type: String, bytes: Vec<u8>) -> Result<Self> {
        let limit = if mime_type.starts_with("image/") {
            MAX_IMAGE_BYTES
        } else {
            MAX_FILE_BYTES
        };
        if bytes.is_empty() {
            bail!("{name} is empty");
        }
        if bytes.len() > limit {
            bail!(
                "{name} is larger than the {} MiB T3 accepts",
                limit / 1024 / 1024
            );
        }
        Ok(Self {
            name,
            mime_type,
            bytes: Arc::new(bytes),
        })
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{} has no file name", path.display()))?
            .to_string();
        let bytes =
            std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        Self::new(name, mime_for(path).to_string(), bytes)
    }
}

/// The media type T3 is told for a file, from its extension: the image types T3 sends to a
/// provider as images, text for source and plain text, and bytes otherwise.
pub fn mime_for(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "txt" | "log" | "rs" | "ts" | "tsx" | "js" | "py" | "nix" | "toml" | "yaml" | "yml"
        | "sh" | "clj" | "el" | "go" | "c" | "h" | "css" => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(mime_for(Path::new("shot.PNG")), "image/png");
        assert_eq!(mime_for(Path::new("notes.md")), "text/markdown");
        assert_eq!(mime_for(Path::new("main.rs")), "text/plain");
        assert_eq!(mime_for(Path::new("blob")), "application/octet-stream");
    }

    #[test]
    fn empty_and_oversized_files_are_refused() {
        assert!(Pending::new("a.png".into(), "image/png".into(), vec![]).is_err());
        assert!(
            Pending::new(
                "a.png".into(),
                "image/png".into(),
                vec![0; MAX_IMAGE_BYTES + 1]
            )
            .is_err()
        );
        assert!(
            Pending::new(
                "a.bin".into(),
                "application/octet-stream".into(),
                vec![0; MAX_IMAGE_BYTES + 1]
            )
            .is_ok()
        );
    }

    #[test]
    fn a_file_on_disk_becomes_a_pending_attachment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "# hi").unwrap();
        let pending = Pending::from_path(&path).unwrap();
        assert_eq!(
            (pending.name.as_str(), pending.mime_type.as_str()),
            ("notes.md", "text/markdown")
        );
        assert_eq!(pending.bytes.len(), 4);
    }
}

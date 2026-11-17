use anyhow::{Context as _, Result};
use std::io::Read as _;
use std::process::{Child, Command, Stdio};

pub struct Recording {
    child: Child,
    reader: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
}

impl Recording {
    pub fn start() -> Result<Self> {
        let mut child = Command::new("pw-record")
            .args([
                "--raw",
                "--rate",
                "16000",
                "--channels",
                "1",
                "--format",
                "f32",
                "--sample-count",
                "4800000",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Could not start microphone capture with pw-record")?;
        let mut stdout = child.stdout.take().context("Microphone output missing")?;
        let reader = std::thread::spawn(move || {
            let mut bytes = vec![];
            stdout.read_to_end(&mut bytes)?;
            Ok(bytes)
        });
        Ok(Self {
            child,
            reader: Some(reader),
        })
    }
    pub fn finish(mut self) -> Result<Vec<u8>> {
        let exited = self.child.try_wait()?;
        if exited.is_none() {
            self.child.kill()?;
        }
        self.child.wait()?;
        let bytes = self
            .reader
            .take()
            .context("Microphone reader missing")?
            .join()
            .map_err(|_| anyhow::anyhow!("Microphone reader failed"))??;
        if bytes.is_empty() {
            let mut error = String::new();
            if let Some(mut stderr) = self.child.stderr.take() {
                stderr.read_to_string(&mut error)?;
            }
            anyhow::bail!("No microphone audio was captured: {}", error.trim());
        }
        anyhow::ensure!(
            bytes.len().is_multiple_of(4),
            "Microphone PCM is incomplete"
        );
        Ok(bytes)
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn encode(pcm: &[u8], options: &serde_json::Value) -> Result<Vec<u8>> {
    let metadata = serde_json::to_vec(options)?;
    anyhow::ensure!(metadata.len() <= 128_000, "Speech options are too large");
    anyhow::ensure!(
        !pcm.is_empty() && pcm.len().is_multiple_of(4),
        "Expected 16 kHz mono float32 PCM"
    );
    let mut body = Vec::with_capacity(4 + metadata.len() + pcm.len());
    body.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    body.extend_from_slice(&metadata);
    body.extend_from_slice(pcm);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_request_matches_reference_binary_layout() {
        let options = serde_json::json!({"speechLanguage":"ja"});
        let body = encode(&[0, 0, 0, 0], &options).unwrap();
        let length = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body[4..4 + length]).unwrap(),
            options
        );
        assert_eq!(&body[4 + length..], &[0, 0, 0, 0]);
        assert!(encode(&[1], &options).is_err());
    }
}

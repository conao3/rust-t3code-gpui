use super::*;

#[derive(Default)]
pub(super) struct DraftWriter {
    next: std::sync::atomic::AtomicU64,
    saved: std::sync::Mutex<u64>,
}

pub(super) struct DraftWrite {
    writer: Arc<DraftWriter>,
    revision: u64,
    path: std::path::PathBuf,
    drafts: HashMap<String, Vec<attachments::Pending>>,
}

impl DraftWriter {
    pub(super) fn request(
        self: &Arc<Self>,
        path: std::path::PathBuf,
        drafts: HashMap<String, Vec<attachments::Pending>>,
    ) -> DraftWrite {
        DraftWrite {
            writer: self.clone(),
            revision: self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
            path,
            drafts,
        }
    }
}

impl DraftWrite {
    pub(super) fn save(self) -> anyhow::Result<()> {
        let mut saved = self.writer.saved.lock().expect("draft attachment writer");
        if self.revision < *saved {
            return Ok(());
        }
        save_attachment_drafts(&self.path, &self.drafts)?;
        *saved = self.revision;
        Ok(())
    }
}

pub(super) fn attachment_drafts_path(path: &std::path::Path, pane: u64) -> std::path::PathBuf {
    path.with_file_name(format!("draft-attachments-{pane}.json"))
}

pub(super) fn load_attachment_drafts(
    path: &std::path::Path,
) -> anyhow::Result<HashMap<String, Vec<attachments::Pending>>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error.into()),
    };
    let saved: HashMap<String, Vec<crate::prompt_store::SavedAttachment>> =
        serde_json::from_slice(&bytes)?;
    saved
        .into_iter()
        .map(|(key, files)| {
            let files = files
                .into_iter()
                .map(|file| attachments::Pending::new(file.name, file.mime_type, file.bytes))
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok((key, files))
        })
        .collect()
}

pub(super) fn save_attachment_drafts(
    path: &std::path::Path,
    drafts: &HashMap<String, Vec<attachments::Pending>>,
) -> anyhow::Result<()> {
    let saved: HashMap<_, _> = drafts
        .iter()
        .map(|(key, files)| {
            (
                key,
                files
                    .iter()
                    .map(|file| crate::prompt_store::SavedAttachment {
                        name: file.name.clone(),
                        mime_type: file.mime_type.clone(),
                        bytes: file.bytes.as_ref().clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    std::fs::create_dir_all(path.parent().context("Attachment draft directory")?)?;
    let temporary = path.with_extension("partial");
    std::fs::write(&temporary, serde_json::to_vec(&saved)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

impl Root {
    pub(super) fn load_timeline_images(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.conversation.open else {
            return;
        };
        let Some(connection) = self.connection(&open.environment) else {
            return;
        };
        let state = open.watch.state();
        let sources: HashMap<String, Vec<Value>> = state
            .lock()
            .expect("thread")
            .items()
            .into_iter()
            .filter(|item| item["type"] == "user_message")
            .filter_map(|item| {
                Some((
                    item["id"].as_str()?.to_string(),
                    item["attachments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|a| a["type"] == "image")
                        .cloned()
                        .collect(),
                ))
            })
            .collect();
        for (row, view) in self
            .conversation
            .rows
            .iter()
            .zip(&self.conversation.row_views)
        {
            let source = sources.get(&row.key).cloned().unwrap_or_default();
            view.update(cx, |view, cx| {
                view.load_images(source, connection.clone(), self.runtime.clone(), cx)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DraftWriter, attachment_drafts_path, load_attachment_drafts, save_attachment_drafts,
    };
    use crate::attachments;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn closing_a_pane_supersedes_its_pending_attachment_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("attachments.json");
        let writer = Arc::new(DraftWriter::default());
        let old = writer.request(path.clone(), HashMap::new());
        let saved = HashMap::from([(
            "thread".into(),
            vec![
                attachments::Pending::new(
                    "note.txt".into(),
                    "text/plain".into(),
                    b"latest attachment".to_vec(),
                )
                .unwrap(),
            ],
        )]);
        writer.request(path.clone(), saved.clone()).save().unwrap();
        old.save().unwrap();
        assert_eq!(load_attachment_drafts(&path).unwrap(), saved);
    }

    #[test]
    fn attachment_drafts_survive_restart_and_removal_per_pane() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = directory.path().join("drafts.json");
        let first = attachment_drafts_path(&drafts, 1);
        let second = attachment_drafts_path(&drafts, 2);
        let file = attachments::Pending::new(
            "note.txt".into(),
            "text/plain".into(),
            b"draft attachment".to_vec(),
        )
        .unwrap();
        let saved = HashMap::from([("same-thread".into(), vec![file])]);
        save_attachment_drafts(&first, &saved).unwrap();
        save_attachment_drafts(&second, &saved).unwrap();
        assert_eq!(load_attachment_drafts(&first).unwrap(), saved);
        save_attachment_drafts(&first, &HashMap::new()).unwrap();
        assert!(load_attachment_drafts(&first).unwrap().is_empty());
        assert_eq!(load_attachment_drafts(&second).unwrap(), saved);
    }
}

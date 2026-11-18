#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Home,
    Draft(String),
    Thread(String, String),
}

pub struct History {
    entries: Vec<Target>,
    at: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            entries: vec![Target::Home],
            at: 0,
        }
    }
}

impl History {
    pub fn promote_draft(&mut self, id: &str, environment: &str, thread: &str) {
        for entry in &mut self.entries {
            if matches!(entry, Target::Draft(draft) if draft == id) {
                *entry = Target::Thread(environment.to_string(), thread.to_string());
            }
        }
    }

    pub fn replace(&mut self, target: Target) {
        self.entries[self.at] = target;
    }

    pub fn visit(&mut self, target: Target) {
        if self.entries.get(self.at) == Some(&target) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.at + 1);
        }
        self.entries.push(target);
        self.at = self.entries.len() - 1;
    }
    pub fn can_back(&self) -> bool {
        self.at > 0
    }
    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.entries.len()
    }
    pub fn step(&mut self, forward: bool) -> Option<Target> {
        if forward && self.can_forward() {
            self.at += 1;
        } else if !forward && self.can_back() {
            self.at -= 1;
        } else {
            return None;
        }
        self.entries.get(self.at).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::{History, Target};
    #[test]
    fn promoted_drafts_navigate_to_the_created_thread() {
        let mut history = History::default();
        history.visit(Target::Draft("draft-a".into()));
        history.visit(Target::Draft("draft-b".into()));
        history.promote_draft("draft-a", "host-a", "thread-a");
        assert_eq!(
            history.step(false),
            Some(Target::Thread("host-a".into(), "thread-a".into()))
        );
        assert_eq!(history.step(true), Some(Target::Draft("draft-b".into())));
    }
    #[test]
    fn visiting_after_back_replaces_the_forward_branch() {
        let mut history = History::default();
        let target = |s: &str| Target::Thread("host".into(), s.into());
        assert!(history.step(false).is_none());
        for id in ["a", "b", "c"] {
            history.visit(target(id));
        }
        assert_eq!(history.step(false), Some(target("b")));
        history.visit(target("b"));
        assert!(history.can_forward());
        history.visit(target("d"));
        assert!(!history.can_forward());
        assert_eq!(history.step(false), Some(target("b")));
        assert_eq!(history.step(true), Some(target("d")));
    }

    #[test]
    fn home_and_drafts_survive_back_forward_and_duplicate_visits() {
        let mut history = History::default();
        let draft = Target::Draft("draft".into());
        history.visit(draft.clone());
        history.visit(draft.clone());
        history.visit(Target::Thread("remote".into(), "thread".into()));
        assert_eq!(history.step(false), Some(draft.clone()));
        assert_eq!(history.step(false), Some(Target::Home));
        assert_eq!(history.step(true), Some(draft));
        history.visit(Target::Home);
        assert!(!history.can_forward());
    }
}

#[derive(Default)]
pub struct History {
    entries: Vec<(String, String)>,
    at: usize,
}

impl History {
    pub fn visit(&mut self, target: (String, String)) {
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
    pub fn step(&mut self, forward: bool) -> Option<(String, String)> {
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
    use super::History;
    #[test]
    fn visiting_after_back_replaces_the_forward_branch() {
        let mut history = History::default();
        let target = |s: &str| ("host".into(), s.into());
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
}

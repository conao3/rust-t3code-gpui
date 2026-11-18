use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SavedPane {
    pub id: u64,
    pub side_chat_parent: Option<(String, String)>,
    pub composer: String,
    pub draft: Option<super::NewThreadDraft>,
    pub thread: Option<(String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) enum Layout {
    Pane(u64),
    Split {
        vertical: bool,
        fraction: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    fn validate(&self) -> anyhow::Result<()> {
        if let Self::Split {
            fraction,
            first,
            second,
            ..
        } = self
        {
            anyhow::ensure!(
                fraction.is_finite() && (0.15..=0.85).contains(fraction),
                "Invalid conversation split ratio"
            );
            first.validate()?;
            second.validate()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn width(&self, target: u64, width: f32) -> Option<f32> {
        match self {
            Self::Pane(id) => (*id == target).then_some(width),
            Self::Split {
                vertical,
                fraction,
                first,
                second,
            } => first
                .width(target, if *vertical { width } else { width * fraction })
                .or_else(|| {
                    second.width(
                        target,
                        if *vertical {
                            width
                        } else {
                            (width * (1. - fraction) - 4.).max(0.)
                        },
                    )
                }),
        }
    }

    pub fn ids(&self) -> Vec<u64> {
        match self {
            Self::Pane(id) => vec![*id],
            Self::Split { first, second, .. } => {
                first.ids().into_iter().chain(second.ids()).collect()
            }
        }
    }

    pub fn split(&mut self, target: u64, added: u64, vertical: bool) -> bool {
        match self {
            Self::Pane(id) if *id == target => {
                *self = Self::Split {
                    vertical,
                    fraction: 0.5,
                    first: Box::new(Self::Pane(target)),
                    second: Box::new(Self::Pane(added)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, added, vertical) || second.split(target, added, vertical)
            }
        }
    }

    pub fn remove(self, target: u64) -> Option<Self> {
        match self {
            Self::Pane(id) => (id != target).then_some(Self::Pane(id)),
            Self::Split {
                vertical,
                fraction,
                first,
                second,
            } => match (first.remove(target), second.remove(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    vertical,
                    fraction,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (first, second) => first.or(second),
            },
        }
    }

    pub fn resize(&mut self, path: &[bool], fraction: f32) {
        if let Self::Split {
            fraction: current,
            first,
            second,
            ..
        } = self
        {
            if let Some((second_child, rest)) = path.split_first() {
                if *second_child {
                    second.resize(rest, fraction)
                } else {
                    first.resize(rest, fraction)
                }
            } else {
                *current = fraction.clamp(0.15, 0.85);
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SavedLayout {
    pub layout: Layout,
    pub active: u64,
    pub panes: Vec<SavedPane>,
    pub closed: Vec<SavedPane>,
}

impl SavedLayout {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.layout.validate()?;
        let ids = self.layout.ids();
        anyhow::ensure!(!ids.is_empty(), "Invalid conversation pane count");
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        anyhow::ensure!(unique.len() == ids.len(), "Duplicate conversation panes");
        anyhow::ensure!(
            ids.contains(&self.active),
            "Active conversation pane is missing"
        );
        anyhow::ensure!(
            ids.len() == self.panes.len()
                && self
                    .panes
                    .iter()
                    .map(|pane| pane.id)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    == ids.len()
                && self.panes.iter().all(|pane| ids.contains(&pane.id)),
            "Conversation pane state is missing"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_width_follows_nested_pane_splits() {
        let mut layout = Layout::Pane(0);
        layout.split(0, 1, false);
        layout.split(1, 2, true);
        assert_eq!(layout.width(0, 1200.), Some(600.));
        assert_eq!(layout.width(1, 1200.), Some(596.));
        assert_eq!(layout.width(2, 1200.), Some(596.));
        layout.resize(&[], 0.25);
        assert_eq!(layout.width(0, 1200.), Some(300.));
        assert_eq!(layout.width(2, 1200.), Some(896.));
        assert_eq!(layout.width(3, 1200.), None);
    }

    #[test]
    fn closing_nested_split_keeps_the_remaining_conversations() {
        let mut layout = Layout::Pane(0);
        assert!(layout.split(0, 1, false));
        assert!(layout.split(1, 2, true));
        layout.resize(&[true], 0.7);
        let layout = layout.remove(1).unwrap();
        assert_eq!(layout.ids(), [0, 2]);
        let restored: Layout =
            serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(restored.ids(), [0, 2]);
        assert_eq!(restored.remove(0).unwrap().ids(), [2]);
    }
}

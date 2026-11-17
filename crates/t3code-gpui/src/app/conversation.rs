use super::*;

pub struct ConversationState {
    pub(super) open: Option<OpenThread>,
    pub(super) draft: Option<NewThreadDraft>,
    pub(super) rows: Vec<Row>,
    pub(super) row_views: Vec<Entity<RowView>>,
    pub(super) timeline_pane: Entity<TimelinePane>,
    pub(super) row_subscriptions: HashMap<String, Subscription>,
    pub(super) list_state: ListState,
    pub(super) user_cursor: Option<usize>,
    pub(super) hovered_message: Option<String>,
    pub(super) composer: Entity<TextareaState>,
    pub(super) composer_controls: Entity<ComposerControlsPane>,
    pub(super) composer_controls_stamp: Option<ComposerControlsStamp>,
    pub(super) message: Option<String>,
    pub(super) picker_open: bool,
    pub(super) picker_instance: Option<String>,
    pub(super) permissions_menu: Option<Entity<permissions::Menu>>,
    pub(super) permissions_subscription: Option<Subscription>,
    pub(super) model_picker: Entity<CommandState>,
    pub(super) attachments: Vec<attachments::Pending>,
    pub(super) queued: Vec<timeline::Queued>,
    pub(super) thread_loading: bool,
    pub(super) completion: Option<Completion>,
    pub(super) completion_seq: u64,
    pub(super) find_input: Entity<InputState>,
    pub(super) find_open: bool,
    pub(super) find_hits: Vec<usize>,
    pub(super) find_at: usize,
    pub(super) pending_composer: Option<String>,
    pub(super) thread_dirty: bool,
}

impl std::ops::Deref for Root {
    type Target = ConversationState;

    fn deref(&self) -> &Self::Target {
        &self.conversation
    }
}

impl std::ops::DerefMut for Root {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.conversation
    }
}

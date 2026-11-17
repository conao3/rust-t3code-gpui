use super::*;

#[derive(Clone)]
pub(super) enum Control {
    Switch,
    Select(&'static [(&'static str, &'static str)]),
    Number(f64, f64),
    Text,
}

#[derive(Clone)]
pub(super) struct Setting {
    pub page: &'static str,
    pub section: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default: Value,
    pub server: bool,
    pub control: Control,
}

pub(super) const PAGES: &[(&str, &str, &str)] = &[
    ("general", "General", ""),
    ("appearance", "Appearance", ""),
    ("chat", "Chat", ""),
    ("keybindings", "Keybindings", ""),
    ("providers", "Providers", "Agents"),
    ("threads", "Threads", ""),
    ("scheduled-tasks", "Scheduled tasks", ""),
    ("connections", "Environments", "Connections"),
    ("maintenance", "Maintenance", ""),
    ("labs", "Labs", ""),
    ("about", "About", ""),
    ("archived", "Archive", ""),
];

pub(super) fn settings() -> Vec<Setting> {
    use Control::*;
    let mut values = Vec::new();
    let mut add = |page, section, key, label, description, default, server, control| {
        values.push(Setting {
            page,
            section,
            key,
            label,
            description,
            default,
            server,
            control,
        });
    };
    add(
        "general",
        "Notifications",
        "notificationMode",
        "Thread notifications",
        "System alerts when a thread finishes, fails, or needs input or approval. Applies to this device while T3 Code is open.",
        json!("off"),
        false,
        Select(&[
            ("off", "Off"),
            ("notifications", "Notifications only"),
            ("sound", "Sound only"),
            ("notifications-and-sound", "Notifications with sound"),
        ]),
    );
    add(
        "general",
        "Notifications",
        "inAppNotificationsEnabled",
        "In-app notifications",
        "Show a toast when another thread finishes, fails, or needs input or approval while this app has focus.",
        json!(false),
        false,
        Switch,
    );
    add(
        "general",
        "Sidebar",
        "sidebarProjectGroupingMode",
        "Project grouping",
        "Combine matching repositories across environments.",
        json!("repository"),
        false,
        Switch,
    );
    add(
        "general",
        "Sidebar",
        "sidebarProjectSortOrder",
        "Project order",
        "Choose how projects are ordered in the sidebar.",
        json!("updated_at"),
        false,
        Select(&[
            ("updated_at", "Last user message"),
            ("created_at", "Created at"),
            ("manual", "Manual"),
        ]),
    );
    add(
        "general",
        "Sidebar",
        "sidebarWorkingShelfEnabled",
        "Working section",
        "Fold working and monitoring threads into a Working section. They return to the top of the inbox when they need you. The boring sidebar in Labs has no Working section.",
        json!(true),
        false,
        Switch,
    );
    add(
        "general",
        "Confirmations",
        "confirmThreadUnpin",
        "Confirm thread unpinning",
        "Ask before unpinning a thread.",
        json!(false),
        false,
        Switch,
    );
    add(
        "general",
        "Confirmations",
        "confirmThreadArchive",
        "Confirm thread archiving",
        "Ask before archiving a thread.",
        json!(false),
        false,
        Switch,
    );
    add(
        "general",
        "Confirmations",
        "confirmThreadDelete",
        "Confirm thread deletion",
        "Ask before deleting a thread.",
        json!(true),
        false,
        Switch,
    );
    add(
        "general",
        "Confirmations",
        "confirmQuit",
        "Quit shortcut behavior",
        "Choose how the quit shortcut closes the application.",
        json!("hold"),
        false,
        Select(&[
            ("direct", "Direct"),
            ("hold", "Hold"),
            ("double-click", "Double press"),
        ]),
    );
    add(
        "general",
        "Time",
        "timestampFormat",
        "Timestamp format",
        "Choose how times are displayed.",
        json!("locale"),
        false,
        Select(&[
            ("locale", "System default"),
            ("12-hour", "12-hour"),
            ("24-hour", "24-hour"),
        ]),
    );
    add(
        "appearance",
        "Theme",
        "gpuiAppearance",
        "Appearance",
        "Choose the color scheme for this device.",
        json!("dark"),
        false,
        Select(&[("dark", "Dark"), ("light", "Light")]),
    );
    add(
        "appearance",
        "Typography",
        "fontSizeInterface",
        "Interface font size",
        "Text in navigation and controls.",
        json!(16),
        false,
        Number(12., 20.),
    );
    add(
        "appearance",
        "Typography",
        "fontSizePrompt",
        "Prompt font size",
        "Text in messages and the composer.",
        json!(14),
        false,
        Number(12., 20.),
    );
    add(
        "appearance",
        "Typography",
        "fontSizeCode",
        "Code font size",
        "Text in code blocks.",
        json!(13),
        false,
        Number(10., 18.),
    );
    add(
        "appearance",
        "Typography",
        "fontSizeTerminal",
        "Terminal font size",
        "Text in the terminal.",
        json!(12),
        false,
        Number(8., 20.),
    );
    add(
        "appearance",
        "Chat",
        "chatWidth",
        "Chat width",
        "Choose the width of the conversation.",
        json!("comfortable"),
        false,
        Select(&[
            ("comfortable", "Comfortable"),
            ("wide", "Wide"),
            ("full", "Full width"),
        ]),
    );
    add(
        "appearance",
        "Chat",
        "effortDisplay",
        "Effort display",
        "Choose how model effort is displayed.",
        json!("label"),
        false,
        Select(&[("label", "Label"), ("bars", "Bars")]),
    );
    add(
        "appearance",
        "Chat",
        "mathRenderingEnabled",
        "Render math",
        "Render mathematical notation in messages.",
        json!(false),
        false,
        Switch,
    );
    add(
        "chat",
        "Composer",
        "composerRichTextEnabled",
        "Rich text composer",
        "Format text in the composer.",
        json!(true),
        false,
        Switch,
    );
    add(
        "chat",
        "Composer",
        "sendShortcut",
        "Send shortcut",
        "Choose the keyboard shortcut that sends a message.",
        json!("enter"),
        false,
        Select(&[
            ("enter", "Enter"),
            ("mod-enter-multiline", "Ctrl + Enter for multiline"),
            ("mod-enter", "Ctrl + Enter"),
        ]),
    );
    add(
        "chat",
        "Composer",
        "showSkillsInSlashMenu",
        "Show skills in slash menu",
        "Include skills in slash command suggestions.",
        json!(true),
        false,
        Switch,
    );
    add(
        "chat",
        "While an agent is running",
        "followUpBehavior",
        "Follow-up behavior",
        "Choose whether follow-up messages steer the running turn or wait in the queue.",
        json!("steer"),
        false,
        Select(&[("steer", "Steer"), ("queue", "Queue")]),
    );
    add(
        "chat",
        "Messages",
        "toolCallVisibility",
        "Tool calls in chat",
        "Choose which tool calls appear in the conversation.",
        json!("showAll"),
        false,
        Select(&[
            ("showAll", "Show all"),
            ("hideSuccessful", "Hide successful"),
            ("hideAll", "Hide all"),
            ("zen", "Zen"),
        ]),
    );
    add(
        "chat",
        "Accessibility",
        "screenReaderAnnouncements",
        "Screen reader announcements",
        "Choose what the screen reader announces.",
        json!("status"),
        false,
        Select(&[
            ("status", "Status only"),
            ("messages", "Status and messages"),
            ("everything", "Everything"),
        ]),
    );
    add(
        "threads",
        "Projects",
        "addProjectBaseDirectory",
        "Add project base directory",
        "Starting directory when adding a project.",
        json!(""),
        true,
        Text,
    );
    add(
        "threads",
        "Projects",
        "ghqCloneLayout",
        "Clone into host folders",
        "Organize cloned repositories by host and owner.",
        json!(false),
        true,
        Switch,
    );
    add(
        "threads",
        "Usage limits",
        "autoResumeLimitedThreads",
        "Resume after usage limits",
        "Resume interrupted threads when their usage limits reset.",
        json!(false),
        true,
        Switch,
    );
    add(
        "threads",
        "Usage limits",
        "snoozeLimitedThreads",
        "Snooze until limits reset",
        "Snooze limited threads until their usage is available again.",
        json!(false),
        true,
        Switch,
    );
    add(
        "threads",
        "Settling",
        "sidebarAutoSettleOnMerge",
        "Auto-settle merged threads",
        "Settle threads when their pull request is merged.",
        json!(true),
        true,
        Switch,
    );
    add(
        "threads",
        "Settling",
        "sidebarAutoSettleAfterDays",
        "Auto-settle inactive threads",
        "Sidebar threads with no activity for this long settle automatically.",
        json!(7),
        true,
        Switch,
    );
    add(
        "threads",
        "Settling",
        "sidebarAutoSettleAfterDays",
        "Days before auto-settle",
        "Settle inactive threads after this many days.",
        json!(7),
        true,
        Number(1., 90.),
    );
    add(
        "threads",
        "Settling",
        "sidebarAutoSettleKeepPullRequests",
        "Keep threads with a PR",
        "Keep threads with an open pull request active.",
        json!(true),
        true,
        Switch,
    );
    add(
        "threads",
        "Advanced",
        "idleCompaction",
        "Compact idle threads",
        "Choose when idle conversations are compacted.",
        json!("onReturn"),
        true,
        Select(&[
            ("onReturn", "When you return"),
            ("beforeCacheExpires", "Before the cache expires (beta)"),
            ("off", "Never"),
        ]),
    );
    add(
        "threads",
        "Advanced",
        "responseStreamingMode",
        "Response streaming",
        "Choose when the agent response becomes visible.",
        json!("paragraph"),
        true,
        Select(&[
            ("turn", "Wait for the full response"),
            ("paragraph", "Show finished paragraphs"),
        ]),
    );
    add(
        "threads",
        "Advanced",
        "threadTitleInstructions",
        "Thread title instructions",
        "Additional instructions for generating thread titles.",
        json!(""),
        true,
        Text,
    );
    add(
        "maintenance",
        "Updates",
        "enableProviderUpdateChecks",
        "Check provider versions",
        "Check for updates to installed providers.",
        json!(true),
        true,
        Switch,
    );
    add(
        "maintenance",
        "Updates",
        "continueThreadsAfterServerUpdate",
        "Continue threads after restarts",
        "Continue running threads after the environment restarts.",
        json!(true),
        true,
        Switch,
    );
    add(
        "labs",
        "Beta",
        "boringSidebarEnabled",
        "Boring sidebar (group by project)",
        "Group sidebar threads into project folders. Pinned threads stay on top, and Snoozed and Settled stay below the folders.",
        json!(false),
        false,
        Switch,
    );
    add(
        "labs",
        "Beta",
        "boringSidebarMultilineRows",
        "Two-line folder rows",
        "Show folder threads on two lines: the title, then the branch, pull request, and agent.",
        json!(false),
        false,
        Switch,
    );
    add(
        "labs",
        "Beta",
        "linkedEnvironmentsEnabled",
        "Linked environments",
        "Let agents here work on your other machines, and theirs here.",
        json!(false),
        true,
        Switch,
    );
    values
}

pub(super) fn project_scoped(key: &str) -> bool {
    matches!(
        key,
        "threadTitleInstructions"
            | "sidebarAutoSettleOnMerge"
            | "sidebarAutoSettleAfterDays"
            | "sidebarAutoSettleKeepPullRequests"
            | "continueThreadsAfterServerUpdate"
            | "responseStreamingMode"
    )
}

use super::*;

#[derive(Clone)]
pub(super) enum Control {
    Switch,
    Select(&'static [(&'static str, &'static str)]),
    Number(f64, f64),
    Font(&'static str, f64, f64),
    Text,
}

#[derive(Clone)]
pub(super) struct Setting {
    pub page: &'static str,
    pub section: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub advanced: bool,
    pub default: Value,
    pub server: bool,
    pub control: Control,
}

pub(super) const PAGES: &[(&str, &str, &str)] = &[
    ("general", "General", ""),
    ("appearance", "Appearance", ""),
    ("chat", "Chat", ""),
    ("diffs", "Diffs & pull requests", ""),
    ("keybindings", "Keybindings", ""),
    ("voice", "Voice", ""),
    ("snap-shot", "SnapShots", ""),
    ("providers", "Providers", "Agents"),
    ("threads", "Threads", ""),
    ("browser", "Browser", ""),
    ("devices", "Devices", ""),
    ("scheduled-tasks", "Scheduled tasks", ""),
    ("projects", "Projects", "Code"),
    ("source-control", "Source control", ""),
    ("worktrees", "Worktrees", ""),
    ("connections", "Environments", "Connections"),
    ("access", "Sharing & access", ""),
    ("maintenance", "Maintenance", " "),
    ("labs", "Labs", ""),
    ("about", "About", ""),
    ("archived", "Archive", ""),
];

pub(super) fn settings() -> Vec<Setting> {
    use Control::*;
    let mut values = Vec::new();
    let mut add = |page, section, key, label, description, default, server, control| {
        values.push(Setting {
            advanced: matches!(
                key,
                "showSkillsInSlashMenu" | "browserDefaultAppearance" | "gpuiSeparateFonts"
            ) || section == "Advanced",
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
        "Order of projects in the sidebar project picker and command palette.",
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
        "Unpin confirmation",
        "Ask before unpinning a thread from the pinned section.",
        json!(false),
        false,
        Switch,
    );
    add(
        "general",
        "Confirmations",
        "confirmThreadArchive",
        "Archive confirmation",
        "Require a second click on the inline archive action before a thread is archived.",
        json!(false),
        false,
        Switch,
    );
    add(
        "general",
        "Confirmations",
        "confirmThreadDelete",
        "Delete confirmation",
        "Ask before deleting a thread and its chat history.",
        json!(true),
        false,
        Switch,
    );
    add(
        "general",
        "Confirmations",
        "confirmQuit",
        "Quit shortcut",
        "Hold mode also quits on two quick presses.",
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
        "Display",
        "timestampFormat",
        "Time format",
        "System default follows your browser or OS clock preference.",
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
        "Colors & themes",
        "gpuiAppearance",
        "Color scheme",
        "",
        json!("dark"),
        false,
        Select(&[("dark", "Dark"), ("light", "Light")]),
    );
    add(
        "appearance",
        "Typography",
        "fontSizeInterface",
        "Interface font",
        "Everything outside code blocks and the terminal.",
        json!(16),
        false,
        Font("fontFamilySans", 12., 20.),
    );
    add(
        "appearance",
        "Typography",
        "fontSizePrompt",
        "Prompt font",
        "Only the box you write prompts in. Mono works well here.",
        json!(14),
        false,
        Font("fontFamilyComposer", 12., 20.),
    );
    add(
        "appearance",
        "Typography",
        "fontSizeCode",
        "Monospace font",
        "Code blocks, diffs, file previews, and the terminal.",
        json!(13),
        false,
        Font("fontFamilyCode", 10., 18.),
    );
    add(
        "appearance",
        "Typography",
        "fontSizeTerminal",
        "Terminal font",
        "Terminal output, independent from code blocks and diffs.",
        json!(12),
        false,
        Font("fontFamilyTerminal", 8., 20.),
    );
    add(
        "appearance",
        "Typography",
        "wordWrap",
        "Word wrap",
        "Wrap long lines in code blocks, diffs, and file previews by default.",
        json!(true),
        false,
        Switch,
    );
    add(
        "appearance",
        "Typography",
        "gpuiSeparateFonts",
        "Separate fonts",
        "Set the prompt, code, and terminal fonts separately instead of one monospace font.",
        json!(false),
        false,
        Switch,
    );
    add(
        "appearance",
        "Interface",
        "chatWidth",
        "Chat width",
        "Set how wide messages and the composer can grow on large screens.",
        json!("comfortable"),
        false,
        Select(&[
            ("comfortable", "Comfortable"),
            ("wide", "Wide"),
            ("full", "Full"),
        ]),
    );
    add(
        "appearance",
        "Interface",
        "effortDisplay",
        "Effort display",
        "Show reasoning effort in the composer as its name, such as High, or as bars.",
        json!("label"),
        false,
        Select(&[("label", "Word"), ("bars", "Bars")]),
    );
    add(
        "appearance",
        "Interface",
        "mathRenderingEnabled",
        "Render math",
        r"Typeset LaTeX math, such as $x^2$ or \[x^2\], in messages and Markdown previews. The composer keeps TeX source editable.",
        json!(false),
        false,
        Switch,
    );
    add(
        "chat",
        "Composer",
        "composerRichTextEnabled",
        "Rich text composer",
        "Show formatted Markdown as you type.",
        json!(true),
        false,
        Switch,
    );
    add(
        "chat",
        "Composer",
        "sendShortcut",
        "Send shortcut",
        "Choose when Enter sends a prompt or inserts a new line",
        json!("enter"),
        false,
        Select(&[
            ("enter", "Enter"),
            ("mod-enter-multiline", "Ctrl + Enter for multiline prompts"),
            ("mod-enter", "Ctrl + Enter always"),
        ]),
    );
    add(
        "chat",
        "Composer",
        "showSkillsInSlashMenu",
        "Show skills in slash menu",
        "Also include skills in the / command menu. Skills always appear when you type $.",
        json!(true),
        false,
        Switch,
    );
    add(
        "chat",
        "Follow-ups",
        "followUpBehavior",
        "Follow-up behavior",
        "Queue follow-ups while the agent runs or steer the current run. Press Ctrl + Enter to do the opposite for one message.",
        json!("steer"),
        false,
        Select(&[("steer", "Steer"), ("queue", "Queue")]),
    );
    add(
        "chat",
        "Activity",
        "toolCallVisibility",
        "Tool calls in chat",
        "Hide the commands, edits, and searches agents run. Hide successful keeps failures. Zen shows only final answers. Approvals, questions, and errors always show.",
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
        "Activity",
        "screenReaderAnnouncements",
        "Screen reader announcements",
        "What a screen reader hears from the open thread: a turn starting, needing you, and ending. Status and messages adds what the agent writes before its last message. Everything adds each step.",
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
        "New projects",
        "addProjectBaseDirectory",
        "Add project starts in",
        "Leave empty to use \"~/\" when the Add Project browser opens.",
        json!(""),
        true,
        Text,
    );
    add(
        "threads",
        "New projects",
        "ghqCloneLayout",
        "Clone into host folders",
        "Clones go to github.com/owner/repo under the folder above, like ghq.",
        json!(false),
        true,
        Switch,
    );
    add(
        "threads",
        "Usage limits",
        "autoResumeLimitedThreads",
        "Resume after usage limits",
        "Automatically resume threads when their usage limits reset. You can cancel auto-resume in each thread.",
        json!(false),
        true,
        Switch,
    );
    add(
        "threads",
        "Usage limits",
        "snoozeLimitedThreads",
        "Snooze until limits reset",
        "Snooze threads until their usage limits reset. Enable auto-resume to continue the agent after the reset.",
        json!(false),
        true,
        Switch,
    );
    add(
        "threads",
        "Auto-settle",
        "sidebarAutoSettleOnMerge",
        "Auto-settle merged threads",
        "Settle a thread when its pull request merges. Closed pull requests still settle automatically.",
        json!(true),
        true,
        Switch,
    );
    add(
        "threads",
        "Auto-settle",
        "sidebarAutoSettleAfterDays",
        "Auto-settle inactive threads",
        "Sidebar threads with no activity for this long settle automatically.",
        json!(7),
        true,
        Switch,
    );
    add(
        "threads",
        "Auto-settle",
        "sidebarAutoSettleAfterDays",
        "Days of inactivity before auto-settle",
        "Any new activity un-settles a thread automatically.",
        json!(7),
        true,
        Number(1., 90.),
    );
    add(
        "threads",
        "Auto-settle",
        "sidebarAutoSettleKeepPullRequests",
        "Keep threads with a PR",
        "Inactivity does not settle threads with a pull request. Merge and close still do.",
        json!(false),
        true,
        Switch,
    );
    add(
        "threads",
        "Advanced",
        "idleCompaction",
        "Compact idle threads",
        "When to compact an idle Claude thread over 200,000 tokens: after you return, just before its prompt cache expires, or never. Each Claude provider can override this.",
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
        "Appended to the prompt for new and regenerated thread titles.",
        json!(""),
        true,
        Text,
    );
    add(
        "maintenance",
        "Updates",
        "enableProviderUpdateChecks",
        "Provider update checks",
        "Check installed provider CLIs for newer available versions and fetch model catalog updates. When off, new models may not appear.",
        json!(true),
        true,
        Switch,
    );
    add(
        "maintenance",
        "Updates",
        "continueThreadsAfterServerUpdate",
        "Continue threads after restarts",
        "Automatically resume interrupted threads after an update, crash, or machine restart on the selected environments.",
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
    add(
        "diffs",
        "Diffs",
        "diffLayout",
        "Diff layout",
        "Show diffs stacked or side by side. The toggle in the diff toolbar changes this too.",
        json!("stacked"),
        false,
        Select(&[("stacked", "Stacked"), ("split", "Split")]),
    );
    add(
        "diffs",
        "Diffs",
        "diffIgnoreWhitespace",
        "Hide whitespace changes",
        "Set whether the diff panel ignores whitespace-only edits by default.",
        json!(true),
        false,
        Switch,
    );
    add(
        "diffs",
        "Diffs",
        "diffFilesCollapsed",
        "Default diff file state",
        "Start with files expanded or collapsed when opening diffs or a pull request's Code tab.",
        json!(true),
        false,
        Select(&[("expanded", "Expanded"), ("collapsed", "Collapsed")]),
    );
    add(
        "diffs",
        "Diffs",
        "diffColorScheme",
        "Diff colors",
        "Choose colors for additions and deletions, including change counts.",
        json!("red-green"),
        false,
        Select(&[
            ("red-green", "Red & green"),
            ("blue-orange", "Blue & orange"),
        ]),
    );
    add(
        "diffs",
        "Pull requests",
        "proactivePanelsEnabled",
        "Open pull requests and changes panels proactively",
        "Open linked pull requests first. Otherwise, open Changes for edits to at least 3 files or 50 lines.",
        json!(false),
        false,
        Switch,
    );
    add(
        "diffs",
        "Pull requests",
        "pullRequestLinksOpenInApp",
        "Open pull requests in T3 Code",
        "Clicking a pull request link opens it in T3 Code. Turn off to open pull request links like any other link, on your git host.",
        json!(true),
        false,
        Switch,
    );
    add(
        "browser",
        "Agent access",
        "enableAgentBrowserAccess",
        "Agent browser access",
        "Allow agents to use the shared browser. Projects can override it.",
        json!(true),
        true,
        Switch,
    );
    add(
        "browser",
        "Browsing",
        "browserLinkTarget",
        "Open links in",
        "Where links in the chat and terminal open. Hold ⌘ or Ctrl while clicking a link to open it in your default browser either way.",
        json!("system"),
        false,
        Select(&[("system", "Your default browser"), ("app", "T3 Code")]),
    );
    add(
        "browser",
        "Browsing",
        "browserAutoShowFloatingPreview",
        "Auto-show floating preview",
        "Show the floating preview when an agent opens a browser or device unless the agent says otherwise.",
        json!(true),
        false,
        Switch,
    );
    add(
        "browser",
        "Browsing",
        "browserSearchEngine",
        "Search engine",
        "What the address bar searches when you type something that isn't a URL.",
        json!("duckduckgo"),
        false,
        Select(&[
            ("duckduckgo", "DuckDuckGo"),
            ("google", "Google"),
            ("bing", "Bing"),
            ("brave", "Brave Search"),
            ("kagi", "Kagi"),
            ("ecosia", "Ecosia"),
            ("custom", "Custom"),
        ]),
    );
    add(
        "browser",
        "Browsing",
        "browserDefaultAppearance",
        "Default browser appearance",
        "The color scheme pages are told to prefer. System follows your OS setting.",
        json!("system"),
        false,
        Select(&[("system", "System"), ("light", "Light"), ("dark", "Dark")]),
    );
    add(
        "browser",
        "Recordings",
        "browserRecordingFrameRate",
        "Browser recording frame rate",
        "Maximum recording rate. 30 fps saves CPU and storage; 60 fps is smoother.",
        json!(30),
        false,
        Select(&[("30", "30 fps"), ("60", "60 fps")]),
    );
    add(
        "browser",
        "Recordings",
        "browserRecordingShowKeyPresses",
        "Show key presses in recordings",
        "Show pressed keys and shortcuts in new recordings. Password fields are excluded.",
        json!(false),
        false,
        Switch,
    );
    add(
        "browser",
        "Recordings",
        "browserRecordingShowMousePresses",
        "Show mouse presses in recordings",
        "Highlight mouse presses and held buttons in new recordings.",
        json!(false),
        false,
        Switch,
    );
    add(
        "voice",
        "Input",
        "voiceShortcutMode",
        "Dictation shortcut",
        "How the dictation shortcut starts and stops recording.",
        json!("auto"),
        false,
        Select(&[
            ("auto", "Auto: press to toggle, hold to record"),
            ("hold", "Hold: record while held"),
            ("toggle", "Toggle: press to start, press again to stop"),
        ]),
    );
    add(
        "voice",
        "Transcription options",
        "speechSilenceFiltering",
        "Silence filtering",
        "Skip silence and other non-speech audio before transcription, keeping a short buffer so words aren’t cut off.",
        json!(true),
        false,
        Switch,
    );
    add(
        "voice",
        "Transcription options",
        "speechRemoveFillerWords",
        "Remove filler words",
        "Remove filler words like “um” and “uh” from transcripts.",
        json!(true),
        false,
        Switch,
    );
    add(
        "worktrees",
        "Creation",
        "worktreesDirectory",
        "Worktree location",
        r"Folder where new worktrees are created, on any drive, such as D:\worktrees or ~/worktrees. Existing worktrees stay where they are. Leave empty to use the T3 home folder.",
        json!(""),
        true,
        Text,
    );
    add(
        "worktrees",
        "Creation",
        "newWorktreesStartFromOrigin",
        "Start from remote",
        "Uses the latest commit of the base branch’s tracked remote branch, even if its name differs. Otherwise uses the same-named branch on origin. Uses your local branch if that remote branch is missing.",
        json!(true),
        true,
        Switch,
    );
    add(
        "worktrees",
        "Branches",
        "branchNamingMode",
        "Worktree branch naming",
        "Choose how new worktree branches are named from your first message.",
        json!("static"),
        true,
        Select(&[
            ("static", "Static prefix"),
            ("semantic", "Semantic prefix"),
            ("custom", "Custom instructions"),
        ]),
    );
    add(
        "worktrees",
        "Branches",
        "branchNamePrefix",
        "Branch prefix",
        "For example, t3 or t3/ produces t3/add-search. Leave empty for no prefix.",
        json!("t3"),
        true,
        Text,
    );
    add(
        "worktrees",
        "Branches",
        "branchNameInstructions",
        "Branch naming instructions",
        "Appended to the naming prompt. The model returns the complete branch name; no prefix or suffix is added.",
        json!(""),
        true,
        Text,
    );
    values
}

pub(super) fn project_scoped(key: &str) -> bool {
    matches!(
        key,
        "threadTitleInstructions"
            | "newWorktreesStartFromOrigin"
            | "branchNamingMode"
            | "branchNamePrefix"
            | "branchNameInstructions"
            | "enableAgentBrowserAccess"
            | "sidebarAutoSettleOnMerge"
            | "sidebarAutoSettleAfterDays"
            | "sidebarAutoSettleKeepPullRequests"
            | "continueThreadsAfterServerUpdate"
            | "responseStreamingMode"
    )
}

pub(super) fn page_icon(page: &str) -> gpui_kit::component::Icon {
    if page == "snap-shot" {
        return gpui_kit::component::Icon::default().data(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H6a3 3 0 0 0-3 3v2M16 3h2a3 3 0 0 1 3 3v2M21 16v2a3 3 0 0 1-3 3h-2M8 21H6a3 3 0 0 1-3-3v-2"/><rect width="10" height="8" x="7" y="8" rx="2"/><circle cx="12" cy="12" r="1.5"/></svg>"#);
    }
    use gpui_kit::assets::IconName::*;
    match page {
        "general" => Settings2,
        "appearance" => Palette,
        "chat" => MessageSquareText,
        "diffs" => FileDiff,
        "keybindings" => Keyboard,
        "voice" => Mic,
        "providers" => Bot,
        "threads" => MessagesSquare,
        "browser" => Globe,
        "devices" => Smartphone,
        "scheduled-tasks" => CalendarClock,
        "projects" => PanelsTopLeft,
        "source-control" => GitBranch,
        "worktrees" => FolderTree,
        "connections" => Server,
        "access" => ShieldCheck,
        "secrets" => KeyRound,
        "maintenance" | "diagnostics" => Wrench,
        "labs" => FlaskConical,
        "about" => Info,
        "archived" => Archive,
        _ => Settings2,
    }
    .into()
}

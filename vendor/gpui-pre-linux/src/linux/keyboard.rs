use gpui::{PlatformKeyboardLayout, SharedString};

#[derive(Clone)]
pub(crate) struct LinuxKeyboardLayout {
    name: SharedString,
}

impl PlatformKeyboardLayout for LinuxKeyboardLayout {
    fn id(&self) -> &str {
        &self.name
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl LinuxKeyboardLayout {
    pub(crate) fn new(name: SharedString) -> Self {
        Self { name }
    }
}

#[cfg(any(feature = "x11", feature = "wayland"))]
pub(crate) fn keyboard_input_context(
    state: &xkbcommon::xkb::State,
    modifiers: gpui::Modifiers,
    keycode: xkbcommon::xkb::Keycode,
    action: gpui::KeyboardInputAction,
) -> gpui::KeyboardInputContext {
    use xkbcommon::xkb;
    let keymap = state.get_keymap();
    let symbol = state.key_get_one_sym(keycode);
    let text =
        char::from_u32(xkb::keysym_to_utf32(symbol)).filter(|character| !character.is_control());
    let unshifted = keymap
        .key_get_syms_by_level(keycode, state.key_get_layout(keycode), 0)
        .first()
        .and_then(|symbol| char::from_u32(xkb::keysym_to_utf32(*symbol)))
        .filter(|character| !character.is_control());
    gpui::KeyboardInputContext {
        physical_key: physical_key(
            keymap.key_get_name(keycode).unwrap_or(""),
            u32::from(keycode),
        )
        .into(),
        text: text.map(|character| character.to_string().into()),
        unshifted_codepoint: text
            .and(unshifted.or(text))
            .map_or(0, |character| character as u32),
        modifiers,
        caps_lock: state.mod_name_is_active(xkb::MOD_NAME_CAPS, xkb::STATE_MODS_EFFECTIVE),
        num_lock: state.mod_name_is_active(xkb::MOD_NAME_NUM, xkb::STATE_MODS_EFFECTIVE),
        alt_graph: state.mod_name_is_active("Mod5", xkb::STATE_MODS_EFFECTIVE),
        is_modifier: symbol.is_modifier_key(),
        action,
    }
}

#[cfg(any(feature = "x11", feature = "wayland"))]
fn physical_key(name: &str, code: u32) -> &'static str {
    const FUNCTION: [&str; 25] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14",
        "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24", "F25",
    ];
    if let Some(function) = name
        .strip_prefix("FK")
        .and_then(|number| number.parse::<usize>().ok())
        && let Some(name) = function
            .checked_sub(1)
            .and_then(|index| FUNCTION.get(index))
    {
        return name;
    }
    match name {
        "TLDE" => "Backquote",
        "BKSL" => "Backslash",
        "LSGT" => "IntlBackslash",
        "AE01" => "Digit1",
        "AE02" => "Digit2",
        "AE03" => "Digit3",
        "AE04" => "Digit4",
        "AE05" => "Digit5",
        "AE06" => "Digit6",
        "AE07" => "Digit7",
        "AE08" => "Digit8",
        "AE09" => "Digit9",
        "AE10" => "Digit0",
        "AE11" => "Minus",
        "AE12" => "Equal",
        "AE13" => "IntlYen",
        "AD01" => "KeyQ",
        "AD02" => "KeyW",
        "AD03" => "KeyE",
        "AD04" => "KeyR",
        "AD05" => "KeyT",
        "AD06" => "KeyY",
        "AD07" => "KeyU",
        "AD08" => "KeyI",
        "AD09" => "KeyO",
        "AD10" => "KeyP",
        "AD11" => "BracketLeft",
        "AD12" => "BracketRight",
        "AC01" => "KeyA",
        "AC02" => "KeyS",
        "AC03" => "KeyD",
        "AC04" => "KeyF",
        "AC05" => "KeyG",
        "AC06" => "KeyH",
        "AC07" => "KeyJ",
        "AC08" => "KeyK",
        "AC09" => "KeyL",
        "AC10" => "Semicolon",
        "AC11" => "Quote",
        "AB01" => "KeyZ",
        "AB02" => "KeyX",
        "AB03" => "KeyC",
        "AB04" => "KeyV",
        "AB05" => "KeyB",
        "AB06" => "KeyN",
        "AB07" => "KeyM",
        "AB08" => "Comma",
        "AB09" => "Period",
        "AB10" => "Slash",
        "AB11" => "IntlRo",
        "BKSP" => "Backspace",
        "TAB" => "Tab",
        "RTRN" => "Enter",
        "ESC" => "Escape",
        "SPCE" => "Space",
        "CAPS" => "CapsLock",
        "LCTL" => "ControlLeft",
        "RCTL" => "ControlRight",
        "LFSH" => "ShiftLeft",
        "RTSH" => "ShiftRight",
        "LALT" => "AltLeft",
        "RALT" => "AltRight",
        "LWIN" => "MetaLeft",
        "RWIN" => "MetaRight",
        "MENU" | "COMP" => "ContextMenu",
        "INS" => "Insert",
        "DELE" => "Delete",
        "HOME" => "Home",
        "END" => "End",
        "PGUP" => "PageUp",
        "PGDN" => "PageDown",
        "LEFT" => "ArrowLeft",
        "RGHT" => "ArrowRight",
        "UP" => "ArrowUp",
        "DOWN" => "ArrowDown",
        "NMLK" => "NumLock",
        "KPDV" => "NumpadDivide",
        "KPMU" => "NumpadMultiply",
        "KPSU" => "NumpadSubtract",
        "KPAD" => "NumpadAdd",
        "KPEN" => "NumpadEnter",
        "KPDL" => "NumpadDecimal",
        "KPEQ" => "NumpadEqual",
        "KPPT" | "JPCM" => "NumpadComma",
        "KP0" => "Numpad0",
        "KP1" => "Numpad1",
        "KP2" => "Numpad2",
        "KP3" => "Numpad3",
        "KP4" => "Numpad4",
        "KP5" => "Numpad5",
        "KP6" => "Numpad6",
        "KP7" => "Numpad7",
        "KP8" => "Numpad8",
        "KP9" => "Numpad9",
        "PRSC" => "PrintScreen",
        "SCLK" => "ScrollLock",
        "PAUS" => "Pause",
        "HKTG" => "KanaMode",
        "HENK" => "Convert",
        "MUHE" => "NonConvert",
        _ => match code.saturating_sub(8) {
            85 => "Lang5",
            89 => "IntlRo",
            90 => "Lang3",
            91 => "Lang4",
            92 => "Convert",
            93 => "KanaMode",
            94 => "NonConvert",
            113 => "AudioVolumeMute",
            114 => "AudioVolumeDown",
            115 => "AudioVolumeUp",
            116 => "Power",
            117 => "NumpadEqual",
            121 => "NumpadComma",
            122 => "Lang1",
            123 => "Lang2",
            124 => "IntlYen",
            128 => "BrowserStop",
            129 => "Again",
            130 => "Props",
            131 => "Undo",
            132 => "Front",
            133 => "Copy",
            134 => "Open",
            135 => "Paste",
            136 => "Find",
            137 => "Cut",
            138 => "Help",
            140 => "LaunchApp2",
            142 => "Sleep",
            143 => "WakeUp",
            155 => "LaunchMail",
            156 => "BrowserFavorites",
            157 => "LaunchApp1",
            158 => "BrowserBack",
            159 => "BrowserForward",
            161 | 162 => "Eject",
            163 => "MediaTrackNext",
            164 => "MediaPlayPause",
            165 => "MediaTrackPrevious",
            166 => "MediaStop",
            172 => "BrowserHome",
            173 => "BrowserRefresh",
            217 => "BrowserSearch",
            226 => "MediaSelect",
            464 => "Fn",
            465 => "FnLock",
            _ => "Unidentified",
        },
    }
}

use std::sync::OnceLock;

use serde::Deserialize;

use crate::Key;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyboardEvent {
    pub code: String,
    pub key: String,
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
    pub mac: bool,
    pub repeat: bool,
    pub release: bool,
    pub composing: bool,
    pub layout_character: Option<String>,
}

fn single(value: &str) -> Option<char> {
    let mut chars = value.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

pub fn key_for_code(code: &str) -> i32 {
    static CODES: OnceLock<Vec<String>> = OnceLock::new();
    CODES
        .get_or_init(|| serde_json::from_str(include_str!("key_codes.json")).unwrap())
        .iter()
        .position(|name| name == code)
        .unwrap_or(0) as i32
}

impl KeyboardEvent {
    pub fn is_copy(&self) -> bool {
        if self.key.eq_ignore_ascii_case("insert") && !self.mac {
            return self.control && !self.shift && !self.meta;
        }
        self.key.eq_ignore_ascii_case("c") && if self.mac { self.meta } else { self.control }
    }

    pub fn is_paste(&self) -> bool {
        if self.key.eq_ignore_ascii_case("insert") && !self.mac {
            return self.shift && !self.control && !self.meta;
        }
        self.key.eq_ignore_ascii_case("v")
            && if self.mac {
                self.meta
            } else {
                self.control && self.shift
            }
    }

    pub fn local_data(&self) -> Option<&'static str> {
        if self.release || self.composing || self.shift {
            return None;
        }
        let left = self.key.eq_ignore_ascii_case("ArrowLeft");
        if left || self.key.eq_ignore_ascii_case("ArrowRight") {
            if self.alt && !self.meta && !self.control {
                return Some(if left { "\u{1b}b" } else { "\u{1b}f" });
            }
            if self.mac && self.meta && !self.alt && !self.control {
                return Some(if left { "\u{1}" } else { "\u{5}" });
            }
        }
        if self.mac
            && self.meta
            && !self.control
            && !self.alt
            && self.key.eq_ignore_ascii_case("Backspace")
        {
            return Some("\u{15}");
        }
        if !self.alt
            && ((self.key.eq_ignore_ascii_case("l") && self.control && !self.meta)
                || (self.mac && self.key.eq_ignore_ascii_case("k") && self.meta && !self.control))
        {
            return Some("\u{c}");
        }
        None
    }

    pub fn encoded(&self) -> Key {
        let key = key_for_code(&self.code);
        Key {
            key: if key == 0 {
                key_for_code(&self.key)
            } else {
                key
            },
            mods: i32::from(self.shift)
                | (i32::from(self.control) << 1)
                | (i32::from(self.alt) << 2)
                | (i32::from(self.meta) << 3)
                | (i32::from(self.caps_lock) << 4)
                | (i32::from(self.num_lock) << 5),
            consumed_mods: if self.control
                || self.meta
                || single(&self.key).is_none()
                || (self.alt && !self.mac)
            {
                0
            } else {
                i32::from(self.shift) | (i32::from(self.alt) << 2)
            },
            unshifted_codepoint: self.unshifted_codepoint(),
            text: if self.key.encode_utf16().count() == 1 {
                self.key.clone()
            } else {
                String::new()
            },
            action: if self.release {
                0
            } else if self.repeat {
                2
            } else {
                1
            },
            composing: self.composing,
        }
    }

    fn unshifted_codepoint(&self) -> u32 {
        let Some(character) = single(&self.key) else {
            return 0;
        };
        if let Some(character) = self.layout_character.as_deref().and_then(single) {
            return character as u32;
        }
        if character.is_ascii_uppercase() {
            return character.to_ascii_lowercase() as u32;
        }
        if self.shift {
            let unshifted = match character {
                '!' => '1',
                '@' => '2',
                '#' => '3',
                '$' => '4',
                '%' => '5',
                '^' => '6',
                '&' => '7',
                '*' => '8',
                '(' => '9',
                ')' => '0',
                '~' => '`',
                '_' => '-',
                '+' => '=',
                '{' => '[',
                '}' => ']',
                '|' => '\\',
                ':' => ';',
                '"' => '\'',
                '<' => ',',
                '>' => '.',
                '?' => '/',
                _ => {
                    let lower = character.to_lowercase().collect::<String>();
                    return single(&lower)
                        .filter(|lower| *lower != character)
                        .map_or(0, |lower| lower as u32);
                }
            };
            return unshifted as u32;
        }
        character as u32
    }
}

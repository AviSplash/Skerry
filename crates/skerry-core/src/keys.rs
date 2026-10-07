//! Platform-neutral keyboard handling.
//!
//! Keys travel over the wire as Linux evdev key codes (`linux/input-event-codes.h`).
//! They describe physical key positions, so the receiving computer applies its
//! own keyboard layout. Each platform backend maps its native codes to and
//! from evdev.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

use crate::geometry::Edge;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsKind {
    Windows,
    Macos,
    Linux,
    Other,
}

impl OsKind {
    pub fn current() -> OsKind {
        if cfg!(target_os = "windows") {
            OsKind::Windows
        } else if cfg!(target_os = "macos") {
            OsKind::Macos
        } else if cfg!(target_os = "linux") {
            OsKind::Linux
        } else {
            OsKind::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OsKind::Windows => "Windows",
            OsKind::Macos => "macOS",
            OsKind::Linux => "Linux",
            OsKind::Other => "Other",
        }
    }
}

pub mod code {
    pub const ESC: u32 = 1;
    pub const KEY_1: u32 = 2;
    pub const KEY_0: u32 = 11;
    pub const MINUS: u32 = 12;
    pub const EQUAL: u32 = 13;
    pub const BACKSPACE: u32 = 14;
    pub const TAB: u32 = 15;
    pub const Q: u32 = 16;
    pub const W: u32 = 17;
    pub const E: u32 = 18;
    pub const R: u32 = 19;
    pub const T: u32 = 20;
    pub const Y: u32 = 21;
    pub const U: u32 = 22;
    pub const I: u32 = 23;
    pub const O: u32 = 24;
    pub const P: u32 = 25;
    pub const LEFTBRACE: u32 = 26;
    pub const RIGHTBRACE: u32 = 27;
    pub const ENTER: u32 = 28;
    pub const LEFTCTRL: u32 = 29;
    pub const A: u32 = 30;
    pub const S: u32 = 31;
    pub const D: u32 = 32;
    pub const F: u32 = 33;
    pub const G: u32 = 34;
    pub const H: u32 = 35;
    pub const J: u32 = 36;
    pub const K: u32 = 37;
    pub const L: u32 = 38;
    pub const SEMICOLON: u32 = 39;
    pub const APOSTROPHE: u32 = 40;
    pub const GRAVE: u32 = 41;
    pub const LEFTSHIFT: u32 = 42;
    pub const BACKSLASH: u32 = 43;
    pub const Z: u32 = 44;
    pub const X: u32 = 45;
    pub const C: u32 = 46;
    pub const V: u32 = 47;
    pub const B: u32 = 48;
    pub const N: u32 = 49;
    pub const M: u32 = 50;
    pub const COMMA: u32 = 51;
    pub const DOT: u32 = 52;
    pub const SLASH: u32 = 53;
    pub const RIGHTSHIFT: u32 = 54;
    pub const KPASTERISK: u32 = 55;
    pub const LEFTALT: u32 = 56;
    pub const SPACE: u32 = 57;
    pub const CAPSLOCK: u32 = 58;
    pub const F1: u32 = 59;
    pub const F2: u32 = 60;
    pub const F3: u32 = 61;
    pub const F4: u32 = 62;
    pub const F5: u32 = 63;
    pub const F6: u32 = 64;
    pub const F7: u32 = 65;
    pub const F8: u32 = 66;
    pub const F9: u32 = 67;
    pub const F10: u32 = 68;
    pub const NUMLOCK: u32 = 69;
    pub const SCROLLLOCK: u32 = 70;
    pub const KP7: u32 = 71;
    pub const KP8: u32 = 72;
    pub const KP9: u32 = 73;
    pub const KPMINUS: u32 = 74;
    pub const KP4: u32 = 75;
    pub const KP5: u32 = 76;
    pub const KP6: u32 = 77;
    pub const KPPLUS: u32 = 78;
    pub const KP1: u32 = 79;
    pub const KP2: u32 = 80;
    pub const KP3: u32 = 81;
    pub const KP0: u32 = 82;
    pub const KPDOT: u32 = 83;
    pub const ZENKAKUHANKAKU: u32 = 85;
    pub const KEY_102ND: u32 = 86;
    pub const F11: u32 = 87;
    pub const F12: u32 = 88;
    pub const RO: u32 = 89;
    pub const KATAKANA: u32 = 90;
    pub const HIRAGANA: u32 = 91;
    pub const HENKAN: u32 = 92;
    pub const KATAKANAHIRAGANA: u32 = 93;
    pub const MUHENKAN: u32 = 94;
    pub const KPJPCOMMA: u32 = 95;
    pub const KPENTER: u32 = 96;
    pub const RIGHTCTRL: u32 = 97;
    pub const KPSLASH: u32 = 98;
    pub const SYSRQ: u32 = 99;
    pub const RIGHTALT: u32 = 100;
    pub const HOME: u32 = 102;
    pub const UP: u32 = 103;
    pub const PAGEUP: u32 = 104;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const END: u32 = 107;
    pub const DOWN: u32 = 108;
    pub const PAGEDOWN: u32 = 109;
    pub const INSERT: u32 = 110;
    pub const DELETE: u32 = 111;
    pub const MUTE: u32 = 113;
    pub const VOLUMEDOWN: u32 = 114;
    pub const VOLUMEUP: u32 = 115;
    pub const POWER: u32 = 116;
    pub const KPEQUAL: u32 = 117;
    pub const PAUSE: u32 = 119;
    pub const KPCOMMA: u32 = 121;
    pub const HANGEUL: u32 = 122;
    pub const HANJA: u32 = 123;
    pub const YEN: u32 = 124;
    pub const LEFTMETA: u32 = 125;
    pub const RIGHTMETA: u32 = 126;
    pub const COMPOSE: u32 = 127;
    pub const STOP: u32 = 128;
    pub const HELP: u32 = 138;
    pub const MENU: u32 = 139;
    pub const CALC: u32 = 140;
    pub const SLEEP: u32 = 142;
    pub const MAIL: u32 = 155;
    pub const BOOKMARKS: u32 = 156;
    pub const COMPUTER: u32 = 157;
    pub const BACK: u32 = 158;
    pub const FORWARD: u32 = 159;
    pub const EJECTCD: u32 = 161;
    pub const NEXTSONG: u32 = 163;
    pub const PLAYPAUSE: u32 = 164;
    pub const PREVIOUSSONG: u32 = 165;
    pub const STOPCD: u32 = 166;
    pub const HOMEPAGE: u32 = 172;
    pub const REFRESH: u32 = 173;
    pub const F13: u32 = 183;
    pub const F14: u32 = 184;
    pub const F15: u32 = 185;
    pub const F16: u32 = 186;
    pub const F17: u32 = 187;
    pub const F18: u32 = 188;
    pub const F19: u32 = 189;
    pub const F20: u32 = 190;
    pub const F21: u32 = 191;
    pub const F22: u32 = 192;
    pub const F23: u32 = 193;
    pub const F24: u32 = 194;
    pub const SEARCH: u32 = 217;
    pub const BRIGHTNESSDOWN: u32 = 224;
    pub const BRIGHTNESSUP: u32 = 225;
}

/// Modifier groups, ignoring left/right.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Mods(pub u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CTRL: Mods = Mods(1);
    pub const ALT: Mods = Mods(2);
    pub const SHIFT: Mods = Mods(4);
    pub const META: Mods = Mods(8);

    pub fn of_key(code: u32) -> Mods {
        match code {
            code::LEFTCTRL | code::RIGHTCTRL => Mods::CTRL,
            code::LEFTALT | code::RIGHTALT => Mods::ALT,
            code::LEFTSHIFT | code::RIGHTSHIFT => Mods::SHIFT,
            code::LEFTMETA | code::RIGHTMETA => Mods::META,
            _ => Mods::NONE,
        }
    }
    pub fn contains(self, o: Mods) -> bool {
        self.0 & o.0 == o.0
    }
    pub fn union(self, o: Mods) -> Mods {
        Mods(self.0 | o.0)
    }
}

pub fn is_modifier(code: u32) -> bool {
    Mods::of_key(code) != Mods::NONE
}

/// Translate a key for a different operating system. With `swap_cmd_ctrl`,
/// Command on a Mac becomes Control on Windows/Linux and vice versa, so
/// Cmd+C on the Mac keyboard copies on the PC.
pub fn translate(code: u32, from: OsKind, to: OsKind, swap_cmd_ctrl: bool) -> u32 {
    if !swap_cmd_ctrl || (from == OsKind::Macos) == (to == OsKind::Macos) {
        return code;
    }
    match code {
        code::LEFTMETA => code::LEFTCTRL,
        code::LEFTCTRL => code::LEFTMETA,
        code::RIGHTMETA => code::RIGHTCTRL,
        code::RIGHTCTRL => code::RIGHTMETA,
        other => other,
    }
}

const NAMES: &[(&str, u32)] = &[
    ("escape", code::ESC),
    ("esc", code::ESC),
    ("backspace", code::BACKSPACE),
    ("tab", code::TAB),
    ("enter", code::ENTER),
    ("return", code::ENTER),
    ("space", code::SPACE),
    ("home", code::HOME),
    ("end", code::END),
    ("pageup", code::PAGEUP),
    ("pagedown", code::PAGEDOWN),
    ("insert", code::INSERT),
    ("delete", code::DELETE),
    ("left", code::LEFT),
    ("right", code::RIGHT),
    ("up", code::UP),
    ("down", code::DOWN),
    ("scrolllock", code::SCROLLLOCK),
    ("pause", code::PAUSE),
    ("f1", code::F1),
    ("f2", code::F2),
    ("f3", code::F3),
    ("f4", code::F4),
    ("f5", code::F5),
    ("f6", code::F6),
    ("f7", code::F7),
    ("f8", code::F8),
    ("f9", code::F9),
    ("f10", code::F10),
    ("f11", code::F11),
    ("f12", code::F12),
    ("a", code::A),
    ("b", code::B),
    ("c", code::C),
    ("d", code::D),
    ("e", code::E),
    ("f", code::F),
    ("g", code::G),
    ("h", code::H),
    ("i", code::I),
    ("j", code::J),
    ("k", code::K),
    ("l", code::L),
    ("m", code::M),
    ("n", code::N),
    ("o", code::O),
    ("p", code::P),
    ("q", code::Q),
    ("r", code::R),
    ("s", code::S),
    ("t", code::T),
    ("u", code::U),
    ("v", code::V),
    ("w", code::W),
    ("x", code::X),
    ("y", code::Y),
    ("z", code::Z),
    ("1", code::KEY_1),
    ("2", code::KEY_1 + 1),
    ("3", code::KEY_1 + 2),
    ("4", code::KEY_1 + 3),
    ("5", code::KEY_1 + 4),
    ("6", code::KEY_1 + 5),
    ("7", code::KEY_1 + 6),
    ("8", code::KEY_1 + 7),
    ("9", code::KEY_1 + 8),
    ("0", code::KEY_0),
];

pub fn key_from_name(name: &str) -> Option<u32> {
    let n = name.trim().to_ascii_lowercase();
    NAMES.iter().find(|(k, _)| *k == n).map(|(_, c)| *c)
}

pub fn key_name(code: u32) -> String {
    NAMES.iter().find(|(_, c)| *c == code).map(|(k, _)| k.to_string()).unwrap_or_else(|| format!("key{code}"))
}

/// A key combination such as `ctrl+alt+shift+right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub mods: Mods,
    pub key: u32,
}

impl Hotkey {
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut mods = Mods::NONE;
        let mut key = None;
        for part in s.split('+') {
            let p = part.trim().to_ascii_lowercase();
            let m = match p.as_str() {
                "ctrl" | "control" => Mods::CTRL,
                "alt" | "option" | "opt" => Mods::ALT,
                "shift" => Mods::SHIFT,
                "meta" | "super" | "win" | "cmd" | "command" => Mods::META,
                _ => Mods::NONE,
            };
            if m != Mods::NONE {
                mods = mods.union(m);
            } else if key.is_none() {
                key = Some(key_from_name(&p)?);
            } else {
                return None;
            }
        }
        Some(Hotkey { mods, key: key? })
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.mods.contains(Mods::CTRL) {
            parts.push("ctrl".to_string());
        }
        if self.mods.contains(Mods::ALT) {
            parts.push("alt".to_string());
        }
        if self.mods.contains(Mods::SHIFT) {
            parts.push("shift".to_string());
        }
        if self.mods.contains(Mods::META) {
            parts.push("meta".to_string());
        }
        parts.push(key_name(self.key));
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    /// Jump to the computer on this side of the one currently in focus.
    Switch(Edge),
    /// Bring the cursor back to this computer.
    ReturnHome,
}

impl HotkeyAction {
    pub fn parse(s: &str) -> Option<HotkeyAction> {
        match s.trim().to_ascii_lowercase().as_str() {
            "home" | "return" | "return_home" => Some(HotkeyAction::ReturnHome),
            other => Edge::parse(other).map(HotkeyAction::Switch),
        }
    }
    pub fn name(&self) -> String {
        match self {
            HotkeyAction::Switch(e) => e.name().to_string(),
            HotkeyAction::ReturnHome => "home".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyVerdict {
    /// Let the key through (forward it, or deliver it locally).
    Pass,
    /// Part of a hotkey: hide it from both computers.
    Swallow,
    /// A hotkey fired: hide the key and run the action.
    Fire(HotkeyAction),
}

/// Tracks modifier state and recognises hotkeys from a stream of key events.
/// Capture backends run one of these on every key they see.
#[derive(Debug, Default, Clone)]
pub struct HotkeyMatcher {
    bindings: Vec<(Hotkey, HotkeyAction)>,
    held_mods: HashSet<u32>,
    swallowed: HashSet<u32>,
}

impl HotkeyMatcher {
    pub fn new(bindings: Vec<(Hotkey, HotkeyAction)>) -> Self {
        HotkeyMatcher { bindings, ..Default::default() }
    }

    pub fn set_bindings(&mut self, bindings: Vec<(Hotkey, HotkeyAction)>) {
        self.bindings = bindings;
    }

    pub fn mods(&self) -> Mods {
        self.held_mods.iter().fold(Mods::NONE, |m, c| m.union(Mods::of_key(*c)))
    }

    pub fn on_key(&mut self, code: u32, pressed: bool) -> KeyVerdict {
        self.on_key_with_mods(code, pressed, None)
    }

    /// Like [`on_key`](Self::on_key), but with the modifier state as the OS
    /// reports it. Tracked state can go stale when the OS swallows a key-up
    /// (for example Win+L or Ctrl+Alt+Del), which would stop hotkeys from
    /// matching; the OS state is always right.
    pub fn on_key_with_mods(&mut self, code: u32, pressed: bool, os_mods: Option<Mods>) -> KeyVerdict {
        if is_modifier(code) {
            if pressed {
                self.held_mods.insert(code);
            } else {
                self.held_mods.remove(&code);
            }
            return KeyVerdict::Pass;
        }
        if !pressed {
            return if self.swallowed.remove(&code) { KeyVerdict::Swallow } else { KeyVerdict::Pass };
        }
        if self.swallowed.contains(&code) {
            // Auto-repeat of a hotkey key.
            return KeyVerdict::Swallow;
        }
        let mods = os_mods.unwrap_or_else(|| self.mods());
        if let Some((_, action)) = self.bindings.iter().find(|(hk, _)| hk.key == code && hk.mods == mods) {
            self.swallowed.insert(code);
            return KeyVerdict::Fire(*action);
        }
        KeyVerdict::Pass
    }

    /// Forget held keys, e.g. after the keyboard focus moved elsewhere.
    pub fn reset(&mut self) {
        self.held_mods.clear();
        self.swallowed.clear();
    }
}

pub fn default_hotkeys() -> Vec<(String, String)> {
    vec![
        ("ctrl+alt+shift+left".into(), "left".into()),
        ("ctrl+alt+shift+right".into(), "right".into()),
        ("ctrl+alt+shift+up".into(), "top".into()),
        ("ctrl+alt+shift+down".into(), "bottom".into()),
        ("ctrl+alt+shift+escape".into(), "home".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print_hotkey() {
        let hk = Hotkey::parse("Ctrl+Alt+Shift+Right").unwrap();
        assert_eq!(hk.key, code::RIGHT);
        assert_eq!(hk.mods, Mods(1 | 2 | 4));
        assert_eq!(hk.to_string(), "ctrl+alt+shift+right");
        assert!(Hotkey::parse("ctrl+nonsense").is_none());
        assert!(Hotkey::parse("ctrl+a+b").is_none());
    }

    #[test]
    fn matcher_fires_and_swallows() {
        let mut m = HotkeyMatcher::new(vec![(
            Hotkey::parse("ctrl+alt+shift+right").unwrap(),
            HotkeyAction::Switch(Edge::Right),
        )]);
        assert_eq!(m.on_key(code::LEFTCTRL, true), KeyVerdict::Pass);
        assert_eq!(m.on_key(code::LEFTALT, true), KeyVerdict::Pass);
        assert_eq!(m.on_key(code::RIGHT, true), KeyVerdict::Pass, "missing shift");
        assert_eq!(m.on_key(code::RIGHT, false), KeyVerdict::Pass);
        assert_eq!(m.on_key(code::RIGHTSHIFT, true), KeyVerdict::Pass);
        assert_eq!(m.on_key(code::RIGHT, true), KeyVerdict::Fire(HotkeyAction::Switch(Edge::Right)));
        assert_eq!(m.on_key(code::RIGHT, true), KeyVerdict::Swallow, "auto-repeat");
        assert_eq!(m.on_key(code::RIGHT, false), KeyVerdict::Swallow);
        assert_eq!(m.on_key(code::A, true), KeyVerdict::Pass);
    }

    #[test]
    fn os_reported_mods_override_stale_state() {
        let mut m = HotkeyMatcher::new(vec![(
            Hotkey::parse("ctrl+alt+shift+right").unwrap(),
            HotkeyAction::Switch(Edge::Right),
        )]);
        // A Win key-up was lost (e.g. Win+L): tracked state wrongly includes META.
        m.on_key(code::LEFTMETA, true);
        for k in [code::LEFTCTRL, code::LEFTALT, code::LEFTSHIFT] {
            m.on_key(k, true);
        }
        assert_eq!(m.on_key(code::RIGHT, true), KeyVerdict::Pass, "stale tracked state blocks the hotkey");
        m.on_key(code::RIGHT, false);
        let actual = Mods::CTRL.union(Mods::ALT).union(Mods::SHIFT);
        assert_eq!(
            m.on_key_with_mods(code::RIGHT, true, Some(actual)),
            KeyVerdict::Fire(HotkeyAction::Switch(Edge::Right))
        );
    }

    #[test]
    fn cmd_ctrl_swap_only_across_mac_boundary() {
        use OsKind::*;
        assert_eq!(translate(code::LEFTMETA, Macos, Windows, true), code::LEFTCTRL);
        assert_eq!(translate(code::LEFTCTRL, Windows, Macos, true), code::LEFTMETA);
        assert_eq!(translate(code::LEFTCTRL, Windows, Linux, true), code::LEFTCTRL);
        assert_eq!(translate(code::LEFTMETA, Macos, Macos, true), code::LEFTMETA);
        assert_eq!(translate(code::LEFTMETA, Macos, Windows, false), code::LEFTMETA);
        assert_eq!(translate(code::A, Macos, Windows, true), code::A);
    }
}

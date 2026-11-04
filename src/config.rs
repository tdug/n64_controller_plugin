//! Persistent configuration (JSON).

use crate::input::{DeviceSel, Input, PadAxis, PadButton};
use crate::n64::N64Button;
use crate::stick::StickSettings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PORTS: usize = 4;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PakType {
    None,
    /// Controller Pak, saved by this plugin.
    Memory,
    Rumble,
    /// Memory + rumble in one pak (like the Tremor Pak / Dual Force Pak combos).
    Tremor,
    /// Let the emulator handle its own Controller Pak (non-raw mode).
    EmulatorMemory,
}

impl PakType {
    pub const ALL: [PakType; 5] = [
        PakType::None,
        PakType::Memory,
        PakType::Rumble,
        PakType::Tremor,
        PakType::EmulatorMemory,
    ];
    pub fn label(self) -> &'static str {
        match self {
            PakType::None => "None",
            PakType::Memory => "Controller Pak (memory)",
            PakType::Rumble => "Rumble Pak",
            PakType::Tremor => "Tremor Pak (memory + rumble)",
            PakType::EmulatorMemory => "Emulator's Controller Pak",
        }
    }
    pub fn has_memory(self) -> bool {
        matches!(self, PakType::Memory | PakType::Tremor)
    }
    pub fn has_rumble(self) -> bool {
        matches!(self, PakType::Rumble | PakType::Tremor)
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StickBindings {
    pub up: Vec<Input>,
    pub down: Vec<Input>,
    pub left: Vec<Input>,
    pub right: Vec<Input>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];
    pub fn label(self) -> &'static str {
        match self {
            Dir::Up => "Stick Up",
            Dir::Down => "Stick Down",
            Dir::Left => "Stick Left",
            Dir::Right => "Stick Right",
        }
    }
}

impl StickBindings {
    pub fn get_mut(&mut self, d: Dir) -> &mut Vec<Input> {
        match d {
            Dir::Up => &mut self.up,
            Dir::Down => &mut self.down,
            Dir::Left => &mut self.left,
            Dir::Right => &mut self.right,
        }
    }
    pub fn use_pad_stick(&mut self, right_stick: bool) {
        let (x, y) = if right_stick {
            (PadAxis::RightStickX, PadAxis::RightStickY)
        } else {
            (PadAxis::LeftStickX, PadAxis::LeftStickY)
        };
        for (d, inp) in [
            (Dir::Up, Input::axis(y, true)),
            (Dir::Down, Input::axis(y, false)),
            (Dir::Left, Input::axis(x, false)),
            (Dir::Right, Input::axis(x, true)),
        ] {
            let v = self.get_mut(d);
            v.retain(|i| !matches!(i, Input::Axis { .. }));
            v.push(inp);
        }
    }
}

/// While held (or toggled on), scales the analog stick — e.g. a "walk" button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Modifier {
    pub inputs: Vec<Input>,
    pub scale: f32,
    pub toggle: bool,
}

impl Default for Modifier {
    fn default() -> Self {
        Modifier {
            inputs: vec![],
            scale: 0.5,
            toggle: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MacroMode {
    /// Macro buttons are OR'd with live input; stick overridden only on steps that set it.
    Merge,
    /// Macro fully replaces live input while playing.
    Override,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Playback {
    /// Plays once per press.
    Once,
    /// Loops while the trigger is held.
    WhileHeld,
    /// Press to start looping, press again to stop.
    Toggle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MacroStep {
    pub buttons: u16,
    pub stick: Option<[i8; 2]>,
    /// How many polls (frames) to hold this step.
    pub frames: u32,
}

impl Default for MacroStep {
    fn default() -> Self {
        MacroStep {
            buttons: 0,
            stick: None,
            frames: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Macro {
    pub name: String,
    /// All of these must be held (a chord) to trigger.
    pub trigger: Vec<Input>,
    pub mode: MacroMode,
    pub playback: Playback,
    pub steps: Vec<MacroStep>,
}

impl Default for Macro {
    fn default() -> Self {
        Macro {
            name: "New macro".into(),
            trigger: vec![],
            mode: MacroMode::Merge,
            playback: Playback::Once,
            steps: vec![MacroStep::default()],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControllerConfig {
    pub plugged: bool,
    pub device: DeviceSel,
    pub pak: PakType,
    /// Pak swapped in by the pak-switch hotkey.
    pub alt_pak: PakType,
    pub pak_switch: Vec<Input>,
    pub buttons: BTreeMap<N64Button, Vec<Input>>,
    pub stick_bindings: StickBindings,
    pub stick: StickSettings,
    pub modifiers: Vec<Modifier>,
    pub macros: Vec<Macro>,
    /// 0..=1
    pub rumble_strength: f32,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self::defaults(1, false)
    }
}

impl ControllerConfig {
    pub fn defaults(port: usize, keyboard: bool) -> Self {
        use N64Button::*;
        use PadButton as P;
        let mut buttons = BTreeMap::new();
        let pad = [
            (A, vec![Input::button(P::South)]),
            (B, vec![Input::button(P::West)]),
            (Z, vec![Input::button(P::LeftTrigger)]),
            (Start, vec![Input::button(P::Start)]),
            (L, vec![Input::button(P::LeftShoulder)]),
            (
                R,
                vec![
                    Input::button(P::RightShoulder),
                    Input::button(P::RightTrigger),
                ],
            ),
            (DUp, vec![Input::button(P::DPadUp)]),
            (DDown, vec![Input::button(P::DPadDown)]),
            (DLeft, vec![Input::button(P::DPadLeft)]),
            (DRight, vec![Input::button(P::DPadRight)]),
            (CUp, vec![Input::axis(PadAxis::RightStickY, true)]),
            (CDown, vec![Input::axis(PadAxis::RightStickY, false)]),
            (CLeft, vec![Input::axis(PadAxis::RightStickX, false)]),
            (CRight, vec![Input::axis(PadAxis::RightStickX, true)]),
        ];
        for (b, v) in pad {
            buttons.insert(b, v);
        }
        let mut stick_bindings = StickBindings::default();
        stick_bindings.use_pad_stick(false);
        let mut modifiers = vec![];
        if keyboard {
            let keys: [(N64Button, u8); 14] = [
                (A, b'X'),
                (B, b'C'),
                (Z, b'Z'),
                (Start, 0x0D),
                (L, b'A'),
                (R, b'S'),
                (DUp, 0x68),
                (DDown, 0x62),
                (DLeft, 0x64),
                (DRight, 0x66),
                (CUp, b'I'),
                (CDown, b'K'),
                (CLeft, b'J'),
                (CRight, b'L'),
            ];
            for (b, k) in keys {
                buttons.entry(b).or_default().insert(0, Input::key(k));
            }
            for (d, k) in [
                (Dir::Up, 0x26),
                (Dir::Down, 0x28),
                (Dir::Left, 0x25),
                (Dir::Right, 0x27),
            ] {
                stick_bindings.get_mut(d).insert(0, Input::key(k));
            }
            modifiers.push(Modifier {
                inputs: vec![Input::key(0xA0)],
                ..Default::default()
            });
        }
        ControllerConfig {
            plugged: port == 0,
            device: DeviceSel::Auto,
            pak: PakType::Tremor,
            alt_pak: PakType::Memory,
            pak_switch: vec![],
            buttons,
            stick_bindings,
            stick: StickSettings::default(),
            modifiers,
            macros: vec![],
            rumble_strength: 0.8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub controllers: Vec<ControllerConfig>,
    /// Read the keyboard even when the emulator window is not focused.
    pub background_input: bool,
    /// Separate Controller Pak file per game (otherwise one per port).
    pub pak_per_game: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            controllers: (0..PORTS)
                .map(|i| ControllerConfig::defaults(i, i == 0))
                .collect(),
            background_input: false,
            pak_per_game: true,
        }
    }
}

impl Config {
    fn normalize(mut self) -> Self {
        self.controllers.truncate(PORTS);
        while self.controllers.len() < PORTS {
            let i = self.controllers.len();
            self.controllers.push(ControllerConfig::defaults(i, false));
        }
        self
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str::<Config>(s)
            .map(Config::normalize)
            .map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn load() -> Self {
        match storage::read() {
            Some(s) => Self::from_json(&s).unwrap_or_else(|e| {
                crate::platform::log(&format!("config parse error, using defaults: {e}"));
                Config::default()
            }),
            None => Config::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        storage::write(&self.to_json())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod storage {
    use std::path::PathBuf;

    pub fn path() -> PathBuf {
        crate::platform::config_dir().join("config.json")
    }

    pub fn read() -> Option<String> {
        std::fs::read_to_string(path()).ok()
    }

    pub fn write(s: &str) -> Result<(), String> {
        let p = path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, s).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    }

    pub fn mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(path()).and_then(|m| m.modified()).ok()
    }
}

#[cfg(target_arch = "wasm32")]
pub mod storage {
    const KEY: &str = "tdug-n64-config";

    fn local() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    pub fn read() -> Option<String> {
        local()?.get_item(KEY).ok()?
    }

    pub fn write(s: &str) -> Result<(), String> {
        local()
            .ok_or("no localStorage")?
            .set_item(KEY, s)
            .map_err(|e| format!("{e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = Config::default();
        let back = Config::from_json(&c.to_json()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let c = Config::from_json(r#"{"background_input": true}"#).unwrap();
        assert!(c.background_input);
        assert_eq!(c.controllers.len(), PORTS);
    }
}

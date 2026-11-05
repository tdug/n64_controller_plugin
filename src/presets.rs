//! Built-in macro presets and controller mapping presets.

use crate::config::{ControllerConfig, Macro, MacroMode, MacroStep, Playback, StickBindings};
use crate::input::{DeviceState, Input};
use crate::n64::N64Button;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn step(buttons: &[N64Button], stick: Option<[i8; 2]>, frames: u32) -> MacroStep {
    MacroStep {
        buttons: buttons.iter().fold(0, |m, b| m | b.mask()),
        stick,
        frames,
    }
}

/// Built-in macro presets (no trigger bound).
pub fn builtin_macros() -> Vec<Macro> {
    use N64Button::*;
    let fwd = Some([0, 80]);
    // Quick spin: one full stick rotation (one step per game frame), then B.
    let r = 80i8;
    let d = 57i8; // 80 / √2
    let circle = [
        [0, -r],
        [-d, -d],
        [-r, 0],
        [-d, d],
        [0, r],
        [d, d],
        [r, 0],
        [d, -d],
    ];
    let mut spin: Vec<MacroStep> = circle.iter().map(|p| step(&[], Some(*p), 1)).collect();
    spin.push(step(&[B], Some([0, -r]), 2));
    vec![
        Macro {
            name: "SM64: Min Distance Triple Jump".into(),
            notes: "Builds just enough forward speed for a triple jump, then presses A on each \
                    landing. One frame = one game frame (SM64 runs at 30 fps). Timings are a \
                    starting point: if a jump is missed, nudge the airborne step lengths by ±1."
                .into(),
            trigger: vec![],
            mode: MacroMode::Override,
            playback: Playback::Once,
            steps: vec![
                step(&[], fwd, 12),
                step(&[A], fwd, 1),
                step(&[], fwd, 12),
                step(&[A], fwd, 1),
                step(&[], fwd, 14),
                step(&[A], fwd, 2),
                step(&[], fwd, 4),
            ],
        },
        Macro {
            name: "OoT/MM: Quick Spin".into(),
            notes: "Rotates the stick a full circle, one direction per game frame, then presses B \
                    for a quick spin attack. Works in Ocarina of Time and Majora's Mask."
                .into(),
            trigger: vec![],
            mode: MacroMode::Override,
            playback: Playback::Once,
            steps: spin,
        },
    ]
}

/// Built-in controller presets (`presets/controllers.yml`).
pub const BUILTIN_CONTROLLERS_YAML: &str = include_str!("../presets/controllers.yml");

/// A host-controller mapping preset, loaded from YAML.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ControllerPreset {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Any inner list whose substrings all appear in the device name (case-insensitive).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<Vec<String>>,
    /// "vvvv:pppp" hex vendor:product ids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// web / windows / macos / linux; empty = all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// Controllers with several modes: presets sharing a `family` are offered as a mode switch.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub family: String,
    /// Mode label within the family (e.g. "D", "S").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    /// Where these bindings come from: "gamepad" (default, OS gamepad layer) or "hid" (raw HID).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadzone: Option<f32>,
    /// Mechanism per direction group; inferred from the bindings when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<crate::config::GroupModes>,
    pub buttons: BTreeMap<N64Button, Vec<Input>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stick: Option<StickBindings>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PresetFile {
    pub presets: Vec<ControllerPreset>,
}

pub fn current_platform() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "web"
    } else if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

pub fn parse_controller_presets(yaml: &str) -> Result<Vec<ControllerPreset>, String> {
    serde_yaml_ng::from_str::<PresetFile>(yaml)
        .map(|f| f.presets)
        .map_err(|e| e.to_string())
}

/// Path of the user-editable presets file (desktop).
#[cfg(not(target_arch = "wasm32"))]
pub fn user_presets_path() -> std::path::PathBuf {
    crate::platform::config_dir().join("controllers.yml")
}

/// Presets for this platform: the user's file if present (desktop), else the built-in one.
/// Returns the presets and an error message if the user's file failed to parse.
pub fn load_controller_presets() -> (Vec<ControllerPreset>, Option<String>) {
    #[cfg(not(target_arch = "wasm32"))]
    let (yaml, err_prefix) = match std::fs::read_to_string(user_presets_path()) {
        Ok(s) => (s, "controllers.yml"),
        Err(_) => (BUILTIN_CONTROLLERS_YAML.to_string(), "built-in presets"),
    };
    #[cfg(target_arch = "wasm32")]
    let (yaml, err_prefix) = (BUILTIN_CONTROLLERS_YAML.to_string(), "built-in presets");
    let platform = current_platform();
    let keep =
        |p: &ControllerPreset| p.platforms.is_empty() || p.platforms.iter().any(|x| x == platform);
    match parse_controller_presets(&yaml) {
        Ok(v) => (v.into_iter().filter(keep).collect(), None),
        Err(e) => {
            let fallback = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap_or_default();
            (
                fallback.into_iter().filter(keep).collect(),
                Some(format!("{err_prefix}: {e}")),
            )
        }
    }
}

impl ControllerPreset {
    pub fn is_hid(&self) -> bool {
        self.source == "hid"
    }

    pub fn matches(&self, dev: &DeviceState) -> bool {
        if self.is_hid() != dev.is_hid {
            return false;
        }
        let n = dev.name.to_lowercase();
        let by_name = self
            .names
            .iter()
            .any(|all| !all.is_empty() && all.iter().all(|s| n.contains(&s.to_lowercase())));
        let id = dev.vid_pid();
        let by_id = self.ids.iter().any(|x| {
            let x = x.to_lowercase();
            id.as_deref() == Some(x.as_str()) || {
                // Browsers put ids in the name: "Vendor: 057e Product: 2019"
                let mut parts = x.split(':');
                matches!((parts.next(), parts.next()), (Some(v), Some(p))
                    if n.contains(&format!("vendor: {v}")) && n.contains(&format!("product: {p}")))
            }
        });
        by_name || by_id
    }

    /// Replace gamepad bindings with this preset's (keyboard keys are kept).
    pub fn apply(&self, c: &mut ControllerConfig) {
        let keep_keys = |l: &mut Vec<Input>| l.retain(|i| matches!(i, Input::Key { .. }));
        for b in N64Button::ALL {
            let list = c.buttons.entry(b).or_default();
            keep_keys(list);
            if let Some(v) = self.buttons.get(&b) {
                list.extend(v.iter().cloned());
            }
        }
        if let Some(s) = &self.stick {
            for d in crate::config::Dir::ALL {
                let list = c.stick_bindings.get_mut(d);
                keep_keys(list);
                list.extend(s.clone().get_mut(d).iter().cloned());
            }
        }
        if let Some(dz) = self.deadzone {
            c.stick.deadzone = dz;
        }
        c.infer_group_modes();
        if let Some(m) = &self.modes {
            c.group_modes = m.clone();
        }
    }

    /// Build a preset from a controller's current gamepad bindings (for "Export").
    pub fn from_bindings(name: &str, c: &ControllerConfig, dev: Option<&DeviceState>) -> Self {
        let pad = |l: &Vec<Input>| -> Vec<Input> {
            l.iter()
                .filter(|i| !matches!(i, Input::Key { .. }))
                .cloned()
                .collect()
        };
        let buttons = c
            .buttons
            .iter()
            .map(|(b, l)| (*b, pad(l)))
            .filter(|(_, l)| !l.is_empty())
            .collect();
        let sb = &c.stick_bindings;
        let raw = buttons_use_raw(&buttons)
            || [&sb.up, &sb.down, &sb.left, &sb.right]
                .iter()
                .any(|l| l.iter().any(is_raw));
        ControllerPreset {
            name: name.to_string(),
            note: String::new(),
            family: String::new(),
            mode: String::new(),
            source: if dev.is_some_and(|d| d.is_hid) {
                "hid".into()
            } else {
                String::new()
            },
            names: dev
                .map(|d| vec![vec![d.name.to_lowercase()]])
                .unwrap_or_default(),
            ids: dev.and_then(|d| d.vid_pid()).into_iter().collect(),
            platforms: if raw {
                vec![current_platform().to_string()]
            } else {
                vec![]
            },
            deadzone: Some(c.stick.deadzone),
            modes: Some(c.group_modes.clone()),
            buttons,
            stick: Some(StickBindings {
                up: pad(&sb.up),
                down: pad(&sb.down),
                left: pad(&sb.left),
                right: pad(&sb.right),
            }),
        }
    }

    pub fn to_yaml(&self) -> String {
        let file = PresetFile {
            presets: vec![self.clone()],
        };
        let s = serde_yaml_ng::to_string(&file).unwrap_or_default();
        // Emit just the list item so it can be pasted under `presets:`.
        s.trim_start_matches("presets:\n").to_string()
    }
}

fn is_raw(i: &Input) -> bool {
    matches!(
        i,
        Input::RawButton { .. } | Input::RawAxis { .. } | Input::RawHat { .. }
    )
}

fn buttons_use_raw(b: &BTreeMap<N64Button, Vec<Input>>) -> bool {
    b.values().any(|l| l.iter().any(is_raw))
}

/// First preset that matches the device.
pub fn suggested_preset<'a>(
    presets: &'a [ControllerPreset],
    dev: &DeviceState,
) -> Option<&'a ControllerPreset> {
    presets.iter().find(|p| p.matches(dev))
}

/// All presets in the device's family (for the mode switch); empty if it has none.
pub fn family_of<'a>(
    presets: &'a [ControllerPreset],
    dev: &DeviceState,
) -> Vec<&'a ControllerPreset> {
    match suggested_preset(presets, dev) {
        Some(p) if !p.family.is_empty() => {
            presets.iter().filter(|x| x.family == p.family).collect()
        }
        _ => vec![],
    }
}

/// True for a Switch-protocol N64 pad (8BitDo 64 in S mode, or Nintendo's Switch Online N64
/// controller). Browsers leave these in Nintendo's basic HID mode, which they can't map.
pub fn is_switch_n64(dev: &DeviceState) -> bool {
    let n = dev.name.to_lowercase();
    dev.vid_pid().as_deref() == Some("057e:2019")
        || (n.contains("057e") && n.contains("2019"))
        || n.contains("nso n64")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str) -> DeviceState {
        DeviceState {
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn builtin_yaml_parses() {
        let p = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap();
        assert!(p.len() >= 2);
    }

    #[test]
    fn suggests_8bitdo() {
        let all = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap();
        assert!(suggested_preset(&all, &dev("8BitDo 64 BT")).is_some());
        assert!(
            suggested_preset(&all, &dev("N64 Controller (Vendor: 057e Product: 2019)")).is_some()
        );
        assert!(suggested_preset(&all, &dev("Xbox Wireless Controller")).is_none());
    }

    #[test]
    fn export_roundtrip() {
        let c = ControllerConfig::defaults(0, true);
        let p = ControllerPreset::from_bindings("Mine", &c, Some(&dev("Pad")));
        let yaml = format!("presets:\n{}", p.to_yaml());
        let back = parse_controller_presets(&yaml).unwrap();
        assert_eq!(back[0].buttons, p.buttons);
        // Applying keeps keyboard keys and restores pad bindings.
        let mut c2 = ControllerConfig::defaults(0, true);
        back[0].apply(&mut c2);
        assert_eq!(c2.buttons, c.buttons);
    }

    #[test]
    fn builtins_have_no_trigger() {
        assert!(
            builtin_macros()
                .iter()
                .all(|m| m.trigger.is_empty() && !m.steps.is_empty())
        );
    }

    // ---- Replays of real captures (macOS, gilrs) -------------------------------------
    // Each case is the input state seen while one physical control was held, and the N64
    // output it must produce: the expected button, or the stick direction (sign of x/y).

    use crate::config::ControllerConfig;
    use crate::input::{PadAxis, PadButton, Snapshot};
    use crate::runtime::evaluate_with;

    enum Ev {
        Btn(PadButton),
        Axis(PadAxis, f32),
        RawAxis(u32, f32),
        RawBtn(u32),
    }

    #[derive(Debug, PartialEq)]
    enum Out {
        Button(N64Button),
        Stick(i8, i8), // signs
        Nothing,
    }

    fn run(preset: &ControllerPreset, name: &str, vid: u16, pid: u16, evs: &[Ev]) -> (u16, i8, i8) {
        let mut c = ControllerConfig::defaults(0, true);
        preset.apply(&mut c);
        let mut d = DeviceState {
            name: name.into(),
            vendor: Some(vid),
            product: Some(pid),
            ..Default::default()
        };
        for e in evs {
            match e {
                Ev::Btn(b) => d.buttons[*b as usize] = 1.0,
                Ev::Axis(a, v) => d.axes[*a as usize] = *v,
                Ev::RawAxis(code, v) => d.raw_axes.push((*code, *v)),
                Ev::RawBtn(code) => d.raw_buttons.push((*code, 1.0)),
            }
        }
        let snap = Snapshot {
            devices: vec![d.clone()],
            ..Default::default()
        };
        let (st, _) = evaluate_with(&c, &snap, Some(&d), 1.0);
        (st.buttons, st.x.signum(), st.y.signum())
    }

    fn check(
        preset_name: &str,
        dev_name: &str,
        vid: u16,
        pid: u16,
        cases: Vec<(&str, Vec<Ev>, Out)>,
    ) {
        let all = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap();
        let preset = all
            .iter()
            .find(|p| p.name == preset_name)
            .expect(preset_name);
        let dev = DeviceState {
            name: dev_name.into(),
            vendor: Some(vid),
            product: Some(pid),
            ..Default::default()
        };
        assert!(
            preset.matches(&dev),
            "{preset_name} doesn't match {dev_name}"
        );
        for (label, evs, want) in cases {
            let (buttons, x, y) = run(preset, dev_name, vid, pid, &evs);
            let got = if buttons != 0 {
                let pressed: Vec<_> = N64Button::ALL
                    .into_iter()
                    .filter(|b| buttons & b.mask() != 0)
                    .collect();
                assert_eq!(pressed.len(), 1, "{label}: several N64 buttons {pressed:?}");
                Out::Button(pressed[0])
            } else if x != 0 || y != 0 {
                Out::Stick(x, y)
            } else {
                Out::Nothing
            };
            assert_eq!(got, want, "{preset_name}: {label}");
        }
    }

    #[test]
    fn eightbitdo_d_mode_capture() {
        use Ev::*;
        use N64Button as N;
        use PadAxis as X;
        use PadButton as P;
        check(
            "8BitDo 64 — D mode",
            "8BitDo 64",
            0x2dc8,
            0x3019,
            vec![
                ("A", vec![Btn(P::South)], Out::Button(N::A)),
                ("B", vec![Btn(P::East)], Out::Button(N::B)),
                (
                    "Z left",
                    vec![Btn(P::LeftTrigger), RawAxis(131269, 1.0)],
                    Out::Button(N::Z),
                ),
                (
                    "Z right",
                    vec![Btn(P::RightTrigger), RawAxis(131268, 1.0)],
                    Out::Button(N::Z),
                ),
                ("Start", vec![Btn(P::Start)], Out::Button(N::Start)),
                ("L", vec![Btn(P::LeftShoulder)], Out::Button(N::L)),
                ("R", vec![Btn(P::RightShoulder)], Out::Button(N::R)),
                ("D up", vec![Axis(X::DPadY, 1.0)], Out::Button(N::DUp)),
                ("D right", vec![Axis(X::DPadX, 1.0)], Out::Button(N::DRight)),
                ("D down", vec![Axis(X::DPadY, -1.0)], Out::Button(N::DDown)),
                ("D left", vec![Axis(X::DPadX, -1.0)], Out::Button(N::DLeft)),
                (
                    "C up",
                    vec![Axis(X::RightStickY, 0.992)],
                    Out::Button(N::CUp),
                ),
                (
                    "C right",
                    vec![Axis(X::RightStickX, 1.0)],
                    Out::Button(N::CRight),
                ),
                (
                    "C down",
                    vec![Axis(X::RightStickY, -1.0)],
                    Out::Button(N::CDown),
                ),
                (
                    "C left",
                    vec![Axis(X::RightStickX, -0.992)],
                    Out::Button(N::CLeft),
                ),
                (
                    "stick up",
                    vec![Axis(X::LeftStickY, 0.992)],
                    Out::Stick(0, 1),
                ),
                (
                    "stick right",
                    vec![Axis(X::LeftStickX, 1.0)],
                    Out::Stick(1, 0),
                ),
                (
                    "stick down",
                    vec![Axis(X::LeftStickY, -1.0)],
                    Out::Stick(0, -1),
                ),
                (
                    "stick left",
                    vec![Axis(X::LeftStickX, -0.992)],
                    Out::Stick(-1, 0),
                ),
                ("Home", vec![Btn(P::Mode)], Out::Nothing),
                (
                    "Star (labelled DPadLeft)",
                    vec![Btn(P::DPadLeft)],
                    Out::Nothing,
                ),
                ("Dash", vec![Btn(P::Select)], Out::Nothing),
                ("stick click", vec![Btn(P::LeftThumb)], Out::Nothing),
            ],
        );
    }

    #[test]
    fn eightbitdo_s_mode_macos_capture() {
        use Ev::*;
        use N64Button as N;
        use PadAxis as X;
        use PadButton as P;
        // C buttons: the mapping DB reports every one as a *positive* right-stick move,
        // alongside its own raw code. Both are present in the device state.
        check(
            "8BitDo 64 — S mode (macOS)",
            "NSO N64 Controller",
            0x057e,
            0x2019,
            vec![
                ("A", vec![Btn(P::South)], Out::Button(N::A)),
                ("B", vec![Btn(P::East)], Out::Button(N::B)),
                ("Z left", vec![Btn(P::LeftTrigger)], Out::Button(N::Z)),
                ("Z right", vec![Btn(P::RightTrigger)], Out::Button(N::Z)),
                ("Start", vec![Btn(P::Start)], Out::Button(N::Start)),
                ("L", vec![Btn(P::LeftShoulder)], Out::Button(N::L)),
                ("R", vec![Btn(P::RightShoulder)], Out::Button(N::R)),
                ("D up", vec![Axis(X::DPadY, 1.0)], Out::Button(N::DUp)),
                ("D right", vec![Axis(X::DPadX, 1.0)], Out::Button(N::DRight)),
                ("D down", vec![Axis(X::DPadY, -1.0)], Out::Button(N::DDown)),
                ("D left", vec![Axis(X::DPadX, -1.0)], Out::Button(N::DLeft)),
                (
                    "C up",
                    vec![Axis(X::RightStickY, 1.0), RawAxis(589827, 1.0)],
                    Out::Button(N::CUp),
                ),
                (
                    "C right",
                    vec![Axis(X::RightStickX, 1.0), RawAxis(589833, 1.0)],
                    Out::Button(N::CRight),
                ),
                (
                    "C down",
                    vec![Axis(X::RightStickY, 1.0), RawAxis(589832, 1.0)],
                    Out::Button(N::CDown),
                ),
                (
                    "C left",
                    vec![Axis(X::RightStickX, 1.0), RawAxis(589828, 1.0)],
                    Out::Button(N::CLeft),
                ),
                ("stick up", vec![Axis(X::LeftStickY, 1.0)], Out::Stick(0, 1)),
                (
                    "stick right",
                    vec![Axis(X::LeftStickX, 1.0)],
                    Out::Stick(1, 0),
                ),
                (
                    "stick down",
                    vec![Axis(X::LeftStickY, -1.0)],
                    Out::Stick(0, -1),
                ),
                (
                    "stick left",
                    vec![Axis(X::LeftStickX, -1.0)],
                    Out::Stick(-1, 0),
                ),
                ("Home", vec![Btn(P::Mode)], Out::Nothing),
                ("Star", vec![RawBtn(589838)], Out::Nothing),
            ],
        );
    }

    #[test]
    fn detects_switch_mode_n64() {
        let browser_s = dev("N64 Controller (Vendor: 057e Product: 2019)");
        let mac_s = DeviceState {
            name: "NSO N64 Controller".into(),
            vendor: Some(0x057e),
            product: Some(0x2019),
            ..Default::default()
        };
        let d_mode = DeviceState {
            name: "8BitDo 64".into(),
            vendor: Some(0x2dc8),
            product: Some(0x3019),
            ..Default::default()
        };
        assert!(is_switch_n64(&browser_s));
        assert!(is_switch_n64(&mac_s));
        assert!(!is_switch_n64(&d_mode));
        assert!(!is_switch_n64(&dev("Xbox Wireless Controller")));
    }

    #[test]
    fn modes_pick_their_own_preset() {
        let all = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap();
        let mac: Vec<_> = all
            .iter()
            .filter(|p| p.platforms.is_empty() || p.platforms.iter().any(|x| x == "macos"))
            .cloned()
            .collect();
        let web: Vec<_> = all
            .iter()
            .filter(|p| p.platforms.is_empty() || p.platforms.iter().any(|x| x == "web"))
            .cloned()
            .collect();
        let d = |n: &str, v, p| DeviceState {
            name: n.into(),
            vendor: Some(v),
            product: Some(p),
            ..Default::default()
        };
        let name = |list: &[ControllerPreset], dev: &DeviceState| {
            suggested_preset(list, dev).map(|p| p.name.clone())
        };
        assert_eq!(
            name(&mac, &d("8BitDo 64", 0x2dc8, 0x3019)).as_deref(),
            Some("8BitDo 64 — D mode")
        );
        assert_eq!(
            name(&mac, &d("NSO N64 Controller", 0x057e, 0x2019)).as_deref(),
            Some("8BitDo 64 — S mode (macOS)")
        );
        let browser_s = DeviceState {
            name: "N64 Controller (Vendor: 057e Product: 2019)".into(),
            ..Default::default()
        };
        // S mode is unsupported in the browser: no preset should claim it.
        assert_eq!(name(&web, &browser_s), None);
    }

    // Captured idle report 0x3F payload from the 8BitDo 64 in S mode (tools/hid-listen).
    const IDLE: [u8; 11] = [
        0x00, 0x00, 0x08, 0xff, 0x7f, 0xff, 0x7f, 0x00, 0x80, 0x00, 0x80,
    ];

    fn hid_dev(payload: [u8; 11]) -> DeviceState {
        DeviceState {
            name: "N64 Controller (HID)".into(),
            vendor: Some(0x057e),
            product: Some(0x2019),
            is_hid: true,
            hid: vec![(63, payload.to_vec())],
            ..Default::default()
        }
    }

    fn with(changes: &[(usize, u8)]) -> [u8; 11] {
        let mut p = IDLE;
        for (i, v) in changes {
            p[*i] = *v;
        }
        p
    }

    #[test]
    fn eightbitdo_s_mode_hid_capture() {
        use N64Button as N;
        let all = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap();
        let preset = all
            .iter()
            .find(|p| p.name == "8BitDo 64 — S mode (HID)")
            .unwrap();
        assert!(preset.matches(&hid_dev(IDLE)));
        // The HID preset must not grab the OS-gamepad view of the same controller, and vice versa.
        let gamepad_view = DeviceState {
            name: "NSO N64 Controller".into(),
            vendor: Some(0x057e),
            product: Some(0x2019),
            ..Default::default()
        };
        assert!(!preset.matches(&gamepad_view));
        let mut c = ControllerConfig::defaults(0, false);
        preset.apply(&mut c);
        let eval = |p: [u8; 11]| {
            let d = hid_dev(p);
            let snap = Snapshot {
                devices: vec![d.clone()],
                ..Default::default()
            };
            evaluate_with(&c, &snap, Some(&d), 1.0).0
        };
        assert_eq!(eval(IDLE).buttons, 0);
        assert_eq!((eval(IDLE).x, eval(IDLE).y), (0, 0));
        let cases: Vec<(&str, [u8; 11], N)> = vec![
            ("A", with(&[(0, 0x02)]), N::A),
            ("B", with(&[(0, 0x01)]), N::B),
            ("Z left", with(&[(0, 0x40)]), N::Z),
            ("Z right", with(&[(1, 0x04)]), N::Z),
            ("Start", with(&[(1, 0x02)]), N::Start),
            ("L", with(&[(0, 0x10)]), N::L),
            ("R", with(&[(0, 0x20)]), N::R),
            ("C up", with(&[(0, 0x04)]), N::CUp),
            ("C right", with(&[(1, 0x01)]), N::CRight),
            ("C down", with(&[(0, 0x80)]), N::CDown),
            ("C left", with(&[(0, 0x08)]), N::CLeft),
            ("D up", with(&[(2, 0)]), N::DUp),
            ("D right", with(&[(2, 2)]), N::DRight),
            ("D down", with(&[(2, 4)]), N::DDown),
            ("D left", with(&[(2, 6)]), N::DLeft),
        ];
        for (label, p, want) in cases {
            assert_eq!(eval(p).buttons, want.mask(), "{label}");
        }
        assert_eq!(eval(with(&[(1, 0x10)])).buttons, 0, "Home unbound");
        assert_eq!(eval(with(&[(1, 0x20)])).buttons, 0, "Star unbound");
        // Stick: right = X 0xffff, up = Y 0x0000 (inverted).
        let r = eval(with(&[(3, 0xff), (4, 0xff)]));
        assert!(r.x > 80 && r.y == 0, "right {r:?}");
        let u = eval(with(&[(5, 0x00), (6, 0x00)]));
        assert!(u.y > 80 && u.x == 0, "up {u:?}");
        let l = eval(with(&[(3, 0x00), (4, 0x00)]));
        assert!(l.x < -80, "left {l:?}");
        let d = eval(with(&[(5, 0xff), (6, 0xff)]));
        assert!(d.y < -80, "down {d:?}");
    }

    #[test]
    fn guided_capture_detects_hid() {
        use crate::input::Input;
        let snap = |p| Snapshot {
            devices: vec![hid_dev(p)],
            ..Default::default()
        };
        let base = snap(IDLE);
        let got = |p| snap(p).detect_new(&base).map(|(i, _)| i);
        assert_eq!(
            got(with(&[(0, 0x02)])),
            Some(Input::HidBit {
                report: 63,
                byte: 0,
                bit: 1
            })
        );
        assert_eq!(
            got(with(&[(1, 0x04)])),
            Some(Input::HidBit {
                report: 63,
                byte: 1,
                bit: 2
            })
        );
        assert_eq!(
            got(with(&[(2, 2)])),
            Some(Input::HidHat {
                report: 63,
                byte: 2,
                dir: 1
            })
        );
        assert_eq!(
            got(with(&[(3, 0xff), (4, 0xff)])),
            Some(Input::HidAxis {
                report: 63,
                byte: 3,
                wide: true,
                invert: false,
                positive: true
            })
        );
        assert_eq!(
            got(with(&[(5, 0x00), (6, 0x00)])),
            Some(Input::HidAxis {
                report: 63,
                byte: 5,
                wide: true,
                invert: false,
                positive: false
            })
        );
        // Small stick noise must not register as a button bit.
        assert_eq!(got(with(&[(3, 0xfe)])), None);
        assert_eq!(got(with(&[(4, 0x80)])), None);
    }

    #[test]
    fn presets_set_group_mechanisms() {
        use crate::config::{Group, GroupMode};
        let all = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap();
        let get = |n: &str| all.iter().find(|p| p.name == n).unwrap();
        let mut c = ControllerConfig::defaults(0, false);
        get("8BitDo 64 — D mode").apply(&mut c);
        // D mode: D-pad arrives as DPadX/DPadY axes, C buttons on the right stick.
        assert_eq!(c.group_modes.get(Group::DPad), GroupMode::DualAxis);
        assert_eq!(c.group_modes.get(Group::C), GroupMode::DualAxis);
        assert_eq!(c.group_modes.get(Group::Stick), GroupMode::DualAxis);
        get("8BitDo 64 — S mode (HID)").apply(&mut c);
        // S mode HID: hat D-pad and bit C buttons are "four buttons".
        assert_eq!(c.group_modes.get(Group::DPad), GroupMode::FourButtons);
        assert_eq!(c.group_modes.get(Group::C), GroupMode::FourButtons);
        assert_eq!(c.group_modes.get(Group::Stick), GroupMode::DualAxis);
    }

    #[test]
    fn mixed_axis_and_buttons_infer() {
        use crate::config::{Dir, Group, GroupMode};
        use crate::input::{Input, PadAxis, PadButton};
        // e.g. C buttons split: X on an axis, Up/Down on two buttons.
        let mut c = ControllerConfig::defaults(0, false);
        *c.group_dir(Group::C, Dir::Right) = vec![Input::axis(PadAxis::RightStickX, true)];
        *c.group_dir(Group::C, Dir::Left) = vec![Input::axis(PadAxis::RightStickX, false)];
        *c.group_dir(Group::C, Dir::Up) = vec![Input::button(PadButton::North)];
        *c.group_dir(Group::C, Dir::Down) = vec![Input::button(PadButton::West)];
        c.infer_group_modes();
        assert_eq!(c.group_modes.get(Group::C), GroupMode::AxisXButtonsY);
        assert_eq!(
            Input::HidAxis {
                report: 63,
                byte: 3,
                wide: true,
                invert: true,
                positive: true
            }
            .opposite(),
            Some(Input::HidAxis {
                report: 63,
                byte: 3,
                wide: true,
                invert: true,
                positive: false
            })
        );
        assert_eq!(
            Input::HidHat {
                report: 63,
                byte: 2,
                dir: 1
            }
            .opposite(),
            Some(Input::HidHat {
                report: 63,
                byte: 2,
                dir: 3
            })
        );
        assert_eq!(Input::button(PadButton::South).opposite(), None);
    }

    #[test]
    fn eightbitdo_d_mode_browser() {
        use crate::input::{PadAxis, PadButton};
        use N64Button as N;
        let all = parse_controller_presets(BUILTIN_CONTROLLERS_YAML).unwrap();
        let web: Vec<_> = all
            .iter()
            .filter(|p| p.platforms.iter().any(|x| x == "web") || p.platforms.is_empty())
            .cloned()
            .collect();
        let mut d = DeviceState {
            name: "8BitDo 64 BT (Vendor: 2dc8 Product: 3019)".into(),
            ..Default::default()
        };
        let p = suggested_preset(&web, &d).expect("web D preset");
        assert_eq!(p.name, "8BitDo 64 — D mode (browser)");
        let mut c = ControllerConfig::defaults(0, false);
        p.apply(&mut c);
        // Saved user profile: C buttons as X axis + Up/Down, 2% deadzone.
        assert_eq!(
            c.group_modes.get(crate::config::Group::C),
            crate::config::GroupMode::AxisXButtonsY
        );
        assert_eq!(c.stick.deadzone, 0.02);
        // Resting state as the browser reports it: hat axis 9 (code 58) at 3.286, axis 5 at 0.
        d.raw_axes = vec![(54, -0.004), (58, 3.2857)];
        d.axes[PadAxis::RightStickY as usize] = -1.0; // browser axis 3 rests at -1
        let eval = |d: &DeviceState| {
            let snap = Snapshot {
                devices: vec![d.clone()],
                ..Default::default()
            };
            evaluate_with(&c, &snap, Some(d), 1.0).0.buttons
        };
        assert_eq!(eval(&d), 0, "nothing pressed at rest");
        let hat = |v: f32| {
            let mut x = d.clone();
            x.raw_axes[1].1 = v;
            eval(&x)
        };
        assert_eq!(hat(-1.0), N::DUp.mask());
        assert_eq!(hat(-0.4286), N::DRight.mask());
        assert_eq!(hat(0.1429), N::DDown.mask());
        assert_eq!(hat(0.7143), N::DLeft.mask());
        let mut x = d.clone();
        x.raw_axes[0].1 = -1.0;
        assert_eq!(eval(&x), N::CUp.mask());
        let mut x = d.clone();
        x.buttons[PadButton::Select as usize] = 1.0;
        assert_eq!(eval(&x), N::Z.mask());
    }
}

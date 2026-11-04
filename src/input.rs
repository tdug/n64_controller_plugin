//! Host input: bindings model, device snapshot, gilrs backend, keyboard.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Gamepad buttons in the standard (SDL-style) layout gilrs maps devices to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PadButton {
    South,
    East,
    North,
    West,
    LeftShoulder,
    LeftTrigger,
    RightShoulder,
    RightTrigger,
    Select,
    Start,
    Mode,
    LeftThumb,
    RightThumb,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    C,
    Z,
}

impl PadButton {
    pub const ALL: [PadButton; 19] = [
        PadButton::South,
        PadButton::East,
        PadButton::North,
        PadButton::West,
        PadButton::LeftShoulder,
        PadButton::LeftTrigger,
        PadButton::RightShoulder,
        PadButton::RightTrigger,
        PadButton::Select,
        PadButton::Start,
        PadButton::Mode,
        PadButton::LeftThumb,
        PadButton::RightThumb,
        PadButton::DPadUp,
        PadButton::DPadDown,
        PadButton::DPadLeft,
        PadButton::DPadRight,
        PadButton::C,
        PadButton::Z,
    ];

    fn gilrs(self) -> gilrs::Button {
        use gilrs::Button as G;
        match self {
            PadButton::South => G::South,
            PadButton::East => G::East,
            PadButton::North => G::North,
            PadButton::West => G::West,
            PadButton::LeftShoulder => G::LeftTrigger,
            PadButton::LeftTrigger => G::LeftTrigger2,
            PadButton::RightShoulder => G::RightTrigger,
            PadButton::RightTrigger => G::RightTrigger2,
            PadButton::Select => G::Select,
            PadButton::Start => G::Start,
            PadButton::Mode => G::Mode,
            PadButton::LeftThumb => G::LeftThumb,
            PadButton::RightThumb => G::RightThumb,
            PadButton::DPadUp => G::DPadUp,
            PadButton::DPadDown => G::DPadDown,
            PadButton::DPadLeft => G::DPadLeft,
            PadButton::DPadRight => G::DPadRight,
            PadButton::C => G::C,
            PadButton::Z => G::Z,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PadButton::South => "A / Cross",
            PadButton::East => "B / Circle",
            PadButton::North => "Y / Triangle",
            PadButton::West => "X / Square",
            PadButton::LeftShoulder => "LB / L1",
            PadButton::LeftTrigger => "LT / L2",
            PadButton::RightShoulder => "RB / R1",
            PadButton::RightTrigger => "RT / R2",
            PadButton::Select => "Back / Select",
            PadButton::Start => "Start",
            PadButton::Mode => "Guide",
            PadButton::LeftThumb => "L3",
            PadButton::RightThumb => "R3",
            PadButton::DPadUp => "D-Pad Up",
            PadButton::DPadDown => "D-Pad Down",
            PadButton::DPadLeft => "D-Pad Left",
            PadButton::DPadRight => "D-Pad Right",
            PadButton::C => "C",
            PadButton::Z => "Z",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PadAxis {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftZ,
    RightZ,
    DPadX,
    DPadY,
}

impl PadAxis {
    pub const ALL: [PadAxis; 8] = [
        PadAxis::LeftStickX,
        PadAxis::LeftStickY,
        PadAxis::RightStickX,
        PadAxis::RightStickY,
        PadAxis::LeftZ,
        PadAxis::RightZ,
        PadAxis::DPadX,
        PadAxis::DPadY,
    ];

    fn gilrs(self) -> gilrs::Axis {
        use gilrs::Axis as G;
        match self {
            PadAxis::LeftStickX => G::LeftStickX,
            PadAxis::LeftStickY => G::LeftStickY,
            PadAxis::RightStickX => G::RightStickX,
            PadAxis::RightStickY => G::RightStickY,
            PadAxis::LeftZ => G::LeftZ,
            PadAxis::RightZ => G::RightZ,
            PadAxis::DPadX => G::DPadX,
            PadAxis::DPadY => G::DPadY,
        }
    }

    pub fn label(self, positive: bool) -> String {
        let (name, pos, neg) = match self {
            PadAxis::LeftStickX => ("Left Stick", "Right", "Left"),
            PadAxis::LeftStickY => ("Left Stick", "Up", "Down"),
            PadAxis::RightStickX => ("Right Stick", "Right", "Left"),
            PadAxis::RightStickY => ("Right Stick", "Up", "Down"),
            PadAxis::LeftZ => ("Left Z", "+", "-"),
            PadAxis::RightZ => ("Right Z", "+", "-"),
            PadAxis::DPadX => ("D-Pad X", "+", "-"),
            PadAxis::DPadY => ("D-Pad Y", "+", "-"),
        };
        format!("{name} {}", if positive { pos } else { neg })
    }
}

/// A single host input that can be bound to something.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Input {
    /// Keyboard key, as a Windows virtual-key code (also used on other platforms).
    Key {
        code: u8,
    },
    Button {
        button: PadButton,
    },
    Axis {
        axis: PadAxis,
        positive: bool,
    },
    /// Unmapped device button (raw platform event code).
    RawButton {
        code: u32,
    },
    RawAxis {
        code: u32,
        positive: bool,
    },
    /// Hat switch reported as a single axis (browser Gamepad API: -1..1 for 8 directions,
    /// ≈1.29 when centred). `dir`: 0 up, 1 right, 2 down, 3 left.
    RawHat {
        code: u32,
        dir: u8,
    },
    /// One bit of a raw HID input report (`byte` counts from after the report id).
    HidBit {
        report: u8,
        byte: u16,
        bit: u8,
    },
    /// HID hat byte: 0-7 clockwise from up, anything else = released. `dir`: 0 up .. 3 left.
    HidHat {
        report: u8,
        byte: u16,
        dir: u8,
    },
    /// HID axis: unsigned, centred at half range; 16-bit little-endian if `wide`, else 8-bit.
    HidAxis {
        report: u8,
        byte: u16,
        #[serde(default)]
        wide: bool,
        /// Flip direction (e.g. Y axes where up is the low value).
        #[serde(default)]
        invert: bool,
        positive: bool,
    },
}

/// Is `dir` (0 up, 1 right, 2 down, 3 left) held at hat position `k` (0-7 clockwise from up)?
pub fn hat_index_pressed(k: i32, dir: u8) -> bool {
    if !(0..8).contains(&k) {
        return false;
    }
    let dirs: &[u8] = match k {
        0 => &[0],
        1 => &[0, 1],
        2 => &[1],
        3 => &[1, 2],
        4 => &[2],
        5 => &[2, 3],
        6 => &[3],
        _ => &[3, 0],
    };
    dirs.contains(&dir)
}

/// Is `dir` (0 up, 1 right, 2 down, 3 left) held on a single-axis hat value?
pub fn hat_pressed(v: f32, dir: u8) -> bool {
    if !(-1.1..=1.1).contains(&v) {
        return false; // centred
    }
    // 8 positions clockwise from up.
    hat_index_pressed((((v + 1.0) * 3.5).round() as i32).rem_euclid(8), dir)
}

impl Input {
    /// The same control pushed the other way: the other half of an axis, or the opposite
    /// hat direction. `None` for plain buttons and keys.
    pub fn opposite(&self) -> Option<Input> {
        Some(match self {
            Input::Axis { axis, positive } => Input::Axis {
                axis: *axis,
                positive: !positive,
            },
            Input::RawAxis { code, positive } => Input::RawAxis {
                code: *code,
                positive: !positive,
            },
            Input::HidAxis {
                report,
                byte,
                wide,
                invert,
                positive,
            } => Input::HidAxis {
                report: *report,
                byte: *byte,
                wide: *wide,
                invert: *invert,
                positive: !positive,
            },
            Input::RawHat { code, dir } => Input::RawHat {
                code: *code,
                dir: (dir + 2) % 4,
            },
            Input::HidHat { report, byte, dir } => Input::HidHat {
                report: *report,
                byte: *byte,
                dir: (dir + 2) % 4,
            },
            _ => return None,
        })
    }

    /// An analog axis half (not a button, key or hat).
    pub fn is_axis(&self) -> bool {
        matches!(
            self,
            Input::Axis { .. } | Input::RawAxis { .. } | Input::HidAxis { .. }
        )
    }

    pub fn key(code: u8) -> Self {
        Input::Key { code }
    }
    pub fn button(button: PadButton) -> Self {
        Input::Button { button }
    }
    pub fn axis(axis: PadAxis, positive: bool) -> Self {
        Input::Axis { axis, positive }
    }

    pub fn label(&self) -> String {
        match self {
            Input::Key { code } => format!("Key {}", vk_name(*code)),
            Input::Button { button } => format!("Pad {}", button.label()),
            Input::Axis { axis, positive } => format!("Pad {}", axis.label(*positive)),
            Input::RawButton { code } => format!("Pad button #{code}"),
            Input::RawAxis { code, positive } => {
                format!("Pad axis #{code}{}", if *positive { "+" } else { "-" })
            }
            Input::RawHat { code, dir } => format!(
                "Pad hat #{code} {}",
                ["Up", "Right", "Down", "Left"][(*dir as usize).min(3)]
            ),
            Input::HidBit { byte, bit, .. } => format!("HID byte {byte} bit {bit}"),
            Input::HidHat { byte, dir, .. } => format!(
                "HID hat (byte {byte}) {}",
                ["Up", "Right", "Down", "Left"][(*dir as usize).min(3)]
            ),
            Input::HidAxis { byte, positive, .. } => {
                format!(
                    "HID axis (byte {byte}){}",
                    if *positive { "+" } else { "-" }
                )
            }
        }
    }
}

/// Which host gamepad drives an N64 controller port.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceSel {
    /// N-th connected gamepad for port N.
    #[default]
    Auto,
    /// Keyboard only.
    None,
    Specific {
        uuid: String,
        name: String,
        nth: usize,
    },
}

#[derive(Clone, Debug, Default)]
pub struct DeviceState {
    pub id: usize,
    pub uuid: String,
    pub name: String,
    pub buttons: [f32; PadButton::ALL.len()],
    pub axes: [f32; PadAxis::ALL.len()],
    pub raw_buttons: Vec<(u32, f32)>,
    pub raw_axes: Vec<(u32, f32)>,
    pub ff: bool,
    pub vendor: Option<u16>,
    pub product: Option<u16>,
    /// Raw HID device (bound byte-by-byte) rather than an OS gamepad.
    pub is_hid: bool,
    /// HID input reports: (report id, payload without the id byte).
    pub hid: Vec<(u8, Vec<u8>)>,
}

impl DeviceState {
    fn hid_report(&self, report: u8) -> Option<&[u8]> {
        self.hid
            .iter()
            .find(|(r, _)| *r == report)
            .map(|(_, p)| p.as_slice())
    }

    /// Normalized -1..1 value of a HID axis.
    pub fn hid_axis(&self, report: u8, byte: u16, wide: bool) -> Option<f32> {
        let p = self.hid_report(report)?;
        let i = byte as usize;
        Some(if wide {
            let v = u16::from_le_bytes([*p.get(i)?, *p.get(i + 1)?]) as f32;
            (v - 32768.0) / 32768.0
        } else {
            (*p.get(i)? as f32 - 128.0) / 128.0
        })
    }

    /// "vvvv:pppp" in lower-case hex, when known.
    pub fn vid_pid(&self) -> Option<String> {
        Some(format!("{:04x}:{:04x}", self.vendor?, self.product?))
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub keys: [bool; 256],
    pub devices: Vec<DeviceState>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            keys: [false; 256],
            devices: Vec::new(),
        }
    }
}

impl Snapshot {
    pub fn resolve(&self, sel: &DeviceSel, port: usize) -> Option<&DeviceState> {
        match sel {
            DeviceSel::None => None,
            DeviceSel::Auto => self.devices.get(port),
            DeviceSel::Specific { uuid, name, nth } => self
                .devices
                .iter()
                .filter(|d| &d.uuid == uuid && &d.name == name)
                .nth(*nth)
                .or_else(|| self.devices.iter().find(|d| &d.name == name)),
        }
    }

    /// Ordinal of `dev` among devices with the same uuid+name, for `DeviceSel::Specific`.
    pub fn select(&self, dev: &DeviceState) -> DeviceSel {
        let nth = self
            .devices
            .iter()
            .filter(|d| d.uuid == dev.uuid && d.name == dev.name)
            .position(|d| d.id == dev.id)
            .unwrap_or(0);
        DeviceSel::Specific {
            uuid: dev.uuid.clone(),
            name: dev.name.clone(),
            nth,
        }
    }

    /// Analog value 0..=1 of an input.
    pub fn value(&self, input: &Input, dev: Option<&DeviceState>) -> f32 {
        let dir = |v: f32, positive: bool| if positive { v.max(0.0) } else { (-v).max(0.0) };
        match input {
            Input::Key { code } => self.keys[*code as usize] as u8 as f32,
            Input::Button { button } => dev.map_or(0.0, |d| d.buttons[*button as usize]),
            Input::Axis { axis, positive } => {
                dev.map_or(0.0, |d| dir(d.axes[*axis as usize], *positive))
            }
            Input::RawButton { code } => dev.map_or(0.0, |d| {
                d.raw_buttons
                    .iter()
                    .find(|(c, _)| c == code)
                    .map_or(0.0, |(_, v)| *v)
            }),
            Input::RawAxis { code, positive } => dev.map_or(0.0, |d| {
                d.raw_axes
                    .iter()
                    .find(|(c, _)| c == code)
                    .map_or(0.0, |(_, v)| dir(*v, *positive))
            }),
            Input::RawHat { code, dir: hd } => dev.map_or(0.0, |d| {
                d.raw_axes
                    .iter()
                    .find(|(c, _)| c == code)
                    .map_or(0.0, |(_, v)| hat_pressed(*v, *hd) as u8 as f32)
            }),
            Input::HidBit { report, byte, bit } => dev
                .and_then(|d| d.hid_report(*report)?.get(*byte as usize).copied())
                .map_or(0.0, |b| ((b >> bit) & 1) as f32),
            Input::HidHat {
                report,
                byte,
                dir: hd,
            } => dev
                .and_then(|d| d.hid_report(*report)?.get(*byte as usize).copied())
                .map_or(0.0, |b| hat_index_pressed(b as i32, *hd) as u8 as f32),
            Input::HidAxis {
                report,
                byte,
                wide,
                invert,
                positive,
            } => dev
                .and_then(|d| d.hid_axis(*report, *byte, *wide))
                .map_or(0.0, |v| dir(if *invert { -v } else { v }, *positive)),
        }
    }

    pub fn any_pressed(&self, inputs: &[Input], dev: Option<&DeviceState>) -> bool {
        inputs.iter().any(|i| self.value(i, dev) > 0.5)
    }

    pub fn all_pressed(&self, inputs: &[Input], dev: Option<&DeviceState>) -> bool {
        !inputs.is_empty() && inputs.iter().all(|i| self.value(i, dev) > 0.5)
    }

    pub fn max_value(&self, inputs: &[Input], dev: Option<&DeviceState>) -> f32 {
        inputs
            .iter()
            .map(|i| self.value(i, dev))
            .fold(0.0, f32::max)
    }

    /// First input that became active between `base` and `self` (for binding capture).
    /// Returns the input and, for gamepad inputs, the device it came from.
    pub fn detect_new(&self, base: &Snapshot) -> Option<(Input, Option<usize>)> {
        for code in 7..=255u8 {
            // Skip mouse buttons and the side-agnostic modifier codes (the L/R variants also fire).
            if matches!(code, 0x10..=0x12) {
                continue;
            }
            if self.keys[code as usize] && !base.keys[code as usize] {
                return Some((Input::key(code), None));
            }
        }
        for d in &self.devices {
            let empty = DeviceState::default();
            let b = base.devices.iter().find(|x| x.id == d.id).unwrap_or(&empty);
            for (i, btn) in PadButton::ALL.iter().enumerate() {
                if d.buttons[i] > 0.6 && b.buttons.get(i).copied().unwrap_or(0.0) < 0.3 {
                    return Some((Input::button(*btn), Some(d.id)));
                }
            }
            for (i, ax) in PadAxis::ALL.iter().enumerate() {
                let (now, was) = (d.axes[i], b.axes.get(i).copied().unwrap_or(0.0));
                if now.abs() > 0.6 && (now - was).abs() > 0.5 {
                    return Some((Input::axis(*ax, now > 0.0), Some(d.id)));
                }
            }
            for (code, v) in &d.raw_buttons {
                let was = b
                    .raw_buttons
                    .iter()
                    .find(|(c, _)| c == code)
                    .map_or(0.0, |x| x.1);
                if *v > 0.6 && was < 0.3 {
                    return Some((Input::RawButton { code: *code }, Some(d.id)));
                }
            }
            for (code, v) in &d.raw_axes {
                let base = b.raw_axes.iter().find(|(c, _)| c == code).map(|x| x.1);
                let was = base.unwrap_or(0.0);
                // Hat encoded as one axis (browser): centred value is outside -1..1.
                // A first-seen axis sitting exactly on the 1/7 hat grid (not ±1) is a hat too.
                let on_grid = |x: f32| {
                    let k = (x + 1.0) * 3.5;
                    (k - k.round()).abs() < 0.01 && x.abs() < 0.99
                };
                let hat_like = was > 1.1 || (base.is_none() && on_grid(*v));
                if hat_like && *v <= 1.1 {
                    if let Some(hd) = (0..4).find(|hd| hat_pressed(*v, *hd)) {
                        return Some((
                            Input::RawHat {
                                code: *code,
                                dir: hd,
                            },
                            Some(d.id),
                        ));
                    }
                }
                if was > 1.1 || *v > 1.1 {
                    continue;
                }
                if v.abs() > 0.6 && (v - was).abs() > 0.5 {
                    return Some((
                        Input::RawAxis {
                            code: *code,
                            positive: *v > 0.0,
                        },
                        Some(d.id),
                    ));
                }
            }
            if let Some(i) = detect_hid(d, b) {
                return Some((i, Some(d.id)));
            }
        }
        None
    }
}

/// Is byte `i` part of a 16-bit value sitting near the centre (an analog axis at rest)?
fn axis_like(p: &[u8], i: usize) -> bool {
    let near = |lo: usize| {
        p.get(lo + 1)
            .map(|hi| u16::from_le_bytes([p[lo], *hi]).abs_diff(0x8000) < 0x1800)
            .unwrap_or(false)
    };
    near(i) || (i > 0 && near(i - 1))
}

/// New HID input between `base` and `now` for binding capture: a single bit turning on, a hat
/// leaving its released state, or an analog axis moving far from centre.
fn detect_hid(now: &DeviceState, base: &DeviceState) -> Option<Input> {
    for (report, p) in &now.hid {
        let Some(b) = base.hid_report(*report) else {
            continue;
        };
        for i in 0..p.len().min(b.len()) {
            if axis_like(b, i) {
                // Axis bytes: bind when the 16-bit value moves far from centre.
                let lo = if i > 0 && axis_like(b, i - 1) && !axis_like_start(b, i) {
                    continue;
                } else {
                    i
                };
                let (Some(v), Some(w)) = (
                    now.hid_axis(*report, lo as u16, true),
                    base.hid_axis(*report, lo as u16, true),
                ) else {
                    continue;
                };
                if v.abs() > 0.6 && w.abs() < 0.3 {
                    return Some(Input::HidAxis {
                        report: *report,
                        byte: lo as u16,
                        wide: true,
                        invert: false,
                        positive: v > 0.0,
                    });
                }
                continue;
            }
            let (was, is) = (b[i], p[i]);
            if was == is {
                continue;
            }
            if was >= 8 && was <= 15 && is < 8 {
                if let Some(dir) = (0..4).find(|dir| hat_index_pressed(is as i32, *dir)) {
                    return Some(Input::HidHat {
                        report: *report,
                        byte: i as u16,
                        dir,
                    });
                }
            }
            let on = is & !was;
            if on.count_ones() == 1 && (was ^ is) == on {
                return Some(Input::HidBit {
                    report: *report,
                    byte: i as u16,
                    bit: on.trailing_zeros() as u8,
                });
            }
        }
    }
    None
}

/// First byte of a centred 16-bit pair (not the high byte of the previous pair).
fn axis_like_start(p: &[u8], i: usize) -> bool {
    // Pairs are aligned from the first axis-like byte of each run.
    let mut j = i;
    while j > 0 && axis_like(p, j - 1) {
        j -= 1;
    }
    (i - j) % 2 == 0
}

// ---------------------------------------------------------------------------
// Keyboard

/// Keys reported through emulator callbacks (Mupen64Plus `SDL_KeyDown`, Zilmar
/// `WM_KeyDown`) or the GUI's own event loop. OR'd with direct polling on Windows.
pub static EVENT_KEYS: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];

pub fn set_event_key(vk: u8, down: bool) {
    EVENT_KEYS[vk as usize].store(down, Ordering::Relaxed);
}

pub fn clear_event_keys() {
    for k in &EVENT_KEYS {
        k.store(false, Ordering::Relaxed);
    }
}

fn read_keyboard(keys: &mut [bool; 256], allow_direct: bool) {
    for (i, k) in keys.iter_mut().enumerate() {
        *k = EVENT_KEYS[i].load(Ordering::Relaxed);
    }
    #[cfg(windows)]
    if allow_direct {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        for vk in 1..256usize {
            // SAFETY: plain Win32 call.
            if unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0 {
                keys[vk] = true;
            }
        }
    }
    #[cfg(not(windows))]
    let _ = allow_direct;
}

/// Human-readable name of a Windows virtual-key code.
pub fn vk_name(vk: u8) -> String {
    let s = match vk {
        0x01 => "Mouse Left",
        0x02 => "Mouse Right",
        0x04 => "Mouse Middle",
        0x05 => "Mouse X1",
        0x06 => "Mouse X2",
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2D => "Insert",
        0x2E => "Delete",
        0x60..=0x69 => return format!("Num {}", vk - 0x60),
        0x6A => "Num *",
        0x6B => "Num +",
        0x6D => "Num -",
        0x6E => "Num .",
        0x6F => "Num /",
        0x70..=0x87 => return format!("F{}", vk - 0x6F),
        0xA0 => "L Shift",
        0xA1 => "R Shift",
        0xA2 => "L Ctrl",
        0xA3 => "R Ctrl",
        0xA4 => "L Alt",
        0xA5 => "R Alt",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        b'0'..=b'9' | b'A'..=b'Z' => return (vk as char).to_string(),
        _ => return format!("Key 0x{vk:02X}"),
    };
    s.to_string()
}

/// SDL2 scancode (as sent by Mupen64Plus `SDL_KeyDown`) → Windows VK code.
pub fn sdl_scancode_to_vk(sc: i32) -> Option<u8> {
    Some(match sc {
        4..=29 => b'A' + (sc - 4) as u8,
        30..=38 => b'1' + (sc - 30) as u8,
        39 => b'0',
        40 => 0x0D,
        41 => 0x1B,
        42 => 0x08,
        43 => 0x09,
        44 => 0x20,
        45 => 0xBD,
        46 => 0xBB,
        47 => 0xDB,
        48 => 0xDD,
        49 => 0xDC,
        51 => 0xBA,
        52 => 0xDE,
        53 => 0xC0,
        54 => 0xBC,
        55 => 0xBE,
        56 => 0xBF,
        57 => 0x14,
        58..=69 => 0x70 + (sc - 58) as u8,
        73 => 0x2D,
        74 => 0x24,
        75 => 0x21,
        76 => 0x2E,
        77 => 0x23,
        78 => 0x22,
        79 => 0x27,
        80 => 0x25,
        81 => 0x28,
        82 => 0x26,
        84 => 0x6F,
        85 => 0x6A,
        86 => 0x6D,
        87 => 0x6B,
        88 => 0x0D,
        89..=97 => 0x61 + (sc - 89) as u8,
        98 => 0x60,
        99 => 0x6E,
        224 => 0xA2,
        225 => 0xA0,
        226 => 0xA4,
        228 => 0xA3,
        229 => 0xA1,
        230 => 0xA5,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Gamepads (gilrs)

pub struct InputManager {
    gilrs: Option<gilrs::Gilrs>,
    rumble: HashMap<usize, gilrs::ff::Effect>,
    #[cfg(not(target_arch = "wasm32"))]
    hid: Option<(Vec<(u16, u16)>, crate::hid::HidReader)>,
}

/// Device ids for raw HID devices start here (gilrs ids are small).
pub const HID_ID_BASE: usize = 100_000;

/// Parse "vvvv:pppp".
pub fn parse_vid_pid(s: &str) -> Option<(u16, u16)> {
    let (v, p) = s.split_once(':')?;
    Some((
        u16::from_str_radix(v, 16).ok()?,
        u16::from_str_radix(p, 16).ok()?,
    ))
}

impl Default for InputManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InputManager {
    pub fn new() -> Self {
        let gilrs = match gilrs::GilrsBuilder::new()
            .with_default_filters(false)
            .build()
        {
            Ok(g) => Some(g),
            Err(gilrs::Error::NotImplemented(g)) => Some(g),
            Err(e) => {
                crate::platform::log(&format!("gamepad backend unavailable: {e}"));
                None
            }
        };
        InputManager {
            gilrs,
            rumble: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            hid: None,
        }
    }

    /// Raw HID devices (vendor:product) to read. Desktop only; the browser asks via WebHID.
    pub fn set_hid_devices(&mut self, ids: &[String]) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ids: Vec<(u16, u16)> = ids.iter().filter_map(|s| parse_vid_pid(s)).collect();
            if self.hid.as_ref().map(|(i, _)| i) != Some(&ids) {
                self.hid =
                    (!ids.is_empty()).then(|| (ids.clone(), crate::hid::HidReader::new(ids)));
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ids;
    }

    fn poll_hid(&self, snap: &mut Snapshot) {
        let mut push = |i: usize, name: String, vid: u16, pid: u16, reports: Vec<(u8, Vec<u8>)>| {
            snap.devices.push(DeviceState {
                id: HID_ID_BASE + i,
                uuid: format!("hid:{vid:04x}:{pid:04x}"),
                name: format!("{name} (HID)"),
                vendor: Some(vid),
                product: Some(pid),
                is_hid: true,
                hid: reports,
                ..Default::default()
            });
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((_, r)) = &self.hid {
            for (i, d) in r.devices().into_iter().enumerate() {
                push(i, d.name, d.vid, d.pid, d.reports.into_iter().collect());
            }
        }
        #[cfg(target_arch = "wasm32")]
        for (i, d) in web_hid::devices().into_iter().enumerate() {
            push(i, d.name, d.vid, d.pid, d.reports);
        }
    }

    /// Refresh `snap` with current device and keyboard state.
    pub fn poll(&mut self, snap: &mut Snapshot, allow_direct_keys: bool) {
        read_keyboard(&mut snap.keys, allow_direct_keys);
        snap.devices.clear();
        self.poll_gilrs(snap);
        self.poll_hid(snap);
    }

    fn poll_gilrs(&mut self, snap: &mut Snapshot) {
        let Some(g) = self.gilrs.as_mut() else { return };
        while g.next_event().is_some() {}
        for (id, pad) in g.gamepads() {
            let mut d = DeviceState {
                id: usize::from(id),
                uuid: pad.uuid().iter().map(|b| format!("{b:02x}")).collect(),
                name: pad.name().to_string(),
                ff: pad.is_ff_supported(),
                vendor: pad.vendor_id(),
                product: pad.product_id(),
                ..Default::default()
            };
            for (i, b) in PadButton::ALL.iter().enumerate() {
                d.buttons[i] = pad.button_data(b.gilrs()).map_or(0.0, |x| x.value());
            }
            for (i, a) in PadAxis::ALL.iter().enumerate() {
                d.axes[i] = pad.axis_data(a.gilrs()).map_or(0.0, |x| x.value());
            }
            let st = pad.state();
            d.raw_buttons = st
                .buttons()
                .map(|(c, b)| (c.into_u32(), b.value()))
                .collect();
            d.raw_axes = st.axes().map(|(c, a)| (c.into_u32(), a.value())).collect();
            #[cfg(target_arch = "wasm32")]
            web_hid::merge_browser_raw(&mut d);
            snap.devices.push(d);
        }
        snap.devices.sort_by_key(|d| d.id);
    }

    /// Start (strength > 0) or stop rumble on a device.
    pub fn set_rumble(&mut self, device: usize, strength: f32) {
        self.rumble.remove(&device); // dropping an Effect stops it
        if strength <= 0.0 {
            return;
        }
        let Some(g) = self.gilrs.as_mut() else { return };
        let Some((id, _)) = g
            .gamepads()
            .find(|(id, p)| usize::from(*id) == device && p.is_ff_supported())
        else {
            return;
        };
        use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Repeat, Replay, Ticks};
        let mag = (strength.clamp(0.0, 1.0) * u16::MAX as f32) as u16;
        let sched = Replay {
            play_for: Ticks::from_ms(1000),
            ..Default::default()
        };
        let effect = EffectBuilder::new()
            .add_effect(BaseEffect {
                kind: BaseEffectType::Strong { magnitude: mag },
                scheduling: sched,
                envelope: Default::default(),
            })
            .add_effect(BaseEffect {
                kind: BaseEffectType::Weak { magnitude: mag },
                scheduling: sched,
                envelope: Default::default(),
            })
            .repeat(Repeat::Infinitely)
            .gamepads(&[id])
            .finish(g);
        match effect {
            Ok(e) => {
                if e.play().is_ok() {
                    self.rumble.insert(device, e);
                }
            }
            Err(e) => crate::platform::log(&format!("rumble failed: {e}")),
        }
    }

    pub fn stop_all_rumble(&mut self) {
        self.rumble.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hat_axis_decoding() {
        assert!(!hat_pressed(1.2857, 0)); // centred
        assert!(hat_pressed(-1.0, 0)); // up
        assert!(hat_pressed(-0.4286, 1)); // right
        assert!(hat_pressed(0.1429, 2)); // down
        assert!(hat_pressed(0.7143, 3)); // left
        assert!(hat_pressed(-0.7143, 0) && hat_pressed(-0.7143, 1)); // up-right
    }
}

/// WebHID bridge (see `index.html`): devices the user granted, with their latest reports.
#[cfg(target_arch = "wasm32")]
pub mod web_hid {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(catch, js_name = tdugHidJson)]
        fn hid_json() -> Result<String, JsValue>;
        #[wasm_bindgen(catch, js_name = tdugHidConnect)]
        fn hid_connect() -> Result<JsValue, JsValue>;
        #[wasm_bindgen(catch, js_name = tdugPadsJson)]
        fn pads_json() -> Result<String, JsValue>;
    }

    #[derive(serde::Deserialize)]
    struct Pad {
        id: String,
        buttons: Vec<f32>,
        axes: Vec<f32>,
    }

    /// gilrs (wasm) only learns a raw axis after it changes, and clamps it to ±1. Overwrite raw
    /// axes/buttons with the browser's own values (all indices, unclamped, including resting
    /// state) so hats like "released = 3.286" are recognised. Codes follow gilrs' numbering:
    /// axis i >= 4 → i + button count + 31, button i >= 17 → i + 31.
    pub fn merge_browser_raw(d: &mut super::DeviceState) {
        let pads: Vec<Pad> =
            serde_json::from_str(&pads_json().unwrap_or_default()).unwrap_or_default();
        let Some(p) = pads.iter().find(|p| p.id == d.name) else {
            return;
        };
        let nb = p.buttons.len();
        let set = |list: &mut Vec<(u32, f32)>, code: u32, v: f32| match list
            .iter_mut()
            .find(|(c, _)| *c == code)
        {
            Some(e) => e.1 = v,
            None => list.push((code, v)),
        };
        for (i, v) in p.axes.iter().enumerate().skip(4) {
            set(&mut d.raw_axes, (i + nb + 31) as u32, *v);
        }
        for (i, v) in p.buttons.iter().enumerate().skip(17) {
            set(&mut d.raw_buttons, (i + 31) as u32, *v);
        }
    }

    #[derive(serde::Deserialize)]
    struct Raw {
        name: String,
        vid: u16,
        pid: u16,
        reports: std::collections::BTreeMap<String, Vec<u8>>,
    }

    pub struct WebHidDevice {
        pub name: String,
        pub vid: u16,
        pub pid: u16,
        pub reports: Vec<(u8, Vec<u8>)>,
    }

    pub fn devices() -> Vec<WebHidDevice> {
        let raw: Vec<Raw> =
            serde_json::from_str(&hid_json().unwrap_or_default()).unwrap_or_default();
        raw.into_iter()
            .map(|r| WebHidDevice {
                name: r.name,
                vid: r.vid,
                pid: r.pid,
                reports: r
                    .reports
                    .into_iter()
                    .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
                    .collect(),
            })
            .collect()
    }

    /// Show the browser's HID device picker (must run from a user gesture).
    pub fn request() {
        let _ = hid_connect();
    }
}

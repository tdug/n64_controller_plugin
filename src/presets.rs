//! Built-in macro presets and controller mapping presets.

use crate::config::{ControllerConfig, Macro, MacroMode, MacroStep, Playback};
use crate::input::{Input, PadAxis, PadButton};
use crate::n64::N64Button;

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

/// A host-controller mapping preset.
pub struct ControllerPreset {
    pub name: &'static str,
    /// Lower-case substrings; all must appear in the device name to suggest this preset.
    pub matches: &'static [&'static [&'static str]],
    pub apply: fn(&mut ControllerConfig),
    pub note: &'static str,
}

pub fn controller_presets() -> &'static [ControllerPreset] {
    &[
        ControllerPreset {
            name: "8BitDo 64 — D mode (“8BitDo 64 BT”)",
            matches: &[&["8bitdo", "64"], &["8bitdo", "n64"]],
            apply: apply_8bitdo_64_xinput,
            note: "Standard layout: C buttons on the right stick, Z on LT, L/R on LB/RB. \
                   Press each button and check its label lights up; if one doesn't (the OS \
                   or browser may not have a mapping for this mode), use Guided setup.",
        },
        ControllerPreset {
            name: "8BitDo 64 — S mode (“N64 Controller”, Switch Online layout)",
            // In S mode the 8BitDo identifies as Nintendo's Switch Online N64 controller.
            matches: &[&["n64 controller"], &["057e", "2019"]],
            apply: apply_8bitdo_64_xinput,
            note: "Same standard layout. Also fits Nintendo's own Switch Online N64 controller. \
                   In browsers this mode often reports a non-standard mapping: if labels don't \
                   light up, use Guided setup.",
        },
    ]
}

/// Device-name check for presets.
pub fn suggested_preset(device_name: &str) -> Option<&'static ControllerPreset> {
    let n = device_name.to_lowercase();
    controller_presets().iter().find(|p| {
        p.matches
            .iter()
            .any(|all| all.iter().all(|s| n.contains(s)))
    })
}

/// Replace gamepad bindings (keeps keyboard keys).
fn set_pad(c: &mut ControllerConfig, b: N64Button, inputs: Vec<Input>) {
    let list = c.buttons.entry(b).or_default();
    list.retain(|i| matches!(i, Input::Key { .. }));
    list.extend(inputs);
}

fn apply_8bitdo_64_xinput(c: &mut ControllerConfig) {
    use N64Button::*;
    use PadButton as P;
    // B sits where an Xbox pad has X or B depending on firmware; bind both (no conflict: the
    // controller has no other face buttons).
    set_pad(c, A, vec![Input::button(P::South)]);
    set_pad(c, B, vec![Input::button(P::West), Input::button(P::East)]);
    set_pad(c, Z, vec![Input::button(P::LeftTrigger)]);
    set_pad(c, Start, vec![Input::button(P::Start)]);
    set_pad(c, L, vec![Input::button(P::LeftShoulder)]);
    set_pad(
        c,
        R,
        vec![
            Input::button(P::RightShoulder),
            Input::button(P::RightTrigger),
        ],
    );
    set_pad(c, DUp, vec![Input::button(P::DPadUp)]);
    set_pad(c, DDown, vec![Input::button(P::DPadDown)]);
    set_pad(c, DLeft, vec![Input::button(P::DPadLeft)]);
    set_pad(c, DRight, vec![Input::button(P::DPadRight)]);
    set_pad(c, CUp, vec![Input::axis(PadAxis::RightStickY, true)]);
    set_pad(c, CDown, vec![Input::axis(PadAxis::RightStickY, false)]);
    set_pad(c, CLeft, vec![Input::axis(PadAxis::RightStickX, false)]);
    set_pad(c, CRight, vec![Input::axis(PadAxis::RightStickX, true)]);
    c.stick_bindings.use_pad_stick(false);
    // The 8BitDo 64 stick is tuned to N64 travel already; a light deadzone is enough.
    c.stick.deadzone = 0.05;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_8bitdo() {
        assert!(suggested_preset("8BitDo 64 BT").is_some());
        assert!(suggested_preset("N64 Controller (Vendor: 057e Product: 2019)").is_some());
        assert!(suggested_preset("Xbox Wireless Controller").is_none());
    }

    #[test]
    fn builtins_have_no_trigger() {
        assert!(
            builtin_macros()
                .iter()
                .all(|m| m.trigger.is_empty() && !m.steps.is_empty())
        );
    }
}

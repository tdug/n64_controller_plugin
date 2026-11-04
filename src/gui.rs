//! Configuration UI (egui). Runs natively (separate process from the emulator) and on the web.

use crate::config::{
    Config, ControllerConfig, Dir, Group, GroupMode, Macro, MacroMode, MacroStep, Modifier, PORTS,
    PakType, Playback,
};
use crate::input::{DeviceSel, Input, InputManager, Snapshot};
use crate::n64::{N64Button, N64State};
use crate::runtime::{Runtime, evaluate_with, self_test};
use crate::stick::{CenterRecorder, SpinRecorder, StickSettings, default_custom_curve, eval_curve};
use eframe::egui::{self, Color32, Ui};
use egui_plot::{Line, Plot, Points};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use web_time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(90, 160, 255);
const LIVE: Color32 = Color32::from_rgb(110, 220, 120);
const WARN: Color32 = Color32::from_rgb(255, 170, 60);

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Button(N64Button),
    Stick(Dir),
    /// Both halves of one axis of a direction group (`true` = horizontal: push right).
    GroupAxis(Group, bool),
    Modifier(usize),
    MacroTrigger(usize),
    PakSwitch,
}

struct Capture {
    port: usize,
    target: Target,
    base: Snapshot,
    started: Instant,
    /// Part of guided setup: index into `wizard_steps()`.
    wizard: Option<usize>,
}

/// One guided-setup prompt.
struct WizStep {
    target: Target,
    label: String,
    /// Adds a second input instead of replacing (e.g. a controller with two Z buttons).
    extra: bool,
}

/// The capture target for one direction of a group.
fn dir_target(g: Group, d: Dir) -> Target {
    match g.button(d) {
        Some(b) => Target::Button(b),
        None => Target::Stick(d),
    }
}

fn dir_word(d: Dir) -> &'static str {
    match d {
        Dir::Up => "up",
        Dir::Down => "down",
        Dir::Left => "left",
        Dir::Right => "right",
    }
}

/// Guided setup order: face/shoulder buttons (with an optional second Z), then the D-pad,
/// C buttons and stick, each following its group's mechanism (an axis step binds both halves).
fn wizard_steps(c: &ControllerConfig) -> Vec<WizStep> {
    use N64Button::*;
    let mut v = vec![];
    for b in [A, B, Z, Start, L, R] {
        v.push(WizStep {
            target: Target::Button(b),
            label: b.label().into(),
            extra: false,
        });
        if b == Z {
            v.push(WizStep {
                target: Target::Button(b),
                label: "a second Z button (Skip if there isn't one)".into(),
                extra: true,
            });
        }
    }
    for g in [Group::DPad, Group::C, Group::Stick] {
        let (hx, vy) = c.group_modes.get(g).axes();
        let name = g.label();
        if vy {
            v.push(WizStep {
                target: Target::GroupAxis(g, false),
                label: format!("{name}: push up (Y axis)"),
                extra: false,
            });
        } else {
            for d in [Dir::Up, Dir::Down] {
                v.push(WizStep {
                    target: dir_target(g, d),
                    label: format!("{name} {}", dir_word(d)),
                    extra: false,
                });
            }
        }
        if hx {
            v.push(WizStep {
                target: Target::GroupAxis(g, true),
                label: format!("{name}: push right (X axis)"),
                extra: false,
            });
        } else {
            for d in [Dir::Left, Dir::Right] {
                v.push(WizStep {
                    target: dir_target(g, d),
                    label: format!("{name} {}", dir_word(d)),
                    extra: false,
                });
            }
        }
    }
    v
}

struct MacroRec {
    idx: usize,
    frames: Vec<N64State>,
    armed: bool,
    acc: f32,
    last: Instant,
}

pub struct App {
    cfg: Config,
    saved: Config,
    mgr: InputManager,
    snap: Snapshot,
    preview: Runtime,
    tab: usize,
    /// Sub-tab within a controller: 0 buttons/stick, 1 pak, 2 modifiers/macros.
    sub: usize,
    capture: Option<Capture>,
    center_rec: Option<(Instant, CenterRecorder)>,
    spin_rec: Option<SpinRecorder>,
    trail: VecDeque<[f32; 2]>,
    macro_sel: usize,
    macro_rec: Option<MacroRec>,
    status: String,
    json: String,
    tests: Vec<(String, bool)>,
    drag_point: Option<usize>,
    controller_presets: Vec<crate::presets::ControllerPreset>,
    presets_error: Option<String>,
    /// What each guided-setup step captured.
    wizard_log: Vec<String>,
    /// Port waiting for a Switch-mode HID device to appear (then assign it + apply its preset).
    hid_assign: Option<usize>,
    /// Instruction shown after choosing a mode the controller isn't in.
    mode_hint: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let cfg = Config::load();
        let (controller_presets, presets_error) = crate::presets::load_controller_presets();
        App {
            saved: cfg.clone(),
            preview: Runtime::new(cfg.clone(), true),
            cfg,
            mgr: InputManager::new(),
            snap: Snapshot::default(),
            tab: 0,
            sub: 0,
            capture: None,
            center_rec: None,
            spin_rec: None,
            trail: VecDeque::new(),
            macro_sel: 0,
            macro_rec: None,
            status: String::new(),
            json: String::new(),
            tests: vec![],
            drag_point: None,
            controller_presets,
            presets_error,
            wizard_log: vec![],
            hid_assign: None,
            mode_hint: None,
        }
    }

    fn save(&mut self) {
        match self.cfg.save() {
            Ok(()) => {
                self.saved = self.cfg.clone();
                self.status = "Saved — the emulator picks changes up live.".into();
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// Feed egui keyboard events into the shared key table (non-Windows; Windows polls directly).
    fn pump_keys(&self, ui: &Ui) {
        if cfg!(windows) {
            return;
        }
        ui.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed, .. } = ev {
                    if let Some(vk) = egui_key_to_vk(*key) {
                        crate::input::set_event_key(vk, *pressed);
                    }
                }
            }
            crate::input::set_event_key(0xA0, i.modifiers.shift);
            crate::input::set_event_key(0xA2, i.modifiers.ctrl);
            crate::input::set_event_key(0xA4, i.modifiers.alt);
        });
    }

    /// Once the requested Switch-mode HID device shows up, select it and apply its preset.
    fn finish_hid_assign(&mut self) {
        let Some(port) = self.hid_assign else { return };
        let Some(d) = self
            .snap
            .devices
            .iter()
            .find(|d| d.is_hid && crate::presets::is_switch_n64(d))
        else {
            return;
        };
        let sel = self.snap.select(d);
        let c = &mut self.cfg.controllers[port];
        c.device = sel;
        if let Some(p) = crate::presets::suggested_preset(&self.controller_presets, d) {
            p.apply(c);
            self.status = format!("Using {} with “{}”", d.name, p.name);
        }
        self.hid_assign = None;
    }

    /// Move guided setup on to step `next` (or finish).
    fn wizard_goto(&mut self, port: usize, next: usize) {
        let steps = wizard_steps(&self.cfg.controllers[port]);
        if next < steps.len() {
            self.capture = Some(Capture {
                port,
                target: steps[next].target,
                base: self.snap.clone(),
                started: Instant::now(),
                wizard: Some(next),
            });
        } else {
            self.capture = None;
            self.status = "Guided setup complete".into();
        }
    }

    fn handle_capture(&mut self) {
        let Some(cap) = &self.capture else { return };
        if self.snap.keys[0x1B] {
            if cap.wizard.is_some() {
                self.status = "Guided setup cancelled".into();
            }
            self.capture = None;
            return;
        }
        if cap.wizard.is_none() && cap.started.elapsed() > Duration::from_secs(6) {
            self.capture = None;
            return;
        }
        let Some((input, dev_id)) = self.snap.detect_new(&cap.base) else {
            return;
        };
        let (port, target, wizard) = (cap.port, cap.target, cap.wizard);
        self.capture = None;
        let c = &mut self.cfg.controllers[port];
        // Gamepad input from a device other than the assigned one: assign it.
        if let Some(id) = dev_id {
            let assigned = self.snap.resolve(&c.device, port).map(|d| d.id);
            if assigned != Some(id) {
                if let Some(d) = self.snap.devices.iter().find(|d| d.id == id) {
                    c.device = self.snap.select(d);
                }
            }
        }
        let step = wizard.map(|i| (i, wizard_steps(c).swap_remove(i)));
        let replace = step.as_ref().is_some_and(|(_, s)| !s.extra);
        let keep_keys = |l: &mut Vec<Input>| l.retain(|i| matches!(i, Input::Key { .. }));
        let text = input.label();
        if let Target::GroupAxis(g, horizontal) = target {
            // One push binds both halves of the axis.
            let (pos, neg) = if horizontal {
                (Dir::Right, Dir::Left)
            } else {
                (Dir::Up, Dir::Down)
            };
            let opp = input.opposite();
            for (d, inp) in [(pos, Some(input)), (neg, opp)] {
                let list = c.group_dir(g, d);
                if replace {
                    keep_keys(list);
                } else {
                    list.retain(|i| !i.is_axis()); // a new axis replaces the old one
                }
                if let Some(inp) = inp {
                    if !list.contains(&inp) {
                        list.push(inp);
                    }
                }
            }
        } else {
            let list = match target {
                Target::Button(b) => c.buttons.entry(b).or_default(),
                Target::Stick(d) => c.stick_bindings.get_mut(d),
                Target::Modifier(i) => &mut c.modifiers[i].inputs,
                Target::MacroTrigger(i) => &mut c.macros[i].trigger,
                Target::PakSwitch => &mut c.pak_switch,
                Target::GroupAxis(..) => unreachable!(),
            };
            if replace {
                keep_keys(list);
            }
            if !list.contains(&input) {
                list.push(input);
            }
        }
        if let Some((i, s)) = step {
            self.wizard_log.push(format!("{} ← {text}", s.label));
            self.wizard_goto(port, i + 1);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.pump_keys(ui);
        self.mgr.set_hid_devices(&self.cfg.hid_devices);
        self.mgr.poll(&mut self.snap, true);
        if self.preview.config != self.cfg {
            self.preview.set_config(self.cfg.clone());
        }
        let port = self.tab.min(PORTS - 1);
        self.preview.vi_tick();
        let out = self.preview.poll(port, &self.snap);
        let dev = self
            .snap
            .resolve(&self.cfg.controllers[port].device, port)
            .cloned();
        let (_, raw) = evaluate_with(&self.cfg.controllers[port], &self.snap, dev.as_ref(), 1.0);
        self.trail.push_back(raw);
        while self.trail.len() > 90 {
            self.trail.pop_front();
        }
        self.handle_capture();
        self.finish_hid_assign();
        self.update_recorders(port, raw, out);

        egui::Panel::top("tabs").show(ui, |ui| self.top_bar(ui));
        if self.tab < PORTS && self.sub != 1 {
            egui::Panel::right("stick")
                .exact_size(420.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("stick_scroll")
                        .show(ui, |ui| self.stick_panel(ui, port, raw, out));
                });
        }
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("main_scroll")
                .auto_shrink(false)
                .show(ui, |ui| {
                    if self.tab < PORTS {
                        self.controller_panel(ui, port, dev.is_some());
                    } else if self.tab == PORTS {
                        self.settings_panel(ui);
                    } else {
                        #[cfg(target_arch = "wasm32")]
                        web_debug::panel(ui, &self.snap);
                    }
                });
        });
        ui.ctx().request_repaint();
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading("N64 Input");
            ui.separator();
            for i in 0..PORTS {
                let plugged = self.cfg.controllers[i].plugged;
                let label = format!("{} Controller {}", if plugged { "●" } else { "○" }, i + 1);
                ui.selectable_value(&mut self.tab, i, label);
            }
            ui.selectable_value(&mut self.tab, PORTS, "⚙ Settings");
            if cfg!(target_arch = "wasm32") {
                ui.selectable_value(&mut self.tab, PORTS + 1, "🔬 Debug");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = self.cfg != self.saved;
                if ui
                    .add_enabled(dirty, egui::Button::new("💾 Save"))
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add_enabled(dirty, egui::Button::new("↺ Revert"))
                    .clicked()
                {
                    self.cfg = self.saved.clone();
                }
                if dirty {
                    ui.colored_label(WARN, "unsaved changes");
                } else {
                    ui.weak(&self.status);
                }
            });
        });
        ui.add_space(4.0);
    }

    fn start_capture(&mut self, port: usize, target: Target) {
        self.capture = Some(Capture {
            port,
            target,
            base: self.snap.clone(),
            started: Instant::now(),
            wizard: None,
        });
    }

    fn controller_panel(&mut self, ui: &mut Ui, port: usize, has_dev: bool) {
        let mut want_capture = None;
        let snap = &self.snap;
        let dev = snap.resolve(&self.cfg.controllers[port].device, port);
        let cap = self
            .capture
            .as_ref()
            .filter(|c| c.port == port)
            .map(|c| c.target);
        let wiz = self
            .capture
            .as_ref()
            .filter(|c| c.port == port)
            .and_then(|c| c.wizard);
        let mut start_wizard = false;
        let mut wiz_action: Option<usize> = None;
        let c = &mut self.cfg.controllers[port];

        ui.horizontal(|ui| {
            ui.checkbox(&mut c.plugged, "Plugged in");
            ui.separator();
            ui.label("Gamepad:");
            egui::ComboBox::from_id_salt("device")
                .width(260.0)
                .selected_text(device_label(&c.device, snap, port))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut c.device,
                        DeviceSel::Auto,
                        format!("Auto (gamepad #{})", port + 1),
                    );
                    ui.selectable_value(&mut c.device, DeviceSel::None, "None (keyboard only)");
                    for d in &snap.devices {
                        let sel = snap.select(d);
                        let txt = format!("{}{}", d.name, if d.ff { "  (rumble)" } else { "" });
                        ui.selectable_value(&mut c.device, sel, txt);
                    }
                });
            if !has_dev && c.device != DeviceSel::None {
                ui.colored_label(WARN, "not connected");
            }
        });
        // Switch-protocol pad seen through the OS/browser gamepad layer: offer raw HID instead.
        let mut use_hid = false;
        if dev.is_some_and(|d| !d.is_hid && crate::presets::is_switch_n64(d)) {
            let hid_dev = snap
                .devices
                .iter()
                .find(|d| d.is_hid && crate::presets::is_switch_n64(d));
            egui::Frame::new()
                .fill(Color32::from_rgb(70, 45, 10))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.colored_label(
                        WARN,
                        egui::RichText::new("⚠ Controller in Switch (S) mode").strong(),
                    );
                    ui.label(if cfg!(target_arch = "wasm32") {
                        "This is an 8BitDo 64 with its switch on S (or Nintendo's Switch Online N64 \
                         controller). The browser's gamepad layer can't read it, but raw HID can. \
                         Click below and pick the controller in the browser's prompt (Chrome/Edge)."
                    } else {
                        "This is an 8BitDo 64 with its switch on S (or Nintendo's Switch Online N64 \
                         controller). Reading it as raw HID gives the most reliable mapping on every OS."
                    });
                    ui.horizontal(|ui| {
                        let label = if hid_dev.is_some() {
                            "🔌 Use the HID device"
                        } else if cfg!(target_arch = "wasm32") {
                            "🔌 Connect via HID…"
                        } else {
                            "🔌 Use HID (waiting for device…)"
                        };
                        if ui.button(label).clicked() {
                            use_hid = true;
                        }
                        ui.weak("…or move the switch to D.");
                    });
                });
        }
        if snap.devices.is_empty() && cfg!(target_arch = "wasm32") {
            ui.weak("Browser: press a button on your gamepad so the page can see it.");
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (i, name) in [
                "🎮 Buttons / Stick",
                "💾 Controller Pak",
                "⚡ Modifiers / Macros",
            ]
            .into_iter()
            .enumerate()
            {
                ui.selectable_value(&mut self.sub, i, egui::RichText::new(name).size(15.0));
            }
        });
        ui.separator();
        let sub = self.sub;

        section_if(ui, sub == 1, "Controller Pak", |ui| {
            egui::Grid::new("pak")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Pak");
                    pak_combo(ui, "pak", &mut c.pak);
                    ui.end_row();
                    ui.label("Hotkey swaps to");
                    pak_combo(ui, "alt_pak", &mut c.alt_pak);
                    ui.end_row();
                    ui.label("Swap hotkey");
                    if bind_cell(ui, &mut c.pak_switch, cap == Some(Target::PakSwitch), |i| {
                        snap.value(i, dev) > 0.5
                    }) {
                        want_capture = Some(Target::PakSwitch);
                    }
                    ui.end_row();
                    ui.label("Rumble strength");
                    ui.add(egui::Slider::new(&mut c.rumble_strength, 0.0..=1.0).show_value(true));
                    ui.end_row();
                });
            if c.pak.has_rumble() || c.alt_pak.has_rumble() {
                match dev {
                    Some(d) if d.ff => ui.weak("Host gamepad supports rumble ✔"),
                    Some(_) => ui.colored_label(WARN, "This gamepad reports no rumble support"),
                    None => ui.weak("No gamepad: rumble has nowhere to go"),
                };
            }
            if c.pak == PakType::EmulatorMemory {
                ui.weak("The emulator stores this Controller Pak itself (no rumble).");
            }
        });

        section_if(ui, sub == 0, "Quick setup", |ui| {
            if let Some(i) = wiz {
                let steps = wizard_steps(c);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(format!(
                            "Step {}/{}: press {}",
                            i + 1,
                            steps.len(),
                            steps[i].label
                        ))
                        .color(ACCENT)
                        .strong(),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("⏭ Skip").clicked() {
                        wiz_action = Some(i + 1);
                    }
                    if i > 0 && ui.button("⏮ Back").clicked() {
                        wiz_action = Some(i - 1);
                    }
                    if ui.button("✖ Cancel").clicked() {
                        wiz_action = Some(usize::MAX);
                    }
                    ui.weak("Esc also cancels");
                });
                for line in self.wizard_log.iter().rev().take(4) {
                    ui.weak(format!("✔ {line}"));
                }
                return;
            }
            let presets = &self.controller_presets;
            let suggested = dev.and_then(|d| crate::presets::suggested_preset(presets, d));
            let family = dev
                .map(|d| crate::presets::family_of(presets, d))
                .unwrap_or_default();
            if let Some(cur) = suggested.filter(|_| !family.is_empty()) {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{} mode:", cur.family)).strong());
                    let mut modes: Vec<&str> = family.iter().map(|p| p.mode.as_str()).collect();
                    modes.sort();
                    modes.dedup();
                    for m in modes {
                        let on = m == cur.mode;
                        if ui.selectable_label(on, format!("  {m}  ")).clicked() {
                            if on {
                                cur.apply(c);
                                self.mode_hint = None;
                            } else {
                                self.mode_hint = Some(format!(
                                    "Move the switch on the back of the controller to {m}, then press a button. \
                                     The detected mode and its preset follow automatically."
                                ));
                            }
                        }
                    }
                    ui.weak(format!("detected: {} ({})", cur.mode, cur.name));
                });
                if let Some(h) = &self.mode_hint {
                    ui.colored_label(ACCENT, h);
                }
            }
            ui.horizontal_wrapped(|ui| {
                if let Some(p) = suggested {
                    if ui
                        .button(egui::RichText::new(format!("✨ Apply “{}”", p.name)).strong())
                        .on_hover_text(&p.note)
                        .clicked()
                    {
                        p.apply(c);
                    }
                }
                ui.menu_button("Controller presets", |ui| {
                    if presets.is_empty() {
                        ui.weak("No presets for this platform.");
                    }
                    for p in presets {
                        let label = if p.mode.is_empty() {
                            p.name.clone()
                        } else {
                            format!("[{}] {}", p.mode, p.name)
                        };
                        if ui.button(label).on_hover_text(&p.note).clicked() {
                            p.apply(c);
                            ui.close();
                        }
                    }
                });
                if ui
                    .button("🧭 Guided setup")
                    .on_hover_text("Press each N64 button on your controller when asked. Works with any controller or adapter. Keyboard keys are kept.")
                    .clicked()
                {
                    start_wizard = true;
                }
                if !cfg!(target_arch = "wasm32")
                    && ui
                    .button("📤 Export bindings as YAML preset")
                    .on_hover_text("Copies this controller's gamepad bindings as a preset entry. Paste it under `presets:` in controllers.yml.")
                    .clicked()
                {
                    let name = dev.map_or("My controller".to_string(), |d| d.name.clone());
                    let yaml = crate::presets::ControllerPreset::from_bindings(&name, c, dev).to_yaml();
                    ui.ctx().copy_text(yaml.clone());
                    self.json = yaml;
                    self.status = "Preset YAML copied to clipboard".into();
                }
            });
            if let Some(e) = &self.presets_error {
                ui.colored_label(Color32::RED, format!("Preset file error: {e}"));
            }
            if !cfg!(target_arch = "wasm32") && self.json.starts_with("- name:") {
                ui.label("Exported preset (also on your clipboard):");
                ui.add(
                    egui::TextEdit::multiline(&mut self.json)
                        .code_editor()
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
            }
        });

        section_if(ui, sub == 0, "Buttons", |ui| {
            egui::Grid::new("buttons")
                .num_columns(2)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    use N64Button::*;
                    for b in [A, B, Z, Start, L, R] {
                        let list = c.buttons.entry(b).or_default();
                        let active = snap.any_pressed(list, dev);
                        ui.label(
                            egui::RichText::new(b.label())
                                .color(if active { LIVE } else { Color32::GRAY })
                                .strong(),
                        );
                        if bind_cell(ui, list, cap == Some(Target::Button(b)), |i| {
                            snap.value(i, dev) > 0.5
                        }) {
                            want_capture = Some(Target::Button(b));
                        }
                        ui.end_row();
                    }
                });
        });

        for g in [Group::DPad, Group::C, Group::Stick] {
            section_if(ui, sub == 0, g.label(), |ui| {
                if let Some(t) = group_editor(ui, g, c, snap, dev, cap) {
                    want_capture = Some(t);
                }
            });
        }

        section_if(
            ui,
            sub == 2,
            "Modifiers (scale the stick while held — e.g. walk)",
            |ui| {
                let mut remove = None;
                for (i, m) in c.modifiers.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let active = snap.any_pressed(&m.inputs, dev);
                        ui.label(egui::RichText::new(format!("#{}", i + 1)).color(if active {
                            LIVE
                        } else {
                            Color32::GRAY
                        }));
                        ui.add(egui::Slider::new(&mut m.scale, 0.0..=1.5).text("scale"));
                        ui.checkbox(&mut m.toggle, "toggle");
                        if bind_cell(ui, &mut m.inputs, cap == Some(Target::Modifier(i)), |x| {
                            snap.value(x, dev) > 0.5
                        }) {
                            want_capture = Some(Target::Modifier(i));
                        }
                        if ui.small_button("🗑").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    c.modifiers.remove(i);
                }
                if ui.button("➕ Add modifier").clicked() {
                    c.modifiers.push(Modifier::default());
                }
            },
        );

        let mut rec_request = None;
        section_if(ui, sub == 2, "Macros", |ui| {
            ui.weak("A macro is a frame-by-frame sequence of N64 inputs (buttons + analog stick). One frame = one game frame: the macro advances only when the game reads the controller on a new frame, so it follows the game's own frame rate.");
            ui.horizontal_wrapped(|ui| {
                for (i, m) in c.macros.iter().enumerate() {
                    let playing = snap.all_pressed(&m.trigger, dev);
                    let txt = egui::RichText::new(&m.name).color(if playing {
                        LIVE
                    } else {
                        Color32::LIGHT_GRAY
                    });
                    if ui.selectable_label(self.macro_sel == i, txt).clicked() {
                        self.macro_sel = i;
                    }
                }
                if ui.button("➕ New").clicked() {
                    c.macros.push(Macro {
                        name: format!("Macro {}", c.macros.len() + 1),
                        ..Default::default()
                    });
                    self.macro_sel = c.macros.len() - 1;
                }
                ui.menu_button("📋 Add from preset", |ui| {
                    let mut add = None;
                    ui.label(egui::RichText::new("Built-in").weak());
                    for m in crate::presets::builtin_macros() {
                        if ui.button(&m.name).on_hover_text(&m.notes).clicked() {
                            add = Some(m);
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Saved").weak());
                    if self.cfg.macro_presets.is_empty() {
                        ui.weak("None yet: use “Save as preset”.");
                    }
                    let mut remove = None;
                    for (i, m) in self.cfg.macro_presets.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button(&m.name).on_hover_text(&m.notes).clicked() {
                                add = Some(m.clone());
                            }
                            if ui
                                .small_button("🗑")
                                .on_hover_text("Delete preset")
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        self.cfg.macro_presets.remove(i);
                    }
                    if let Some(m) = add {
                        c.macros.push(m);
                        self.macro_sel = c.macros.len() - 1;
                        ui.close();
                    }
                });
            });
            let sel = self.macro_sel;
            if sel >= c.macros.len() {
                return;
            }
            ui.separator();
            let mut delete = false;
            {
                let m = &mut c.macros[sel];
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut m.name);
                    if ui
                        .button("💾 Save as preset")
                        .on_hover_text("Saves this macro (without its trigger) under its name. Same name overwrites.")
                        .clicked()
                    {
                        let mut p = m.clone();
                        p.trigger.clear();
                        let presets = &mut self.cfg.macro_presets;
                        match presets.iter_mut().find(|x| x.name == p.name) {
                            Some(x) => *x = p,
                            None => presets.push(p),
                        }
                        self.status = format!("Saved preset “{}”", m.name);
                    }
                    if ui.button("🗑 Delete macro").clicked() {
                        delete = true;
                    }
                });
                ui.add(
                    egui::TextEdit::multiline(&mut m.notes)
                        .hint_text("Notes")
                        .desired_rows(1)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    ui.label("Trigger (all held)");
                    if bind_cell(
                        ui,
                        &mut m.trigger,
                        cap == Some(Target::MacroTrigger(sel)),
                        |i| snap.value(i, dev) > 0.5,
                    ) {
                        want_capture = Some(Target::MacroTrigger(sel));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Mode");
                    ui.selectable_value(&mut m.mode, MacroMode::Merge, "Merge with live input");
                    ui.selectable_value(&mut m.mode, MacroMode::Override, "Override live input");
                });
                ui.horizontal(|ui| {
                    ui.label("Playback");
                    ui.selectable_value(&mut m.playback, Playback::Once, "Once per press");
                    ui.selectable_value(&mut m.playback, Playback::WhileHeld, "Loop while held");
                    ui.selectable_value(&mut m.playback, Playback::Toggle, "Toggle loop");
                });
                let total: u32 = m.steps.iter().map(|s| s.frames.max(1)).sum();
                ui.horizontal(|ui| {
                    ui.label(format!("{} steps, {} game frames", m.steps.len(), total));
                    let recording = self.macro_rec.as_ref().is_some_and(|r| r.idx == sel);
                    if recording {
                        if ui.button(egui::RichText::new("⏹ Stop recording").color(WARN)).clicked() {
                            rec_request = Some(false);
                        }
                    } else if ui.button("⏺ Record from controller").on_hover_text("Records this controller's live N64 output at 60 frames/s. Starts at your first input.").clicked() {
                        rec_request = Some(true);
                    }
                });
                macro_steps(ui, &mut m.steps);
            }
            if delete {
                c.macros.remove(sel);
                self.macro_sel = sel.saturating_sub(1);
            }
        });

        match rec_request {
            Some(true) => {
                self.macro_rec = Some(MacroRec {
                    idx: self.macro_sel,
                    frames: vec![],
                    armed: false,
                    acc: 0.0,
                    last: Instant::now(),
                })
            }
            Some(false) => self.finish_macro_rec(port),
            None => {}
        }
        if let Some(t) = want_capture {
            self.start_capture(port, t);
        }
        if use_hid {
            #[cfg(target_arch = "wasm32")]
            crate::input::web_hid::request();
            self.hid_assign = Some(port);
        }
        if start_wizard {
            self.wizard_log.clear();
            self.wizard_goto(port, 0);
        }
        match wiz_action {
            Some(usize::MAX) => {
                self.capture = None;
                self.status = "Guided setup cancelled".into();
            }
            Some(next) => self.wizard_goto(port, next),
            None => {}
        }
    }

    fn update_recorders(&mut self, port: usize, raw: [f32; 2], out: N64State) {
        if let Some((t, rec)) = &mut self.center_rec {
            rec.add(raw);
            if t.elapsed() > Duration::from_millis(1500) {
                if let Some((center, noise)) = rec.finish() {
                    let s = &mut self.cfg.controllers[port].stick;
                    let mut cal = s.calibration.clone().unwrap_or(crate::stick::Calibration {
                        center,
                        radii: vec![1.0; crate::stick::BINS],
                    });
                    cal.center = center;
                    s.calibration = Some(cal);
                    let reach = s.reach(0.0).max(0.1);
                    s.deadzone = ((noise / reach) * 1.5 + 0.02).clamp(0.02, 0.4);
                    self.status = format!(
                        "Center recorded; deadzone set to {:.0}%",
                        s.deadzone * 100.0
                    );
                }
                self.center_rec = None;
            }
        }
        if let Some(rec) = &mut self.spin_rec {
            rec.add(raw);
        }
        if let Some(rec) = &mut self.macro_rec {
            let now = Instant::now();
            rec.acc += (now - rec.last).as_secs_f32();
            rec.last = now;
            while rec.acc >= 1.0 / 60.0 {
                rec.acc -= 1.0 / 60.0;
                if !rec.armed && out != N64State::default() {
                    rec.armed = true;
                }
                if rec.armed {
                    rec.frames.push(out);
                }
            }
        }
    }

    fn finish_macro_rec(&mut self, port: usize) {
        let Some(rec) = self.macro_rec.take() else {
            return;
        };
        let mut steps: Vec<MacroStep> = vec![];
        for f in rec.frames {
            let stick = (f.x != 0 || f.y != 0).then_some([f.x, f.y]);
            match steps.last_mut() {
                Some(s) if s.buttons == f.buttons && s.stick == stick => s.frames += 1,
                _ => steps.push(MacroStep {
                    buttons: f.buttons,
                    stick,
                    frames: 1,
                }),
            }
        }
        // Trailing neutral time isn't useful.
        while steps
            .last()
            .is_some_and(|s| s.buttons == 0 && s.stick.is_none())
        {
            steps.pop();
        }
        if let Some(m) = self.cfg.controllers[port].macros.get_mut(rec.idx) {
            if !steps.is_empty() {
                m.steps = steps;
                m.mode = MacroMode::Override;
            }
        }
    }

    fn stick_panel(&mut self, ui: &mut Ui, port: usize, raw: [f32; 2], out: N64State) {
        ui.add_space(4.0);
        ui.heading(if self.sub == 2 {
            "Live output"
        } else {
            "Analog stick"
        });
        let s = &self.cfg.controllers[port].stick;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label("Host input");
                draw_raw(ui, s, raw, &self.trail, self.spin_rec.as_ref());
            });
            ui.vertical(|ui| {
                ui.label("N64 output");
                draw_out(ui, s, out);
            });
        });
        let (m, _) = s.shape(raw);
        ui.monospace(format!(
            "raw ({:+.3}, {:+.3}) → N64 ({:+4}, {:+4}) {:3.0}%",
            raw[0],
            raw[1],
            out.x,
            out.y,
            m * 100.0
        ));
        n64_buttons(ui, out);
        ui.separator();
        if self.sub == 2 {
            ui.weak("Hold a macro's trigger to preview it here.");
            return;
        }

        ui.label(egui::RichText::new("Range calibration").strong());
        ui.weak("Record center with the stick released, then spin the stick slowly around its edge a few times.");
        ui.horizontal(|ui| {
            if let Some((t, _)) = &self.center_rec {
                ui.spinner();
                ui.label(format!(
                    "Hold still… {:.1}s",
                    (1.5 - t.elapsed().as_secs_f32()).max(0.0)
                ));
            } else if ui.button("◎ Record center").clicked() {
                self.center_rec = Some((Instant::now(), CenterRecorder::default()));
            }
            match &self.spin_rec {
                Some(rec) => {
                    let cov = rec.coverage();
                    if ui
                        .button(
                            egui::RichText::new(format!("⏹ Finish spin ({:.0}%)", cov * 100.0))
                                .color(WARN),
                        )
                        .clicked()
                    {
                        match rec.finish() {
                            Some(cal) => {
                                self.cfg.controllers[port].stick.calibration = Some(cal);
                                self.status = "Range recorded from spin".into();
                            }
                            None => {
                                self.status =
                                    "Spin covered too little of the circle — try again".into()
                            }
                        }
                        self.spin_rec = None;
                    }
                }
                None => {
                    if ui.button("⟳ Record spin").clicked() {
                        let center = self.cfg.controllers[port].stick.center();
                        self.spin_rec = Some(SpinRecorder::new(center));
                    }
                }
            }
            let s = &mut self.cfg.controllers[port].stick;
            if ui
                .add_enabled(s.calibration.is_some(), egui::Button::new("Reset"))
                .clicked()
            {
                s.calibration = None;
            }
        });
        if let Some(cal) = &self.cfg.controllers[port].stick.calibration {
            let min = cal.radii.iter().copied().fold(f32::MAX, f32::min);
            let max = cal.radii.iter().copied().fold(0.0, f32::max);
            ui.weak(format!(
                "Calibrated: center ({:+.3}, {:+.3}), reach {:.0}%–{:.0}%",
                cal.center[0],
                cal.center[1],
                min * 100.0,
                max * 100.0
            ));
        }
        ui.separator();

        let s = &mut self.cfg.controllers[port].stick;
        egui::Grid::new("stickset").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Deadzone");
            ui.add(egui::Slider::new(&mut s.deadzone, 0.0..=0.5).custom_formatter(pct));
            ui.end_row();
            ui.label("Saturation").on_hover_text("Fraction of the stick's reach that maps to 100%. Raise if your stick hits 100% too early.");
            ui.add(egui::Slider::new(&mut s.saturation, 0.5..=1.0).custom_formatter(pct));
            ui.end_row();
            ui.label("Range").on_hover_text("N64 value at full tilt (cardinal). Real controllers ≈ 80–85.");
            ui.add(egui::Slider::new(&mut s.range, 40.0..=127.0).integer());
            ui.end_row();
            ui.label("Gate");
            ui.horizontal(|ui| {
                use crate::stick::Gate;
                ui.selectable_value(&mut s.gate, Gate::Octagon, "Octagon");
                ui.selectable_value(&mut s.gate, Gate::Circle, "Circle");
                ui.selectable_value(&mut s.gate, Gate::Square, "Square");
            });
            ui.end_row();
            if s.gate == crate::stick::Gate::Octagon {
                ui.label("Diagonal");
                ui.add(egui::Slider::new(&mut s.diagonal, 0.6..=1.0).custom_formatter(pct));
                ui.end_row();
            }
            ui.label("Curve");
            ui.horizontal(|ui| {
                let mut custom = s.custom_curve.is_some();
                if ui.selectable_label(!custom, "Exponent").clicked() {
                    custom = false;
                }
                if ui.selectable_label(custom, "Custom").clicked() {
                    custom = true;
                }
                if custom != s.custom_curve.is_some() {
                    s.custom_curve = custom.then(default_custom_curve);
                }
            });
            ui.end_row();
            if s.custom_curve.is_none() {
                ui.label("Exponent");
                ui.add(egui::Slider::new(&mut s.curve, 0.3..=3.0).logarithmic(true));
                ui.end_row();
            }
        });
        curve_editor(ui, s, &mut self.drag_point, m);
        ui.horizontal(|ui| {
            if ui.button("Reset stick settings").clicked() {
                let cal = s.calibration.take();
                *s = StickSettings {
                    calibration: cal,
                    ..Default::default()
                };
            }
        });
    }

    fn settings_panel(&mut self, ui: &mut Ui) {
        section(ui, "General", |ui| {
            ui.checkbox(
                &mut self.cfg.background_input,
                "Keyboard input while the emulator is in the background",
            );
            ui.checkbox(
                &mut self.cfg.pak_per_game,
                "Separate Controller Pak file for each game",
            );
            #[cfg(not(target_arch = "wasm32"))]
            {
                let dir = crate::platform::config_dir();
                ui.horizontal(|ui| {
                    ui.label("Data folder:");
                    ui.monospace(dir.display().to_string());
                    if ui.button("Open").clicked() {
                        crate::platform::open_folder(&dir);
                    }
                });
                ui.weak("Tip: create a `tdug-n64` folder next to the plugin for portable mode.");
                ui.horizontal(|ui| {
                    let path = crate::presets::user_presets_path();
                    if ui
                        .button("📝 Open presets file")
                        .on_hover_text("Creates controllers.yml in the data folder from the built-in presets (if missing) and opens it.")
                        .clicked()
                    {
                        if !path.exists() {
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::fs::write(&path, crate::presets::BUILTIN_CONTROLLERS_YAML);
                        }
                        crate::platform::open_file(&path);
                    }
                    if ui.button("↻ Reload presets").clicked() {
                        let (p, e) = crate::presets::load_controller_presets();
                        self.controller_presets = p;
                        self.presets_error = e;
                    }
                    ui.weak(format!("{} presets loaded", self.controller_presets.len()));
                });
            }
            if ui.button("Reset everything to defaults").clicked() {
                self.cfg = Config::default();
            }
        });
        section(ui, "Connected gamepads", |ui| {
            if self.snap.devices.is_empty() {
                ui.weak("None detected.");
            }
            for d in &self.snap.devices {
                ui.label(format!(
                    "#{}  {}  {}",
                    d.id,
                    d.name,
                    if d.ff { "· rumble" } else { "" }
                ));
            }
            if let Some(d) = self.snap.devices.first() {
                if d.ff
                    && ui
                        .button("Test rumble on first gamepad (hold)")
                        .is_pointer_button_down_on()
                {
                    self.mgr.set_rumble(d.id, 1.0);
                } else {
                    self.mgr.set_rumble(d.id, 0.0);
                }
            }
        });
        section(ui, "Import / export", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Export current config").clicked() {
                    self.json = self.cfg.to_json();
                }
                if ui.button("Import from text").clicked() {
                    match Config::from_json(&self.json) {
                        Ok(c) => {
                            self.cfg = c;
                            self.status = "Imported (not yet saved)".into();
                        }
                        Err(e) => self.status = format!("Import failed: {e}"),
                    }
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.json)
                    .code_editor()
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            );
        });
        section(
            ui,
            "Self-test (PIF / Controller Pak / Rumble Pak emulation)",
            |ui| {
                if ui.button("▶ Run self-test").clicked() {
                    self.tests = self_test();
                }
                for (name, ok) in &self.tests {
                    ui.colored_label(
                        if *ok { LIVE } else { Color32::RED },
                        format!("{} {name}", if *ok { "✔" } else { "✘" }),
                    );
                }
            },
        );
        ui.weak(crate::config_about());
    }
}

// ---------------------------------------------------------------------------
// Widgets

fn section_if(ui: &mut Ui, show: bool, title: &str, body: impl FnOnce(&mut Ui)) {
    if show {
        section(ui, title, body);
    }
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong().size(15.0))
        .default_open(true)
        .show(ui, body);
    ui.add_space(4.0);
}

fn pct(v: f64, _: std::ops::RangeInclusive<usize>) -> String {
    format!("{:.0}%", v * 100.0)
}

fn pak_combo(ui: &mut Ui, id: &str, v: &mut PakType) {
    egui::ComboBox::from_id_salt(id)
        .width(240.0)
        .selected_text(v.label())
        .show_ui(ui, |ui| {
            for p in PakType::ALL {
                ui.selectable_value(v, p, p.label());
            }
        });
}

/// Binding chips + bind button. Returns true when the user wants to capture a new input.
/// Removable chips for `items`; returns (index to remove, bind requested).
fn chip_row(
    ui: &mut Ui,
    items: &[Input],
    capturing: bool,
    active: impl Fn(&Input) -> bool,
    chip_text: impl Fn(&Input) -> String,
) -> (Option<usize>, bool) {
    let (mut remove, mut want) = (None, false);
    ui.horizontal_wrapped(|ui| {
        for (i, inp) in items.iter().enumerate() {
            let chip = egui::Button::new(format!("{}  ✕", chip_text(inp)))
                .small()
                .fill(if active(inp) {
                    Color32::from_rgb(40, 80, 50)
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                });
            if ui.add(chip).on_hover_text("Click to remove").clicked() {
                remove = Some(i);
            }
        }
        if capturing {
            ui.colored_label(ACCENT, "Push / press it now… (Esc cancels)");
        } else if ui.small_button("➕ Bind").clicked() {
            want = true;
        }
    });
    (remove, want)
}

/// Editor for a four-direction group (stick, D-pad, C buttons) in its chosen mechanism.
/// Returns a capture request.
fn group_editor(
    ui: &mut Ui,
    g: Group,
    c: &mut ControllerConfig,
    snap: &Snapshot,
    dev: Option<&crate::input::DeviceState>,
    cap: Option<Target>,
) -> Option<Target> {
    let mut want = None;
    ui.horizontal(|ui| {
        ui.label("Mechanism");
        let m = c.group_modes.get_mut(g);
        egui::ComboBox::from_id_salt(("group_mode", g as u8))
            .width(240.0)
            .selected_text(m.label())
            .show_ui(ui, |ui| {
                for opt in GroupMode::ALL {
                    ui.selectable_value(m, opt, opt.label());
                }
            });
        if g == Group::Stick || g == Group::C {
            if ui.small_button("Left stick").clicked() {
                bind_pad_stick(c, g, false);
            }
            if ui.small_button("Right stick").clicked() {
                bind_pad_stick(c, g, true);
            }
        }
    });
    let (hx, vy) = c.group_modes.get(g).axes();
    let on = |i: &Input| snap.value(i, dev) > 0.5;
    egui::Grid::new(("group", g as u8))
        .num_columns(2)
        .striped(true)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for (axis, horizontal, pos, neg) in [
                (vy, false, Dir::Up, Dir::Down),
                (hx, true, Dir::Right, Dir::Left),
            ] {
                let negs = c.group_dir(g, neg).clone();
                let pair = |i: &Input| i.opposite().is_some_and(|o| negs.contains(&o));
                if axis {
                    // Axis row: inputs on `pos` whose opposite is on `neg`.
                    let items: Vec<Input> = c
                        .group_dir(g, pos)
                        .iter()
                        .filter(|i| pair(i))
                        .cloned()
                        .collect();
                    let live = items
                        .iter()
                        .any(|i| on(i) || i.opposite().is_some_and(|o| on(&o)));
                    ui.label(
                        egui::RichText::new(if horizontal { "X axis" } else { "Y axis" })
                            .color(if live { LIVE } else { Color32::GRAY })
                            .strong(),
                    );
                    let target = Target::GroupAxis(g, horizontal);
                    let (rm, w) = chip_row(
                        ui,
                        &items,
                        cap == Some(target),
                        |i| on(i) || i.opposite().is_some_and(|o| on(&o)),
                        |i| format!("{}  ⇄", i.label()),
                    );
                    if let Some(k) = rm {
                        let inp = items[k].clone();
                        let opp = inp.opposite();
                        c.group_dir(g, pos).retain(|x| *x != inp);
                        c.group_dir(g, neg).retain(|x| Some(x) != opp.as_ref());
                    }
                    if w {
                        want = Some(target);
                    }
                    ui.end_row();
                }
                // Direction rows: always in button mode; in axis mode only for leftovers.
                for d in [pos, neg] {
                    let other = c.group_dir(g, if d == pos { neg } else { pos }).clone();
                    let list = c.group_dir(g, d);
                    let paired = |i: &Input| i.opposite().is_some_and(|o| other.contains(&o));
                    let items: Vec<Input> = list
                        .iter()
                        .filter(|i| !axis || !paired(i))
                        .cloned()
                        .collect();
                    if axis && items.is_empty() {
                        continue;
                    }
                    let active = items.iter().any(&on);
                    let label = format!("{} {}", if axis { "  also" } else { "" }, dir_word(d));
                    ui.label(
                        egui::RichText::new(label.trim().to_string())
                            .color(if active { LIVE } else { Color32::GRAY })
                            .strong(),
                    );
                    let target = dir_target(g, d);
                    let (rm, w) = chip_row(ui, &items, cap == Some(target), &on, |i| i.label());
                    if let Some(k) = rm {
                        let inp = items[k].clone();
                        c.group_dir(g, d).retain(|x| *x != inp);
                    }
                    if w {
                        want = Some(target);
                    }
                    ui.end_row();
                }
            }
        });
    want
}

/// Bind a gamepad stick's axes to a group (and switch it to dual-axis).
fn bind_pad_stick(c: &mut ControllerConfig, g: Group, right: bool) {
    use crate::input::PadAxis;
    let (x, y) = if right {
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
        let l = c.group_dir(g, d);
        l.retain(|i| !i.is_axis());
        l.push(inp);
    }
    *c.group_modes.get_mut(g) = GroupMode::DualAxis;
}

/// Binding chips + bind button. Each chip lights up only while *its* input is active.
fn bind_cell(
    ui: &mut Ui,
    list: &mut Vec<Input>,
    capturing: bool,
    active: impl Fn(&Input) -> bool,
) -> bool {
    let mut want = false;
    ui.horizontal_wrapped(|ui| {
        let mut remove = None;
        for (i, inp) in list.iter().enumerate() {
            let chip = egui::Button::new(format!("{}  ✕", inp.label()))
                .small()
                .fill(if active(inp) {
                    Color32::from_rgb(40, 80, 50)
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                });
            if ui.add(chip).on_hover_text("Click to remove").clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            list.remove(i);
        }
        if capturing {
            ui.colored_label(ACCENT, "Press a key / button / axis… (Esc cancels)");
        } else if ui.small_button("➕ Bind").clicked() {
            want = true;
        }
    });
    want
}

fn device_label(sel: &DeviceSel, snap: &Snapshot, port: usize) -> String {
    match sel {
        DeviceSel::Auto => match snap.resolve(sel, port) {
            Some(d) => format!("Auto → {}", d.name),
            None => format!("Auto (gamepad #{})", port + 1),
        },
        DeviceSel::None => "None (keyboard only)".into(),
        DeviceSel::Specific { name, nth, .. } => {
            if *nth > 0 {
                format!("{name} #{}", nth + 1)
            } else {
                name.clone()
            }
        }
    }
}

fn macro_steps(ui: &mut Ui, steps: &mut Vec<MacroStep>) {
    enum Op {
        Up(usize),
        Down(usize),
        Dup(usize),
        Del(usize),
    }
    let mut op = None;
    egui::ScrollArea::horizontal()
        .id_salt("steps")
        .show(ui, |ui| {
            egui::Grid::new("steps")
                .striped(true)
                .spacing([4.0, 3.0])
                .show(ui, |ui| {
                    ui.label("#");
                    ui.label("frames");
                    for b in N64Button::ALL {
                        ui.label(b.short());
                    }
                    ui.label("Stick (x, y)").on_hover_text("Tick to set the analog stick on this step (−128..127, +y = up). Unticked: Merge keeps live stick, Override centers it.");
                    ui.label("");
                    ui.end_row();
                    for (i, s) in steps.iter_mut().enumerate() {
                        ui.weak(format!("{}", i + 1));
                        ui.add(
                            egui::DragValue::new(&mut s.frames)
                                .range(1..=9999)
                                .speed(0.2),
                        );
                        for b in N64Button::ALL {
                            let mut on = s.buttons & b.mask() != 0;
                            if ui.checkbox(&mut on, "").on_hover_text(b.label()).changed() {
                                s.buttons ^= b.mask();
                            }
                        }
                        ui.horizontal(|ui| {
                            let mut has = s.stick.is_some();
                            if ui.checkbox(&mut has, "set").changed() {
                                s.stick = has.then_some([0, 0]);
                            }
                            if let Some(st) = &mut s.stick {
                                ui.add(
                                    egui::DragValue::new(&mut st[0])
                                        .range(-128..=127)
                                        .prefix("x "),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut st[1])
                                        .range(-128..=127)
                                        .prefix("y "),
                                );
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.small_button("⬆").clicked() {
                                op = Some(Op::Up(i));
                            }
                            if ui.small_button("⬇").clicked() {
                                op = Some(Op::Down(i));
                            }
                            if ui.small_button("⧉").on_hover_text("Duplicate").clicked() {
                                op = Some(Op::Dup(i));
                            }
                            if ui.small_button("🗑").clicked() {
                                op = Some(Op::Del(i));
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    match op {
        Some(Op::Up(i)) if i > 0 => steps.swap(i, i - 1),
        Some(Op::Down(i)) if i + 1 < steps.len() => steps.swap(i, i + 1),
        Some(Op::Dup(i)) => {
            let s = steps[i].clone();
            steps.insert(i + 1, s);
        }
        Some(Op::Del(i)) => {
            steps.remove(i);
        }
        _ => {}
    }
    if ui.button("➕ Add step").clicked() {
        steps.push(MacroStep::default());
    }
}

fn n64_buttons(ui: &mut Ui, out: N64State) {
    ui.horizontal_wrapped(|ui| {
        for b in N64Button::ALL {
            let on = out.pressed(b);
            let txt = egui::RichText::new(b.short()).monospace().color(if on {
                Color32::BLACK
            } else {
                Color32::GRAY
            });
            let btn =
                egui::Button::new(txt)
                    .small()
                    .fill(if on { LIVE } else { Color32::from_gray(40) });
            ui.add_enabled(false, btn);
        }
    });
}

fn ring(f: impl Fn(f32) -> [f64; 2]) -> Vec<[f64; 2]> {
    (0..=96).map(|i| f(i as f32 / 96.0 * TAU)).collect()
}

/// Square, fixed (non-zoomable) plot with symmetric bounds.
fn square_plot(id: &str, half: f64, size: f32) -> Plot<'static> {
    Plot::new(id.to_string())
        .width(size)
        .height(size)
        .data_aspect(1.0)
        .include_x(-half)
        .include_x(half)
        .include_y(-half)
        .include_y(half)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_axes(false)
        .show_x(false)
        .show_y(false)
}

fn draw_raw(
    ui: &mut Ui,
    s: &StickSettings,
    raw: [f32; 2],
    trail: &VecDeque<[f32; 2]>,
    spin: Option<&SpinRecorder>,
) {
    let cen = s.center();
    let polar = |rad: f32, a: f32| {
        [
            (cen[0] + rad * a.cos()) as f64,
            (cen[1] + rad * a.sin()) as f64,
        ]
    };
    square_plot("raw_plot", 1.2, 190.0).show(ui, |p| {
        p.line(Line::new("reach", ring(|a| polar(s.reach(a), a))).color(Color32::from_gray(150)));
        p.line(
            Line::new("deadzone", ring(|a| polar(s.reach(a) * s.deadzone, a)))
                .color(Color32::from_rgb(220, 90, 90)),
        );
        if s.saturation < 0.999 {
            p.line(
                Line::new("saturation", ring(|a| polar(s.reach(a) * s.saturation, a)))
                    .color(Color32::YELLOW),
            );
        }
        if let Some(rec) = spin {
            let n = rec.radii.len();
            let pts: Vec<[f64; 2]> = (0..=n)
                .map(|i| {
                    let a = (i % n) as f32 / n as f32 * TAU;
                    let r = rec.radii[i % n];
                    [
                        (rec.center[0] + r * a.cos()) as f64,
                        (rec.center[1] + r * a.sin()) as f64,
                    ]
                })
                .collect();
            p.line(Line::new("spin", pts).color(WARN).width(2.0));
        }
        let trail: Vec<[f64; 2]> = trail.iter().map(|t| [t[0] as f64, t[1] as f64]).collect();
        p.line(Line::new("trail", trail).color(ACCENT.gamma_multiply(0.4)));
        p.points(
            Points::new("input", vec![[raw[0] as f64, raw[1] as f64]])
                .radius(5.0)
                .color(ACCENT),
        );
    });
}

fn draw_out(ui: &mut Ui, s: &StickSettings, out: N64State) {
    square_plot("out_plot", 128.0, 190.0).show(ui, |p| {
        p.line(
            Line::new(
                "gate",
                ring(|a| {
                    let r = s.range * s.gate_radius(a);
                    [(r * a.cos()) as f64, (r * a.sin()) as f64]
                }),
            )
            .color(Color32::from_gray(150)),
        );
        p.points(
            Points::new("output", vec![[out.x as f64, out.y as f64]])
                .radius(5.0)
                .color(LIVE),
        );
    });
}

/// Response curve plot; points draggable in custom mode. Double-click adds, right-click removes.
fn curve_editor(ui: &mut Ui, s: &mut StickSettings, drag: &mut Option<usize>, live: f32) {
    let curve_line: Vec<[f64; 2]> = (0..=100)
        .map(|i| {
            let x = i as f32 / 100.0;
            [x as f64, s.respond(x) as f64]
        })
        .collect();
    let live_pt = [live as f64, s.respond(live) as f64];
    let handles: Option<Vec<[f64; 2]>> = s
        .custom_curve
        .as_ref()
        .map(|c| c.iter().map(|p| [p[0] as f64, p[1] as f64]).collect());
    let resp = Plot::new("curve")
        .height(200.0)
        .width(ui.available_width().min(380.0))
        .include_x(0.0)
        .include_x(1.0)
        .include_y(0.0)
        .include_y(1.0)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .x_axis_label("input")
        .y_axis_label("output")
        .show(ui, |p| {
            p.line(Line::new("linear", vec![[0.0, 0.0], [1.0, 1.0]]).color(Color32::from_gray(70)));
            p.line(Line::new("curve", curve_line).color(ACCENT).width(2.0));
            if let Some(h) = handles {
                p.points(
                    Points::new("points", h)
                        .radius(5.0)
                        .filled(false)
                        .color(Color32::WHITE),
                );
            }
            p.points(Points::new("live", vec![live_pt]).radius(4.0).color(LIVE));
            p.pointer_coordinate()
        });
    let Some(curve) = s.custom_curve.as_mut() else {
        return;
    };
    let r = &resp.response;
    let pointer = resp
        .inner
        .map(|p| [p.x.clamp(0.0, 1.0) as f32, p.y.clamp(0.0, 1.0) as f32]);
    let nearest = |c: &Vec<[f32; 2]>, q: [f32; 2]| {
        c.iter()
            .enumerate()
            .map(|(i, v)| (i, (v[0] - q[0]).hypot(v[1] - q[1])))
            .filter(|(_, d)| *d < 0.06)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    if r.drag_started() {
        *drag = pointer.and_then(|q| nearest(curve, q));
    }
    if let (true, Some(i), Some(q)) = (r.dragged(), *drag, pointer) {
        let lo = if i > 0 { curve[i - 1][0] + 0.01 } else { 0.0 };
        let hi = if i + 1 < curve.len() {
            curve[i + 1][0] - 0.01
        } else {
            1.0
        };
        curve[i] = [q[0].clamp(lo, hi.max(lo)), q[1]];
    }
    if r.drag_stopped() {
        *drag = None;
    }
    if let (true, Some(q)) = (r.double_clicked(), pointer) {
        let at = curve
            .iter()
            .position(|c| c[0] > q[0])
            .unwrap_or(curve.len());
        curve.insert(at, [q[0], eval_curve(curve, q[0])]);
    }
    if let (true, Some(i)) = (
        r.secondary_clicked(),
        pointer.and_then(|q| nearest(curve, q)),
    ) {
        if curve.len() > 2 {
            curve.remove(i);
        }
    }
    ui.weak("Drag points · double-click adds · right-click removes");
    ui.horizontal_wrapped(|ui| {
        ui.label("Presets:");
        if ui.small_button("Linear").clicked() {
            *curve = default_custom_curve();
        }
        if ui.small_button("Precise center").clicked() {
            *curve = vec![[0.0, 0.0], [0.5, 0.3], [0.8, 0.65], [1.0, 1.0]];
        }
        if ui.small_button("Fast start").clicked() {
            *curve = vec![[0.0, 0.0], [0.3, 0.45], [0.7, 0.85], [1.0, 1.0]];
        }
    });
}

/// egui key → Windows VK code (used off-Windows, where we can't poll the keyboard directly).
fn egui_key_to_vk(k: egui::Key) -> Option<u8> {
    use egui::Key as K;
    let name = k.name();
    if name.len() == 1 {
        let c = name.as_bytes()[0].to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c);
        }
    }
    Some(match k {
        K::ArrowDown => 0x28,
        K::ArrowLeft => 0x25,
        K::ArrowRight => 0x27,
        K::ArrowUp => 0x26,
        K::Escape => 0x1B,
        K::Tab => 0x09,
        K::Backspace => 0x08,
        K::Enter => 0x0D,
        K::Space => 0x20,
        K::Insert => 0x2D,
        K::Delete => 0x2E,
        K::Home => 0x24,
        K::End => 0x23,
        K::PageUp => 0x21,
        K::PageDown => 0x22,
        K::Minus => 0xBD,
        K::Equals => 0xBB,
        K::Comma => 0xBC,
        K::Period => 0xBE,
        K::Semicolon => 0xBA,
        K::Slash => 0xBF,
        K::Backslash => 0xDC,
        K::OpenBracket => 0xDB,
        K::CloseBracket => 0xDD,
        K::Backtick => 0xC0,
        K::Quote => 0xDE,
        K::F1 => 0x70,
        K::F2 => 0x71,
        K::F3 => 0x72,
        K::F4 => 0x73,
        K::F5 => 0x74,
        K::F6 => 0x75,
        K::F7 => 0x76,
        K::F8 => 0x77,
        K::F9 => 0x78,
        K::F10 => 0x79,
        K::F11 => 0x7A,
        K::F12 => 0x7B,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Entry points

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TDug N64 Input — Configuration")
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(
        "tdug-n64-config",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    ) {
        crate::platform::log(&format!("config UI failed: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
pub fn run_web() {
    use wasm_bindgen::JsCast;
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("the_canvas_id"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("canvas #the_canvas_id");
        let _ = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(App::new(cc)))),
            )
            .await;
    });
}

/// Browser-only debug tab: raw Gamepad API values and raw WebHID input reports.
#[cfg(target_arch = "wasm32")]
mod web_debug {
    use super::*;
    use crate::input::{PadAxis, PadButton};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(catch, js_name = tdugHidSupported)]
        fn hid_supported() -> Result<bool, JsValue>;
        #[wasm_bindgen(catch, js_name = tdugHidConnect)]
        fn hid_connect() -> Result<js_sys_promise::Promise, JsValue>;
        #[wasm_bindgen(catch, js_name = tdugHidJson)]
        fn hid_json() -> Result<String, JsValue>;
        #[wasm_bindgen(catch, js_name = tdugPadsJson)]
        fn pads_json() -> Result<String, JsValue>;
    }

    mod js_sys_promise {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen]
        extern "C" {
            pub type Promise;
        }
    }

    #[derive(serde::Deserialize)]
    struct HidDev {
        name: String,
        vid: u16,
        pid: u16,
        reports: std::collections::BTreeMap<String, Vec<u8>>,
        count: u64,
    }

    #[derive(serde::Deserialize)]
    struct Pad {
        index: u32,
        id: String,
        mapping: String,
        buttons: Vec<f32>,
        axes: Vec<f32>,
        rumble: bool,
    }

    pub fn panel(ui: &mut Ui, snap: &Snapshot) {
        section(ui, "Browser Gamepad API (unfiltered)", |ui| {
            ui.weak("Every button/axis index the browser reports. After switching a controller's mode (e.g. 8BitDo S/D switch) or reconnecting, press any button so the browser exposes it again.");
            let pads: Vec<Pad> =
                serde_json::from_str(&pads_json().unwrap_or_default()).unwrap_or_default();
            if pads.is_empty() {
                ui.weak("No gamepads exposed yet.");
            }
            for p in &pads {
                ui.label(egui::RichText::new(format!("[{}] {}", p.index, p.id)).strong());
                ui.horizontal(|ui| {
                    ui.monospace(format!("mapping: {}", p.mapping));
                    if p.mapping != "standard" {
                        ui.colored_label(WARN, "non-standard: bind via raw codes or Guided setup");
                    }
                    if p.rumble {
                        ui.weak("· vibrationActuator present");
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("buttons:");
                    for (i, v) in p.buttons.iter().enumerate() {
                        let t = egui::RichText::new(format!("{i}:{v:.1}")).monospace();
                        ui.label(if *v > 0.5 {
                            t.color(LIVE).strong()
                        } else {
                            t.weak()
                        });
                    }
                });
                // Axes that ever read > 1.1 are hats (centred ≈ 1.286).
                let hats_id = ui.scope_id().with(("hats", p.index));
                let mut hats: Vec<usize> = ui.data(|d| d.get_temp(hats_id)).unwrap_or_default();
                ui.horizontal_wrapped(|ui| {
                    ui.label("axes:");
                    for (i, v) in p.axes.iter().enumerate() {
                        if *v > 1.1 && !hats.contains(&i) {
                            hats.push(i);
                        }
                        let t = egui::RichText::new(format!("{i}:{v:+.3}")).monospace();
                        ui.label(if v.abs() > 0.2 && *v <= 1.1 {
                            t.color(LIVE)
                        } else {
                            t
                        });
                        if hats.contains(&i) {
                            let arrows: String = (0..4u8)
                                .filter(|d| crate::input::hat_pressed(*v, *d))
                                .map(|d| ["↑", "→", "↓", "←"][d as usize])
                                .collect();
                            let label = if arrows.is_empty() {
                                "hat: centred".to_string()
                            } else {
                                format!("hat: {arrows}")
                            };
                            ui.colored_label(
                                if arrows.is_empty() {
                                    Color32::GRAY
                                } else {
                                    LIVE
                                },
                                label,
                            );
                        }
                    }
                });
                ui.data_mut(|d| d.insert_temp(hats_id, hats));
                ui.separator();
            }
        });
        section(ui, "Gamepad state as the plugin sees it (gilrs)", |ui| {
            if snap.devices.is_empty() {
                ui.weak(
                    "No gamepads. Press a button on your controller so the browser exposes it.",
                );
            }
            for d in &snap.devices {
                ui.label(egui::RichText::new(format!("#{} {}", d.id, d.name)).strong());
                ui.monospace(format!("uuid {}", d.uuid));
                ui.horizontal_wrapped(|ui| {
                    for (i, b) in PadButton::ALL.iter().enumerate() {
                        let v = d.buttons[i];
                        ui.colored_label(
                            if v > 0.5 { LIVE } else { Color32::GRAY },
                            format!("{}={v:.2}", b.label()),
                        );
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (i, a) in PadAxis::ALL.iter().enumerate() {
                        ui.monospace(format!("{a:?}={:+.3}", d.axes[i]));
                    }
                });
                ui.label("Raw buttons (code=value):");
                ui.horizontal_wrapped(|ui| {
                    for (c, v) in &d.raw_buttons {
                        ui.colored_label(
                            if *v > 0.5 { LIVE } else { Color32::GRAY },
                            format!("{c}={v:.2}"),
                        );
                    }
                });
                ui.label("Raw axes:");
                ui.horizontal_wrapped(|ui| {
                    for (c, v) in &d.raw_axes {
                        ui.monospace(format!("{c}={v:+.3}"));
                    }
                });
                ui.separator();
            }
        });
        section(ui, "Raw HID input reports (WebHID)", |ui| {
            if !hid_supported().unwrap_or(false) {
                ui.colored_label(
                    WARN,
                    "WebHID isn't available in this browser (use Chrome or Edge).",
                );
                return;
            }
            ui.weak("Shows the bytes a HID device sends, before any mapping. Useful for adapters and controllers without a mapping. Bytes that differ from the previous frame are highlighted.");
            if ui.button("🔌 Connect HID device…").clicked() {
                let _ = hid_connect();
            }
            let devs: Vec<HidDev> =
                serde_json::from_str(&hid_json().unwrap_or_default()).unwrap_or_default();
            let prev_id = ui.scope_id().with("hid_prev");
            let prev: Vec<Vec<u8>> = ui.data(|d| d.get_temp(prev_id)).unwrap_or_default();
            let mut now = vec![];
            for d in &devs {
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ({:04x}:{:04x})  {} reports",
                        d.name, d.vid, d.pid, d.count
                    ))
                    .strong(),
                );
                for (id, bytes) in &d.reports {
                    let old = prev.get(now.len());
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(format!("id {id:>3}:"));
                        for (i, b) in bytes.iter().enumerate() {
                            let changed = old.is_some_and(|o| o.get(i) != Some(b));
                            let t = egui::RichText::new(format!("{b:02x}")).monospace();
                            ui.label(if changed { t.color(LIVE) } else { t });
                        }
                    });
                    now.push(bytes.clone());
                }
            }
            ui.data_mut(|d| d.insert_temp(prev_id, now));
        });
    }
}

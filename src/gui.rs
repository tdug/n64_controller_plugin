//! Configuration UI (egui). Runs natively (separate process from the emulator) and on the web.

use crate::config::{Config, Dir, Macro, MacroMode, MacroStep, Modifier, PORTS, PakType, Playback};
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
    Modifier(usize),
    MacroTrigger(usize),
    PakSwitch,
}

struct Capture {
    port: usize,
    target: Target,
    base: Snapshot,
    started: Instant,
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
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let cfg = Config::load();
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

    fn handle_capture(&mut self) {
        let Some(cap) = &self.capture else { return };
        if self.snap.keys[0x1B] || cap.started.elapsed() > Duration::from_secs(6) {
            self.capture = None;
            return;
        }
        let Some((input, dev_id)) = self.snap.detect_new(&cap.base) else {
            return;
        };
        let (port, target) = (cap.port, cap.target);
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
        let list = match target {
            Target::Button(b) => c.buttons.entry(b).or_default(),
            Target::Stick(d) => c.stick_bindings.get_mut(d),
            Target::Modifier(i) => &mut c.modifiers[i].inputs,
            Target::MacroTrigger(i) => &mut c.macros[i].trigger,
            Target::PakSwitch => &mut c.pak_switch,
        };
        if !list.contains(&input) {
            list.push(input);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.pump_keys(ui);
        self.mgr.poll(&mut self.snap, true);
        if self.preview.config != self.cfg {
            self.preview.set_config(self.cfg.clone());
        }
        let port = self.tab.min(PORTS - 1);
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
                    } else {
                        self.settings_panel(ui);
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
                    let active = snap.any_pressed(&c.pak_switch, dev);
                    if bind_cell(
                        ui,
                        &mut c.pak_switch,
                        cap == Some(Target::PakSwitch),
                        active,
                    ) {
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

        section_if(ui, sub == 0, "Buttons", |ui| {
            egui::Grid::new("buttons")
                .num_columns(2)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for b in N64Button::ALL {
                        let list = c.buttons.entry(b).or_default();
                        let active = snap.any_pressed(list, dev);
                        ui.label(
                            egui::RichText::new(b.label())
                                .color(if active { LIVE } else { Color32::GRAY })
                                .strong(),
                        );
                        if bind_cell(ui, list, cap == Some(Target::Button(b)), active) {
                            want_capture = Some(Target::Button(b));
                        }
                        ui.end_row();
                    }
                });
        });

        section_if(ui, sub == 0, "Analog stick inputs", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Use left stick").clicked() {
                    c.stick_bindings.use_pad_stick(false);
                }
                if ui.button("Use right stick").clicked() {
                    c.stick_bindings.use_pad_stick(true);
                }
            });
            egui::Grid::new("stickb")
                .num_columns(2)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for d in Dir::ALL {
                        let list = c.stick_bindings.get_mut(d);
                        let v = snap.max_value(list, dev);
                        ui.label(
                            egui::RichText::new(d.label())
                                .color(if v > 0.1 { LIVE } else { Color32::GRAY })
                                .strong(),
                        );
                        if bind_cell(ui, list, cap == Some(Target::Stick(d)), v > 0.5) {
                            want_capture = Some(Target::Stick(d));
                        }
                        ui.end_row();
                    }
                });
        });

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
                        if bind_cell(ui, &mut m.inputs, cap == Some(Target::Modifier(i)), active) {
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
            ui.weak("A macro is a frame-by-frame sequence of N64 inputs. One frame = one controller poll by the game.");
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
                    if ui.button("🗑 Delete macro").clicked() {
                        delete = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Trigger (all held)");
                    let active = snap.all_pressed(&m.trigger, dev);
                    if bind_cell(
                        ui,
                        &mut m.trigger,
                        cap == Some(Target::MacroTrigger(sel)),
                        active,
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
                    ui.label(format!("{} steps, {} frames (≈{:.2}s at 60 polls/s)", m.steps.len(), total, total as f32 / 60.0));
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
fn bind_cell(ui: &mut Ui, list: &mut Vec<Input>, capturing: bool, active: bool) -> bool {
    let mut want = false;
    ui.horizontal_wrapped(|ui| {
        let mut remove = None;
        for (i, inp) in list.iter().enumerate() {
            let chip = egui::Button::new(format!("{}  ✕", inp.label()))
                .small()
                .fill(if active {
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
                    ui.label("stick");
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
                            if ui.checkbox(&mut has, "").changed() {
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

//! Emulator-independent controller runtime: mapping, macros, modifiers, PIF commands.

use crate::config::{Config, ControllerConfig, Macro, MacroMode, PORTS, PakType, Playback};
use crate::input::Snapshot;
use crate::n64::N64State;
use crate::pak::{Pak, data_crc};
use web_time::{Duration, Instant};

/// How long a swapped pak reads as "unplugged" so games notice the change.
const SWAP_GAP: Duration = Duration::from_millis(600);

/// Map live input (no macros) to N64 state. Also returns the raw stick vector.
pub fn evaluate_with(
    cfg: &ControllerConfig,
    snap: &Snapshot,
    dev: Option<&crate::input::DeviceState>,
    scale: f32,
) -> (N64State, [f32; 2]) {
    let mut buttons = 0u16;
    for (b, inputs) in &cfg.buttons {
        if snap.any_pressed(inputs, dev) {
            buttons |= b.mask();
        }
    }
    let sb = &cfg.stick_bindings;
    let raw = [
        snap.max_value(&sb.right, dev) - snap.max_value(&sb.left, dev),
        snap.max_value(&sb.up, dev) - snap.max_value(&sb.down, dev),
    ];
    let (x, y) = cfg.stick.process(raw, scale);
    (N64State { buttons, x, y }, raw)
}

#[derive(Default, Clone)]
struct MacroPlayer {
    active: bool,
    step: usize,
    frame: u32,
    prev_trigger: bool,
    /// Just (re)started: current step not yet shown.
    fresh: bool,
}

impl MacroPlayer {
    /// `advance`: a new game frame started, so move to the next macro frame.
    fn update(&mut self, m: &Macro, pressed: bool, state: &mut N64State, advance: bool) {
        let edge = pressed && !self.prev_trigger;
        self.prev_trigger = pressed;
        match m.playback {
            Playback::Once => {
                if edge {
                    self.restart();
                }
            }
            Playback::WhileHeld => {
                if edge {
                    self.restart();
                }
                if !pressed {
                    self.active = false;
                }
            }
            Playback::Toggle => {
                if edge {
                    if self.active {
                        self.active = false;
                    } else {
                        self.restart();
                    }
                }
            }
        }
        if !self.active || m.steps.is_empty() {
            self.active = false;
            return;
        }
        if !advance && !self.fresh {
            self.apply(m, state);
            return;
        }
        if !self.fresh {
            self.frame += 1;
            if self.frame >= m.steps[self.step.min(m.steps.len() - 1)].frames.max(1) {
                self.frame = 0;
                self.step += 1;
                if self.step >= m.steps.len() {
                    self.step = 0;
                    if m.playback == Playback::Once {
                        self.active = false;
                        return;
                    }
                }
            }
        }
        self.fresh = false;
        self.apply(m, state);
    }

    fn apply(&self, m: &Macro, state: &mut N64State) {
        let step = &m.steps[self.step.min(m.steps.len() - 1)];
        match m.mode {
            MacroMode::Merge => {
                state.buttons |= step.buttons;
                if let Some([x, y]) = step.stick {
                    state.x = x;
                    state.y = y;
                }
            }
            MacroMode::Override => {
                let [x, y] = step.stick.unwrap_or([0, 0]);
                *state = N64State {
                    buttons: step.buttons,
                    x,
                    y,
                };
            }
        }
    }

    fn restart(&mut self) {
        self.active = true;
        self.step = 0;
        self.frame = 0;
        self.fresh = true;
    }
}

#[derive(Default)]
struct Port {
    pak: Option<Pak>,
    use_alt: bool,
    swap_until: Option<Instant>,
    macros: Vec<MacroPlayer>,
    mod_on: Vec<bool>,
    mod_prev: Vec<bool>,
    switch_prev: bool,
    rumble: bool,
    last: N64State,
    /// A new game frame began since this port's last macro advance.
    frame_ready: bool,
}

pub struct Runtime {
    pub config: Config,
    pub rom_name: String,
    /// Disable pak file I/O (GUI preview).
    pub preview: bool,
    ports: Vec<Port>,
    /// Pending motor changes `(port, on)` for the input thread.
    pub rumble_events: Vec<(usize, bool)>,
    /// Whether the host gives us a frame clock (VI ticks or PIF cycle ends).
    clock_seen: bool,
    vi_clock: bool,
}

impl Runtime {
    pub fn new(config: Config, preview: bool) -> Self {
        let mut rt = Runtime {
            config,
            rom_name: String::new(),
            preview,
            ports: (0..PORTS).map(|_| Port::default()).collect(),
            rumble_events: vec![],
            clock_seen: false,
            vi_clock: false,
        };
        rt.sync_ports();
        rt
    }

    pub fn set_config(&mut self, config: Config) {
        let paks_changed = self
            .config
            .controllers
            .iter()
            .zip(&config.controllers)
            .any(|(a, b)| a.pak != b.pak || a.alt_pak != b.alt_pak)
            || self.config.pak_per_game != config.pak_per_game;
        self.config = config;
        self.sync_ports();
        if paks_changed {
            self.reopen_paks();
        }
    }

    fn sync_ports(&mut self) {
        for (p, c) in self.ports.iter_mut().zip(&self.config.controllers) {
            p.macros.resize(c.macros.len(), MacroPlayer::default());
            p.mod_on.resize(c.modifiers.len(), false);
            p.mod_prev.resize(c.modifiers.len(), false);
        }
    }

    pub fn active_pak(&self, port: usize) -> PakType {
        let c = &self.config.controllers[port];
        if self.ports[port].use_alt {
            c.alt_pak
        } else {
            c.pak
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    fn pak_path(&self, port: usize) -> Option<std::path::PathBuf> {
        if self.preview {
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let name = if self.config.pak_per_game && !self.rom_name.is_empty() {
                format!("{}-P{}.mpk", self.rom_name, port + 1)
            } else {
                format!("Controller{}.mpk", port + 1)
            };
            Some(crate::platform::config_dir().join("paks").join(name))
        }
        #[cfg(target_arch = "wasm32")]
        None
    }

    pub fn reopen_paks(&mut self) {
        for port in 0..PORTS {
            self.ports[port].pak = None; // flushes old
            let kind = self.active_pak(port);
            let path = self.pak_path(port);
            self.ports[port].pak = Some(Pak::new(kind, path));
            self.set_rumble(port, false);
        }
    }

    pub fn rom_open(&mut self, rom_name: String) {
        self.rom_name = rom_name;
        for p in &mut self.ports {
            p.use_alt = false;
            p.swap_until = None;
        }
        self.reopen_paks();
    }

    pub fn rom_closed(&mut self) {
        for port in 0..PORTS {
            self.set_rumble(port, false);
        }
        self.flush_paks();
    }

    pub fn flush_paks(&mut self) {
        for p in &mut self.ports {
            if let Some(pak) = &mut p.pak {
                pak.flush();
            }
        }
    }

    fn set_rumble(&mut self, port: usize, on: bool) {
        if self.ports[port].rumble != on {
            self.ports[port].rumble = on;
            self.rumble_events.push((port, on));
        }
    }

    /// Frame clock: once per video frame (emulator's `GetKeys` cadence).
    /// When present, macros advance at most once per VI *and* only when the game polls,
    /// so they track the game's own frame rate (e.g. 30 fps games poll every other VI).
    pub fn vi_tick(&mut self) {
        self.clock_seen = true;
        self.vi_clock = true;
        for p in &mut self.ports {
            p.frame_ready = true;
        }
    }

    /// Frame clock fallback: end of one PIF transaction (`ReadController(-1)`).
    /// Multiple polls of the same port inside one transaction count as one frame.
    pub fn pif_cycle_end(&mut self) {
        self.clock_seen = true;
        if !self.vi_clock {
            for p in &mut self.ports {
                p.frame_ready = true;
            }
        }
    }

    pub fn last_state(&self, port: usize) -> N64State {
        self.ports[port].last
    }

    /// One game poll of controller `port`: live input + modifiers + macros + hotkeys.
    pub fn poll(&mut self, port: usize, snap: &Snapshot) -> N64State {
        let cfg = &self.config.controllers[port];
        let dev = snap.resolve(&cfg.device, port);
        let p = &mut self.ports[port];

        let mut scale = 1.0;
        for (i, m) in cfg.modifiers.iter().enumerate() {
            let held = snap.any_pressed(&m.inputs, dev);
            if m.toggle {
                if held && !p.mod_prev[i] {
                    p.mod_on[i] = !p.mod_on[i];
                }
            } else {
                p.mod_on[i] = held;
            }
            p.mod_prev[i] = held;
            if p.mod_on[i] {
                scale *= m.scale;
            }
        }

        let (mut state, _) = evaluate_with(cfg, snap, dev, scale);
        let advance = p.frame_ready || !self.clock_seen;
        p.frame_ready = false;
        for (m, player) in cfg.macros.iter().zip(p.macros.iter_mut()) {
            let pressed = snap.all_pressed(&m.trigger, dev);
            player.update(m, pressed, &mut state, advance);
        }
        p.last = state;

        let switch = snap.any_pressed(&cfg.pak_switch, dev);
        let edge = switch && !p.switch_prev;
        p.switch_prev = switch;
        if edge && !self.preview {
            self.swap_pak(port);
        }
        state
    }

    fn swap_pak(&mut self, port: usize) {
        self.set_rumble(port, false);
        let p = &mut self.ports[port];
        p.use_alt = !p.use_alt;
        p.swap_until = Some(Instant::now() + SWAP_GAP);
        p.pak = None;
        let kind = self.active_pak(port);
        let path = self.pak_path(port);
        self.ports[port].pak = Some(Pak::new(kind, path));
        crate::platform::log(&format!("port {} pak -> {:?}", port + 1, kind));
    }

    fn pak_present(&self, port: usize) -> bool {
        let p = &self.ports[port];
        if p.swap_until.is_some_and(|t| Instant::now() < t) {
            return false;
        }
        p.pak.as_ref().is_some_and(|k| k.present())
    }

    /// Process one raw PIF command block for `port`:
    /// `cmd[0]` = tx length, `cmd[1]` = rx length, `cmd[2..]` = tx bytes then rx bytes.
    pub fn pif_command(&mut self, port: usize, cmd: &mut [u8], snap: &Snapshot) {
        if cmd.len() < 3 {
            return;
        }
        if !self.config.controllers[port].plugged {
            cmd[1] |= 0x80;
            return;
        }
        let tx = cmd[0] as usize;
        let rx_at = 2 + tx;
        match cmd[2] {
            0x00 | 0xFF => {
                if cmd.len() >= rx_at + 3 {
                    let present = self.pak_present(port);
                    cmd[rx_at..rx_at + 3].copy_from_slice(&[0x05, 0x00, present as u8]);
                }
            }
            0x01 => {
                let st = self.poll(port, snap);
                if cmd.len() >= rx_at + 4 {
                    cmd[rx_at..rx_at + 4].copy_from_slice(&st.wire());
                }
            }
            0x02 => {
                if cmd.len() >= 5 + 33 {
                    let addr = u16::from_be_bytes([cmd[3], cmd[4]]) & 0xFFE0;
                    let present = self.pak_present(port);
                    let (data, crc) = cmd[5..38].split_at_mut(32);
                    match (&self.ports[port].pak, present) {
                        (Some(pak), true) => pak.read(addr, data),
                        _ => data.fill(0),
                    }
                    crc[0] = data_crc(data) ^ if present { 0 } else { 0xFF };
                }
            }
            0x03 => {
                if cmd.len() >= 5 + 33 {
                    let addr = u16::from_be_bytes([cmd[3], cmd[4]]) & 0xFFE0;
                    let present = self.pak_present(port);
                    let mut data = [0u8; 32];
                    data.copy_from_slice(&cmd[5..37]);
                    if present {
                        let change = self.ports[port]
                            .pak
                            .as_mut()
                            .and_then(|p| p.write(addr, &data));
                        if let Some(on) = change {
                            self.set_rumble(port, on);
                        }
                    }
                    cmd[37] = data_crc(&data) ^ if present { 0 } else { 0xFF };
                }
            }
            _ => cmd[1] |= 0x80,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MacroStep;
    use crate::input::Input;
    use crate::n64::N64Button;

    fn snap_with_key(k: u8, down: bool) -> Snapshot {
        let mut s = Snapshot::default();
        s.keys[k as usize] = down;
        s
    }

    #[test]
    fn keyboard_a_button() {
        let mut rt = Runtime::new(Config::default(), true);
        let st = rt.poll(0, &snap_with_key(b'X', true));
        assert!(st.pressed(N64Button::A));
    }

    #[test]
    fn status_and_read_commands() {
        let mut rt = Runtime::new(Config::default(), true);
        rt.reopen_paks();
        let snap = Snapshot::default();
        let mut cmd = [1u8, 3, 0x00, 0, 0, 0];
        rt.pif_command(0, &mut cmd, &snap);
        assert_eq!(&cmd[3..6], &[0x05, 0x00, 0x01]);

        // Rumble detect on the default Tremor pak.
        let mut w = [0u8; 38];
        w[0] = 35;
        w[1] = 1;
        w[2] = 0x03;
        w[3] = 0x80;
        w[5..37].fill(0x80);
        rt.pif_command(0, &mut w, &snap);
        let mut r = [0u8; 38];
        r[0] = 3;
        r[1] = 33;
        r[2] = 0x02;
        r[3] = 0x80;
        rt.pif_command(0, &mut r, &snap);
        assert_eq!(&r[5..37], &[0x80; 32]);
        assert_eq!(r[37], data_crc(&[0x80; 32]));

        // Motor on
        w[3] = 0xC0;
        w[5..37].fill(1);
        rt.pif_command(0, &mut w, &snap);
        assert_eq!(rt.rumble_events.last(), Some(&(0, true)));

        // Unplugged port
        let mut cmd = [1u8, 3, 0x00, 0, 0, 0];
        rt.pif_command(2, &mut cmd, &snap);
        assert_eq!(cmd[1] & 0x80, 0x80);
    }

    #[test]
    fn macro_once_plays_frames() {
        let mut cfg = Config::default();
        cfg.controllers[0].macros.push(Macro {
            trigger: vec![Input::key(b'M')],
            steps: vec![
                MacroStep {
                    buttons: N64Button::A.mask(),
                    stick: None,
                    frames: 2,
                },
                MacroStep {
                    buttons: N64Button::B.mask(),
                    stick: Some([10, -10]),
                    frames: 1,
                },
            ],
            ..Default::default()
        });
        let mut rt = Runtime::new(cfg, true);
        let down = snap_with_key(b'M', true);
        let up = Snapshot::default();
        let a = N64Button::A.mask();
        let b = N64Button::B.mask();
        let frame = |rt: &mut Runtime, s: &Snapshot| {
            rt.vi_tick();
            let first = rt.poll(0, s);
            // Extra polls within the same frame must not advance the macro.
            assert_eq!(rt.poll(0, s), first);
            first
        };
        assert_eq!(frame(&mut rt, &down).buttons, a);
        assert_eq!(frame(&mut rt, &up).buttons, a);
        let s = frame(&mut rt, &up);
        assert_eq!((s.buttons, s.x, s.y), (b, 10, -10));
        assert_eq!(frame(&mut rt, &up).buttons, 0);
        // A 30 fps game polls every other VI: VIs without a poll don't consume macro frames.
        rt.vi_tick();
        assert_eq!(rt.poll(0, &down).buttons, a);
        rt.vi_tick();
        rt.vi_tick();
        assert_eq!(rt.poll(0, &up).buttons, a);
        rt.vi_tick();
        rt.vi_tick();
        assert_eq!(rt.poll(0, &up).buttons, b);
    }

    #[test]
    fn modifier_scales_stick() {
        let mut rt = Runtime::new(Config::default(), true);
        let mut s = snap_with_key(0x27, true);
        assert_eq!(rt.poll(0, &s).x, 85);
        s.keys[0xA0] = true;
        assert_eq!(rt.poll(0, &s).x, 43);
    }
}

/// Exercises the PIF/pak emulation end to end; shown in the UI (works in the browser build).
pub fn self_test() -> Vec<(String, bool)> {
    let mut out = vec![];
    let mut check = |name: &str, ok: bool| out.push((name.to_string(), ok));
    let snap = Snapshot::default();
    let mut rt = Runtime::new(Config::default(), true);
    rt.config.controllers[0].pak = PakType::Tremor;
    rt.reopen_paks();

    let mut st = [1u8, 3, 0x00, 0, 0, 0];
    rt.pif_command(0, &mut st, &snap);
    check(
        "Status: controller + pak present",
        st[3..6] == [0x05, 0x00, 0x01],
    );

    let write = |rt: &mut Runtime, addr: u16, fill: u8| {
        let mut w = [0u8; 38];
        w[..3].copy_from_slice(&[35, 1, 0x03]);
        w[3..5].copy_from_slice(&addr.to_be_bytes());
        w[5..37].fill(fill);
        rt.pif_command(0, &mut w, &snap);
        w[37] == data_crc(&[fill; 32])
    };
    let read = |rt: &mut Runtime, addr: u16| {
        let mut r = [0u8; 38];
        r[..3].copy_from_slice(&[3, 33, 0x02]);
        r[3..5].copy_from_slice(&addr.to_be_bytes());
        rt.pif_command(0, &mut r, &snap);
        let mut d = [0u8; 32];
        d.copy_from_slice(&r[5..37]);
        (d, r[37] == data_crc(&d))
    };

    let (id, crc_ok) = read(&mut rt, 0x0020);
    let sum: u32 = (0..14)
        .map(|i| u16::from_be_bytes([id[i * 2], id[i * 2 + 1]]) as u32)
        .sum();
    let sum = (sum & 0xFFFF) as u16;
    check(
        "Memory: formatted ID block checksum",
        crc_ok && u16::from_be_bytes([id[28], id[29]]) == sum,
    );
    check("Memory: write CRC", write(&mut rt, 0x0400, 0x5A));
    check("Memory: read back", read(&mut rt, 0x0400).0 == [0x5A; 32]);
    write(&mut rt, 0x8000, 0xFE);
    write(&mut rt, 0x8000, 0x80);
    check(
        "Rumble: detect (0x8000 reads 0x80)",
        read(&mut rt, 0x8000).0 == [0x80; 32],
    );
    rt.rumble_events.clear();
    write(&mut rt, 0xC000, 0x01);
    check("Rumble: motor on", rt.rumble_events == [(0, true)]);
    write(&mut rt, 0xC000, 0x00);
    check(
        "Rumble: motor off",
        rt.rumble_events.last() == Some(&(0, false)),
    );

    let mut poll = [1u8, 4, 0x01, 0, 0, 0, 0];
    rt.pif_command(0, &mut poll, &snap);
    check("Poll: neutral controller", poll[3..7] == [0, 0, 0, 0]);
    let mut none = [1u8, 3, 0x00, 0, 0, 0];
    rt.pif_command(3, &mut none, &snap);
    check("Unplugged port reports no device", none[1] & 0x80 != 0);
    out
}

#[cfg(test)]
mod self_test_tests {
    #[test]
    fn self_test_passes() {
        for (name, ok) in super::self_test() {
            assert!(ok, "{name}");
        }
    }
}

//! Logs every gamepad event (mapped + raw codes) to stdout. Used to build controller presets.
//!
//! - `cargo run --release`            log mode: one line per event
//! - `cargo run --release -- --guided` prompts for each N64 control, then prints a YAML preset
//!   entry for `presets/controllers.yml`.

use gilrs::{Axis, Button, EventType, Gilrs, GilrsBuilder};
use std::io::Write;
use std::time::{Duration, Instant};

fn main() {
    let mut g = GilrsBuilder::new()
        .with_default_filters(false)
        .build()
        .expect("gilrs");
    if std::env::args().any(|a| a == "--guided") {
        guided(&mut g);
    } else {
        log(&mut g);
    }
}

fn log(g: &mut Gilrs) {
    let t0 = Instant::now();
    let mut out = std::io::stdout().lock();
    for (id, p) in g.gamepads() {
        writeln!(
            out,
            "PAD {id} name={:?} os_name={:?} vid={:04x?} pid={:04x?} mapping={:?} map_name={:?} ff={}",
            p.name(),
            p.os_name(),
            p.vendor_id(),
            p.product_id(),
            p.mapping_source(),
            p.map_name(),
            p.is_ff_supported()
        )
        .ok();
    }
    out.flush().ok();
    loop {
        let Some(ev) = g.next_event_blocking(None) else { continue };
        let t = t0.elapsed().as_secs_f32();
        let line = match ev.event {
            EventType::ButtonPressed(b, c) => format!("PRESS   {b:?} code={}", c.into_u32()),
            EventType::ButtonReleased(b, c) => format!("RELEASE {b:?} code={}", c.into_u32()),
            EventType::ButtonChanged(b, v, c) => format!("BTNVAL  {b:?} code={} v={v:.3}", c.into_u32()),
            EventType::AxisChanged(a, v, c) => {
                if v.abs() < 0.3 {
                    continue; // skip small stick noise
                }
                format!("AXIS    {a:?} code={} v={v:+.3}", c.into_u32())
            }
            EventType::Connected => {
                let p = g.gamepad(ev.id);
                format!(
                    "CONNECT name={:?} vid={:04x?} pid={:04x?} mapping={:?}",
                    p.name(),
                    p.vendor_id(),
                    p.product_id(),
                    p.mapping_source()
                )
            }
            EventType::Disconnected => "DISCONNECT".into(),
            other => format!("{other:?}"),
        };
        writeln!(out, "{t:8.3} pad{} {line}", ev.id).ok();
        out.flush().ok();
    }
}

/// Plugin `PadButton` name for a gilrs button (the plugin names shoulders/triggers differently).
fn pad_button(b: Button) -> Option<&'static str> {
    Some(match b {
        Button::South => "South",
        Button::East => "East",
        Button::North => "North",
        Button::West => "West",
        Button::LeftTrigger => "LeftShoulder",
        Button::LeftTrigger2 => "LeftTrigger",
        Button::RightTrigger => "RightShoulder",
        Button::RightTrigger2 => "RightTrigger",
        Button::Select => "Select",
        Button::Start => "Start",
        Button::Mode => "Mode",
        Button::LeftThumb => "LeftThumb",
        Button::RightThumb => "RightThumb",
        Button::DPadUp => "DPadUp",
        Button::DPadDown => "DPadDown",
        Button::DPadLeft => "DPadLeft",
        Button::DPadRight => "DPadRight",
        Button::C => "C",
        Button::Z => "Z",
        _ => return None,
    })
}

fn pad_axis(a: Axis) -> Option<&'static str> {
    Some(match a {
        Axis::LeftStickX => "LeftStickX",
        Axis::LeftStickY => "LeftStickY",
        Axis::RightStickX => "RightStickX",
        Axis::RightStickY => "RightStickY",
        Axis::LeftZ => "LeftZ",
        Axis::RightZ => "RightZ",
        Axis::DPadX => "DPadX",
        Axis::DPadY => "DPadY",
        _ => return None,
    })
}

/// Wait for one deliberate input; returns its preset YAML form. `None` = skipped (timeout).
fn capture(g: &mut Gilrs, timeout: Duration) -> Option<String> {
    let start = Instant::now();
    // Drain anything pending (releases, stick returning to centre).
    while g.next_event().is_some() {}
    while start.elapsed() < timeout {
        let Some(ev) = g.next_event_blocking(Some(Duration::from_millis(100))) else { continue };
        match ev.event {
            EventType::ButtonPressed(b, c) => {
                return Some(match pad_button(b) {
                    Some(n) => format!("{{type: button, button: {n}}}"),
                    None => format!("{{type: raw_button, code: {}}}", c.into_u32()),
                });
            }
            EventType::AxisChanged(a, v, c) if v.abs() > 0.8 => {
                let pos = v > 0.0;
                return Some(match pad_axis(a) {
                    Some(n) => format!("{{type: axis, axis: {n}, positive: {pos}}}"),
                    None => format!("{{type: raw_axis, code: {}, positive: {pos}}}", c.into_u32()),
                });
            }
            _ => {}
        }
    }
    None
}

fn guided(g: &mut Gilrs) {
    let controls: &[(&str, &str)] = &[
        ("A", "A"),
        ("B", "B"),
        ("Z", "Z"),
        ("Start", "Start"),
        ("L", "L"),
        ("R", "R"),
        ("DUp", "D-pad Up"),
        ("DDown", "D-pad Down"),
        ("DLeft", "D-pad Left"),
        ("DRight", "D-pad Right"),
        ("CUp", "C-Up"),
        ("CDown", "C-Down"),
        ("CLeft", "C-Left"),
        ("CRight", "C-Right"),
        ("up", "stick fully Up"),
        ("down", "stick fully Down"),
        ("left", "stick fully Left"),
        ("right", "stick fully Right"),
    ];
    eprintln!("Press any button on the controller to start…");
    let mut pad = None;
    while pad.is_none() {
        if let Some(ev) = g.next_event_blocking(None) {
            if matches!(ev.event, EventType::ButtonPressed(..)) {
                pad = Some(ev.id);
            }
        }
    }
    let p = g.gamepad(pad.unwrap());
    let (name, vid, pid) = (p.name().to_string(), p.vendor_id(), p.product_id());
    eprintln!("Using {name:?}. For each prompt press the control once (10 s to skip).\n");
    std::thread::sleep(Duration::from_millis(600));

    let mut got = vec![];
    for (key, label) in controls {
        eprint!("  {label:<18} … ");
        std::io::stderr().flush().ok();
        let input = capture(g, Duration::from_secs(10));
        eprintln!("{}", input.as_deref().unwrap_or("(skipped)"));
        got.push((*key, input));
        std::thread::sleep(Duration::from_millis(350)); // let the control return to rest
    }

    let raw = got.iter().any(|(_, i)| i.as_deref().is_some_and(|s| s.contains("raw_")));
    let os = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let mut y = String::new();
    y += &format!("  - name: {name:?}\n");
    y += &format!("    note: \"Captured with gamepad-listen --guided on {os}.\"\n");
    y += &format!("    names: [[{:?}]]\n", name.to_lowercase());
    if let (Some(v), Some(p)) = (vid, pid) {
        y += &format!("    ids: [\"{v:04x}:{p:04x}\"]\n");
    }
    if raw {
        y += &format!("    platforms: [{os}]\n");
    }
    y += "    buttons:\n";
    for (k, i) in got.iter().take(14) {
        if let Some(i) = i {
            y += &format!("      {:<7} [{i}]\n", format!("{k}:"));
        }
    }
    y += "    stick:\n";
    for (k, i) in got.iter().skip(14) {
        if let Some(i) = i {
            y += &format!("      {:<6} [{i}]\n", format!("{k}:"));
        }
    }
    eprintln!("\nPaste under `presets:` in presets/controllers.yml:\n");
    print!("{y}");
}

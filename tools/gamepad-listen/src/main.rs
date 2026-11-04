//! Logs every gamepad event (mapped + raw codes) to stdout. Used to build controller presets.
//! `cargo run --release` (from tools/gamepad-listen)

use gilrs::{EventType, GilrsBuilder};
use std::io::Write;
use std::time::Instant;

fn main() {
    let mut g = GilrsBuilder::new()
        .with_default_filters(false)
        .build()
        .expect("gilrs");
    let t0 = Instant::now();
    let out = std::io::stdout();
    let mut out = out.lock();
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
        if let Some(ev) = g.next_event_blocking(None) {
            let t = t0.elapsed().as_secs_f32();
            let line = match ev.event {
                EventType::ButtonPressed(b, c) => format!("PRESS   {b:?} code={}", c.into_u32()),
                EventType::ButtonReleased(b, c) => format!("RELEASE {b:?} code={}", c.into_u32()),
                EventType::ButtonChanged(b, v, c) => {
                    format!("BTNVAL  {b:?} code={} v={v:.3}", c.into_u32())
                }
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
}

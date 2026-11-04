//! Raw HID report listener for mapping controllers byte-by-byte, especially Switch-mode pads
//! (8BitDo 64 on S, Nintendo Switch Online N64) that OS gamepad layers don't map properly.
//!
//! ```text
//! hid-listen                         list HID devices
//! hid-listen 057e:2019               print input reports from that device, diffing each against the last
//! hid-listen 057e:2019 --full        first send the Switch "full report mode" handshake (0x30 reports)
//! hid-listen 057e:2019 --ignore 1,13-48   don't report changes in these byte indices
//! ```
//!
//! Output: a line per report whose (non-ignored) bytes changed, with changed bytes in [brackets],
//! followed by the individual bit flips, e.g. `byte 3 bit 1 0→1`.

use async_hid::{AsyncHidRead, AsyncHidWrite, Device, HidBackend, HidResult};
use futures_lite::StreamExt;
use std::collections::BTreeSet;
use std::io::Write;
use std::time::Instant;

fn parse_ids(s: &str) -> Option<(u16, u16)> {
    let (v, p) = s.split_once(':')?;
    Some((u16::from_str_radix(v, 16).ok()?, u16::from_str_radix(p, 16).ok()?))
}

fn parse_ignore(s: &str) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for part in s.split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    out.extend(a..=b);
                }
            }
            None => {
                if let Ok(a) = part.parse() {
                    out.insert(a);
                }
            }
        }
    }
    out
}

fn main() -> HidResult<()> {
    futures_lite::future::block_on(run())
}

async fn run() -> HidResult<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ids = args.iter().find_map(|a| parse_ids(a));
    let full = args.iter().any(|a| a == "--full");
    let mut ignore = args
        .iter()
        .position(|a| a == "--ignore")
        .and_then(|i| args.get(i + 1))
        .map(|s| parse_ignore(s))
        .unwrap_or_default();

    let backend = HidBackend::default();
    let devices: Vec<Device> = backend.enumerate().await?.collect().await;

    let Some((vid, pid)) = ids else {
        println!("HID devices (pass vvvv:pppp to listen):");
        for d in &devices {
            println!(
                "  {:04x}:{:04x}  usage {:04x}:{:04x}  {}",
                d.vendor_id, d.product_id, d.usage_page, d.usage_id, d.name
            );
        }
        return Ok(());
    };

    let Some(info) = devices
        .into_iter()
        .find(|d| d.vendor_id == vid && d.product_id == pid)
    else {
        eprintln!("No HID device {vid:04x}:{pid:04x}. Run without arguments to list devices.");
        std::process::exit(1);
    };
    println!(
        "OPEN {:04x}:{:04x} usage {:04x}:{:04x} {:?}",
        info.vendor_id, info.product_id, info.usage_page, info.usage_id, info.name
    );
    let mut dev = info.open().await?;

    if full {
        // Nintendo Switch subcommand 0x03 (set input report mode) = 0x30 (standard full mode).
        // Output report 0x01: [id, packet counter, 8 bytes neutral rumble, subcommand, arg].
        let rumble = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
        let mut pkt = vec![0x01, 0x00];
        pkt.extend_from_slice(&rumble);
        pkt.extend_from_slice(&[0x03, 0x30]);
        match dev.write_output_report(&pkt).await {
            Ok(()) => println!("SENT full-mode handshake (subcommand 0x03 0x30)"),
            Err(e) => println!("handshake failed: {e:?}"),
        }
        // Full-mode reports carry a timer (byte 1) and IMU data (bytes 13..): ignore by default.
        if ignore.is_empty() {
            ignore.insert(1);
            ignore.extend(13..64);
            println!("IGNORING bytes 1 and 13-63 (timer + IMU); override with --ignore");
        }
    }

    let t0 = Instant::now();
    let mut last: Vec<u8> = vec![];
    let mut buf = [0u8; 128];
    let mut out = std::io::stdout().lock();
    loop {
        let n = dev.read_input_report(&mut buf).await?;
        let r = &buf[..n];
        let changed: Vec<usize> = (0..n)
            .filter(|i| !ignore.contains(i) && last.get(*i) != Some(&r[*i]))
            .collect();
        if changed.is_empty() {
            continue;
        }
        let hex: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if changed.contains(&i) && !last.is_empty() {
                    format!("[{b:02x}]")
                } else {
                    format!(" {b:02x} ")
                }
            })
            .collect();
        writeln!(out, "{:8.3} len={n:<3} {}", t0.elapsed().as_secs_f32(), hex.join("")).ok();
        if !last.is_empty() {
            for &i in &changed {
                let (old, new) = (last.get(i).copied().unwrap_or(0), r[i]);
                let flips: Vec<String> = (0..8)
                    .filter(|b| (old ^ new) & (1 << b) != 0)
                    .map(|b| format!("bit {b} {}→{}", (old >> b) & 1, (new >> b) & 1))
                    .collect();
                writeln!(out, "         byte {i}: {old:02x}→{new:02x}  {}", flips.join(", ")).ok();
            }
        }
        out.flush().ok();
        last = r.to_vec();
    }
}

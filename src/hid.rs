//! Native raw HID input (desktop). Reads input reports from selected devices (by vendor:product)
//! so controllers whose OS gamepad mapping is broken (e.g. Switch-mode pads) can be bound
//! byte-by-byte. The browser build uses WebHID instead (see `index.html`).

use async_hid::{AsyncHidRead, AsyncHidWrite, HidBackend};
use futures_lite::{FutureExt, StreamExt};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Latest input reports of one open HID device.
#[derive(Clone, Debug, Default)]
pub struct HidDevice {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    /// report id → payload (without the id byte, same as WebHID)
    pub reports: BTreeMap<u8, Vec<u8>>,
    /// Whether this device is driven by [`switch_pro_rumble`] and can actually rumble.
    pub ff: bool,
}

/// Consecutive read errors to tolerate (as transient Bluetooth hiccups) before concluding a
/// device actually disconnected.
const MAX_CONSECUTIVE_READ_ERRORS: u32 = 15;

type Reports = Arc<Mutex<BTreeMap<String, HidDevice>>>;
type RumbleRequests = Arc<Mutex<BTreeMap<String, f32>>>;

pub struct HidReader {
    shared: Reports,
    rumble: RumbleRequests,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HidReader {
    /// Watch for and read devices matching any of `ids` (vendor, product).
    pub fn new(ids: Vec<(u16, u16)>) -> Self {
        let shared: Reports = Default::default();
        let rumble: RumbleRequests = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, rumble, stop) = (shared.clone(), rumble.clone(), stop.clone());
            std::thread::Builder::new()
                .name("tdug-n64-hid".into())
                .spawn(move || manager(ids, shared, rumble, stop))
                .ok()
        };
        HidReader {
            shared,
            rumble,
            stop,
            thread,
        }
    }

    pub fn devices(&self) -> Vec<HidDevice> {
        self.shared
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Request rumble strength (0..1) for the device at `devices()[index]`. A device that isn't
    /// `ff` ignores this (there is nowhere to send it).
    pub fn set_rumble(&self, index: usize, strength: f32) {
        let Some(key) = self
            .shared
            .lock()
            .ok()
            .and_then(|m| m.keys().nth(index).cloned())
        else {
            return;
        };
        if let Ok(mut r) = self.rumble.lock() {
            if strength > 0.0 {
                r.insert(key, strength.clamp(0.0, 1.0));
            } else {
                r.remove(&key);
            }
        }
    }

    pub fn stop_all_rumble(&self) {
        if let Ok(mut r) = self.rumble.lock() {
            r.clear();
        }
    }
}

impl Drop for HidReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Re-enumerates every 2 s and starts a reader thread per newly seen matching device.
fn manager(ids: Vec<(u16, u16)>, shared: Reports, rumble: RumbleRequests, stop: Arc<AtomicBool>) {
    let mut readers: Vec<(String, JoinHandle<()>)> = vec![];
    let backend = HidBackend::default();
    while !stop.load(Ordering::Relaxed) {
        readers.retain(|(_, h)| !h.is_finished());
        let found: Vec<async_hid::Device> = futures_lite::future::block_on(async {
            match backend.enumerate().await {
                Ok(s) => s.collect().await,
                Err(_) => vec![],
            }
        });
        for d in found {
            if !ids.contains(&(d.vendor_id, d.product_id)) {
                continue;
            }
            let key = format!("{:04x}:{:04x}:{}", d.vendor_id, d.product_id, d.name);
            if readers.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let (shared, rumble, stop, k) =
                (shared.clone(), rumble.clone(), stop.clone(), key.clone());
            let handle = if switch_pro_rumble::is_switch_pro(d.vendor_id, d.product_id) {
                std::thread::Builder::new()
                    .name("tdug-n64-hid-rumble".into())
                    .spawn(move || switch_pro_rumble::run(d, k, shared, rumble, stop))
            } else {
                std::thread::Builder::new()
                    .name("tdug-n64-hid-read".into())
                    .spawn(move || read_device(d, k, shared, stop))
            };
            if let Ok(h) = handle {
                readers.push((key, h));
            }
        }
        for _ in 0..20 {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    for (_, h) in readers {
        let _ = h.join();
    }
}

fn read_device(d: async_hid::Device, key: String, shared: Reports, stop: Arc<AtomicBool>) {
    futures_lite::future::block_on(async {
        let Ok(mut reader) = d.open_readable().await else {
            crate::platform::log(&format!("HID: can't open {key}"));
            return;
        };
        crate::platform::log(&format!("HID: reading {key}"));
        if let Ok(mut m) = shared.lock() {
            m.insert(
                key.clone(),
                HidDevice {
                    name: d.name.clone(),
                    vid: d.vendor_id,
                    pid: d.product_id,
                    reports: BTreeMap::new(),
                    ff: false,
                },
            );
        }
        let mut buf = [0u8; 256];
        // Bluetooth HID reads occasionally time out or glitch transiently; only treat it as a
        // real disconnect after several in a row, not the first one (that caused frequent
        // close/reopen churn — see tdug/n64_controller_plugin#3 discussion).
        let mut consecutive_errors = 0u32;
        while !stop.load(Ordering::Relaxed) {
            // Time out regularly so shutdown never waits on an idle device.
            let r = async { Some(reader.read_input_report(&mut buf).await) }
                .or(async {
                    async_io::Timer::after(Duration::from_millis(200)).await;
                    None
                })
                .await;
            match r {
                None => continue,
                Some(Ok(n)) if n > 0 => {
                    consecutive_errors = 0;
                    // Desktop reads start with the report id byte.
                    let (id, payload) = (buf[0], buf[1..n].to_vec());
                    if let Ok(mut m) = shared.lock() {
                        if let Some(dev) = m.get_mut(&key) {
                            dev.reports.insert(id, payload);
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_READ_ERRORS {
                        break; // actually disconnected
                    }
                }
            }
        }
        if let Ok(mut m) = shared.lock() {
            m.remove(&key);
        }
        crate::platform::log(&format!("HID: closed {key}"));
    });
}

/// Rumble for pads that identify as a genuine Nintendo Switch Pro Controller over Bluetooth
/// (057e:2019) — e.g. the 8BitDo 64 in S mode. Confirmed working live against real hardware;
/// see tdug/n64_controller_plugin#3.
///
/// Uses the public Switch controller protocol (dekuNukem/Nintendo_Switch_Reverse_Engineering):
/// switch to full standard input mode, enable vibration, then send HD-rumble output reports.
/// Rather than reproduce Nintendo's full nonlinear amplitude-encoding table (unverified for this
/// clone hardware), strength is approximated by duty-cycling a single known-working "on" rumble
/// value against a neutral one — confirmed perceptible but coarse. The neutral value is the one
/// SDL had to use specifically for the 8BitDo 64 (not Nintendo's documented default): see
/// https://discourse.libsdl.org/t/sdl-fixed-initial-rumble-on-the-8bitdo-64-bluetooth-controller/64658
mod switch_pro_rumble {
    use super::*;

    const VID: u16 = 0x057e;
    const PID: u16 = 0x2019;

    /// SDL's 8BitDo-64-specific neutral value, not Nintendo's documented `[00,01,40,40]`.
    const NEUTRAL_MOTOR: [u8; 4] = [0x00, 0x00, 0x01, 0x40];
    /// Confirmed live to produce real vibration (see tdug/n64_controller_plugin#3 discussion).
    const STRONG_MOTOR: [u8; 4] = [0x04, 0x92, 0x48, 0x40];
    /// Duty-cycle period for approximating strength: 4 ticks of `TICK`.
    const DUTY_PERIOD: u32 = 4;
    const TICK: Duration = Duration::from_millis(40);

    pub fn is_switch_pro(vid: u16, pid: u16) -> bool {
        (vid, pid) == (VID, PID)
    }

    fn rumble_bytes(motor: [u8; 4]) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&motor);
        b[4..].copy_from_slice(&motor);
        b
    }

    /// Report 0x01: rumble + subcommand.
    fn subcommand_report(counter: u8, subcommand: u8, data: &[u8]) -> Vec<u8> {
        let mut v = vec![0x01u8, counter];
        v.extend_from_slice(&rumble_bytes(NEUTRAL_MOTOR));
        v.push(subcommand);
        v.extend_from_slice(data);
        v
    }

    /// Report 0x10: rumble only (fast path once in standard input mode).
    fn rumble_report(counter: u8, motor: [u8; 4]) -> Vec<u8> {
        let mut v = vec![0x10u8, counter];
        v.extend_from_slice(&rumble_bytes(motor));
        v
    }

    pub fn run(
        d: async_hid::Device,
        key: String,
        shared: Reports,
        rumble: RumbleRequests,
        stop: Arc<AtomicBool>,
    ) {
        futures_lite::future::block_on(async {
            let Ok((mut reader, mut writer)) = d.open().await else {
                crate::platform::log(&format!("HID: can't open {key}"));
                return;
            };
            crate::platform::log(&format!("HID: reading+rumble {key}"));
            if let Ok(mut m) = shared.lock() {
                m.insert(
                    key.clone(),
                    HidDevice {
                        name: d.name.clone(),
                        vid: d.vendor_id,
                        pid: d.product_id,
                        reports: BTreeMap::new(),
                        ff: true,
                    },
                );
            }

            let mut counter: u8 = 0;
            let mut next_counter = || {
                let v = counter;
                counter = (counter + 1) & 0x0F;
                v
            };
            // Enable vibration only — deliberately not switching input report mode (0x03/0x30):
            // that changes the report's byte layout entirely, breaking every binding captured
            // against the device's default report shape. Rumble (0x10) works independent of
            // which input mode is active.
            let _ = writer
                .write_output_report(&subcommand_report(next_counter(), 0x48, &[0x01]))
                .await;

            let mut buf = [0u8; 256];
            let mut phase: u32 = 0;
            let mut last_rumble = Instant::now();
            let mut consecutive_errors = 0u32;
            while !stop.load(Ordering::Relaxed) {
                // Poll reads frequently so input stays responsive; rumble sends run on their own
                // clock below, independent of how often reads happen to win this race.
                let r = async { Some(reader.read_input_report(&mut buf).await) }
                    .or(async {
                        async_io::Timer::after(Duration::from_millis(5)).await;
                        None
                    })
                    .await;
                match r {
                    Some(Ok(n)) if n > 0 => {
                        consecutive_errors = 0;
                        let (id, payload) = (buf[0], buf[1..n].to_vec());
                        if let Ok(mut m) = shared.lock() {
                            if let Some(dev) = m.get_mut(&key) {
                                dev.reports.insert(id, payload);
                            }
                        }
                    }
                    Some(Err(_)) => {
                        consecutive_errors += 1;
                        if consecutive_errors >= MAX_CONSECUTIVE_READ_ERRORS {
                            break; // actually disconnected
                        }
                    }
                    _ => {}
                }
                if last_rumble.elapsed() < TICK {
                    continue;
                }
                last_rumble = Instant::now();
                let strength = rumble.lock().ok().and_then(|r| r.get(&key).copied());
                let on_ticks = strength.map_or(0, |s| (s * DUTY_PERIOD as f32).round() as u32);
                let on = (phase % DUTY_PERIOD) < on_ticks;
                phase = phase.wrapping_add(1);
                let motor = if on { STRONG_MOTOR } else { NEUTRAL_MOTOR };
                if let Err(e) = writer
                    .write_output_report(&rumble_report(next_counter(), motor))
                    .await
                {
                    crate::platform::log(&format!("HID: rumble write failed on {key}: {e:?}"));
                }
            }
            let _ = writer
                .write_output_report(&rumble_report(next_counter(), NEUTRAL_MOTOR))
                .await;
            if let Ok(mut m) = shared.lock() {
                m.remove(&key);
            }
            crate::platform::log(&format!("HID: closed {key}"));
        });
    }
}

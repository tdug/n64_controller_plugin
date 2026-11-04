//! Native raw HID input (desktop). Reads input reports from selected devices (by vendor:product)
//! so controllers whose OS gamepad mapping is broken (e.g. Switch-mode pads) can be bound
//! byte-by-byte. The browser build uses WebHID instead (see `index.html`).

use async_hid::{AsyncHidRead, HidBackend};
use futures_lite::{FutureExt, StreamExt};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Latest input reports of one open HID device.
#[derive(Clone, Debug, Default)]
pub struct HidDevice {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    /// report id → payload (without the id byte, same as WebHID)
    pub reports: BTreeMap<u8, Vec<u8>>,
}

type Shared = Arc<Mutex<BTreeMap<String, HidDevice>>>;

pub struct HidReader {
    shared: Shared,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HidReader {
    /// Watch for and read devices matching any of `ids` (vendor, product).
    pub fn new(ids: Vec<(u16, u16)>) -> Self {
        let shared: Shared = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (shared.clone(), stop.clone());
            std::thread::Builder::new()
                .name("tdug-n64-hid".into())
                .spawn(move || manager(ids, shared, stop))
                .ok()
        };
        HidReader {
            shared,
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
fn manager(ids: Vec<(u16, u16)>, shared: Shared, stop: Arc<AtomicBool>) {
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
            let (shared, stop, k) = (shared.clone(), stop.clone(), key.clone());
            if let Ok(h) = std::thread::Builder::new()
                .name("tdug-n64-hid-read".into())
                .spawn(move || read_device(d, k, shared, stop))
            {
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

fn read_device(d: async_hid::Device, key: String, shared: Shared, stop: Arc<AtomicBool>) {
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
                },
            );
        }
        let mut buf = [0u8; 256];
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
                    // Desktop reads start with the report id byte.
                    let (id, payload) = (buf[0], buf[1..n].to_vec());
                    if let Ok(mut m) = shared.lock() {
                        if let Some(dev) = m.get_mut(&key) {
                            dev.reports.insert(id, payload);
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => break, // disconnected
            }
        }
        if let Ok(mut m) = shared.lock() {
            m.remove(&key);
        }
        crate::platform::log(&format!("HID: closed {key}"));
    });
}

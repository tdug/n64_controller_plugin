//! C ABI exports.
//!
//! One library serves two plugin APIs:
//! * Zilmar controller spec 1.1 (Project64, 1964, Nemu64, legacy Mupen64...)
//! * Mupen64Plus input plugin API 2.x (mupen64plus-ui-console, RMG, ...)
//!
//! Both use the same names for the core entry points (`InitiateControllers`, `GetKeys`,
//! `ReadController`, ...). Mupen64Plus always calls `PluginStartup` first, which is how we
//! tell them apart.
#![allow(non_snake_case, clippy::not_unsafe_ptr_arg_deref)]

use crate::config::{Config, PORTS, PakType, storage};
use crate::input::{InputManager, Snapshot, clear_event_keys, sdl_scancode_to_vk, set_event_key};
use crate::platform::{keyboard_allowed, log};
use crate::runtime::Runtime;
use std::ffi::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const PLUGIN_NAME: &str = concat!("TDug N64 Input ", env!("CARGO_PKG_VERSION"));

const PLUGIN_MEMPAK: i32 = 2;
const PLUGIN_RAW: i32 = 5;

// ---------------------------------------------------------------------------
// Shared state

struct Controls {
    ptr: usize,
    /// CONTROL struct size in i32 units (3 for Zilmar / old Mupen64Plus, 4 with `Type`).
    stride: usize,
    raw: [bool; PORTS],
}

struct Shared {
    rt: Mutex<Runtime>,
    snap: Mutex<Snapshot>,
    controls: Mutex<Controls>,
}

struct Running {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static M64P: AtomicBool = AtomicBool::new(false);
static CORE_HANDLE: AtomicUsize = AtomicUsize::new(0);
static CORE_VERSION: AtomicI32 = AtomicI32::new(0);
static ZILMAR_ROM_NAME: Mutex<String> = Mutex::new(String::new());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn shared() -> Arc<Shared> {
    let mut g = lock(&RUNNING);
    if let Some(r) = g.as_ref() {
        return r.shared.clone();
    }
    log(&format!(
        "{PLUGIN_NAME} starting (m64p={})",
        M64P.load(Ordering::Relaxed)
    ));
    let shared = Arc::new(Shared {
        rt: Mutex::new(Runtime::new(Config::load(), false)),
        snap: Mutex::new(Snapshot::default()),
        controls: Mutex::new(Controls {
            ptr: 0,
            stride: 3,
            raw: [false; PORTS],
        }),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let (s, st) = (shared.clone(), stop.clone());
        std::thread::Builder::new()
            .name("tdug-n64-input".into())
            .spawn(move || input_thread(s, st))
            .ok()
    };
    *g = Some(Running {
        shared: shared.clone(),
        stop,
        thread,
    });
    shared
}

fn shutdown() {
    let r = lock(&RUNNING).take();
    if let Some(mut r) = r {
        r.stop.store(true, Ordering::Relaxed);
        if let Some(t) = r.thread.take() {
            let _ = t.join();
        }
        lock(&r.shared.rt).rom_closed();
        log("stopped");
    }
}

fn apply_controls(sh: &Shared) {
    let rt = lock(&sh.rt);
    let mut c = lock(&sh.controls);
    if c.ptr == 0 {
        return;
    }
    for i in 0..PORTS {
        let cc = &rt.config.controllers[i];
        let (raw, plugin) = if rt.active_pak(i) == PakType::EmulatorMemory {
            (false, PLUGIN_MEMPAK)
        } else {
            (true, PLUGIN_RAW)
        };
        c.raw[i] = raw;
        // SAFETY: the emulator gave us an array of 4 CONTROL structs that outlives the plugin.
        unsafe {
            let p = (c.ptr as *mut i32).add(i * c.stride);
            p.write(cc.plugged as i32);
            p.add(1).write(raw as i32);
            p.add(2).write(plugin);
            if c.stride >= 4 {
                p.add(3).write(0); // CONT_TYPE_STANDARD
            }
        }
    }
}

fn reload_config(sh: &Shared) {
    let cfg = Config::load();
    lock(&sh.rt).set_config(cfg);
    apply_controls(sh);
    log("config reloaded");
}

fn input_thread(sh: Arc<Shared>, stop: Arc<AtomicBool>) {
    #[cfg(windows)]
    // SAFETY: plain Win32 call; paired with timeEndPeriod below.
    unsafe {
        windows_sys::Win32::Media::timeBeginPeriod(1);
    }
    let mut mgr = InputManager::new();
    mgr.set_hid_devices(&lock(&sh.rt).config.hid_devices);
    let mut snap = Snapshot::default();
    let mut rumbling: [Option<usize>; PORTS] = [None; PORTS];
    let mut mtime = storage::mtime();
    let mut last_check = Instant::now();
    let mut last_flush = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let (bg, events, targets) = {
            let mut rt = lock(&sh.rt);
            let events = std::mem::take(&mut rt.rumble_events);
            let targets: Vec<_> = if events.is_empty() {
                vec![]
            } else {
                rt.config
                    .controllers
                    .iter()
                    .map(|c| (c.device.clone(), c.rumble_strength))
                    .collect()
            };
            (rt.config.background_input, events, targets)
        };
        mgr.poll(&mut snap, keyboard_allowed(bg));
        for (port, on) in events {
            if let Some(dev) = rumbling[port].take() {
                mgr.set_rumble(dev, 0.0);
            }
            if on {
                let (sel, strength) = &targets[port];
                if let Some(dev) = snap.resolve(sel, port) {
                    mgr.set_rumble(dev.id, *strength);
                    rumbling[port] = Some(dev.id);
                }
            }
        }
        *lock(&sh.snap) = snap.clone();

        if last_check.elapsed() > Duration::from_millis(500) {
            last_check = Instant::now();
            let m = storage::mtime();
            if m != mtime {
                mtime = m;
                reload_config(&sh);
                mgr.set_hid_devices(&lock(&sh.rt).config.hid_devices);
            }
        }
        if last_flush.elapsed() > Duration::from_secs(1) {
            last_flush = Instant::now();
            lock(&sh.rt).flush_paks();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    mgr.stop_all_rumble();
    #[cfg(windows)]
    // SAFETY: matches timeBeginPeriod above.
    unsafe {
        windows_sys::Win32::Media::timeEndPeriod(1);
    }
}

fn guard<R: Default>(f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log("panic caught at FFI boundary");
        R::default()
    })
}

fn rom_name_from_header(h: &[u8], dword_swapped: bool) -> String {
    let get = |i: usize| if dword_swapped { h[i ^ 3] } else { h[i] };
    let raw: String = (0x20..0x34).map(|i| get(i) as char).collect();
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                ' '
            }
        })
        .collect();
    clean.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Zilmar spec

#[repr(C)]
pub struct PluginInfo {
    version: u16,
    kind: u16,
    name: [c_char; 100],
    normal_memory: c_int,
    memory_bswaped: c_int,
}

#[repr(C)]
pub struct ControlInfo {
    hwnd: *mut c_void,
    hinst: *mut c_void,
    memory_bswaped: c_int,
    header: *const u8,
    controls: *mut c_int,
}

#[unsafe(no_mangle)]
pub extern "C" fn GetDllInfo(info: *mut PluginInfo) {
    if info.is_null() {
        return;
    }
    // SAFETY: emulator passes a valid PLUGIN_INFO.
    let info = unsafe { &mut *info };
    info.version = 0x0101;
    info.kind = 4; // PLUGIN_TYPE_CONTROLLER
    info.name = [0; 100];
    for (d, s) in info.name.iter_mut().zip(PLUGIN_NAME.bytes().take(99)) {
        *d = s as c_char;
    }
    info.normal_memory = 0;
    info.memory_bswaped = 1;
}

/// Zilmar 1.1: `ControlInfo*`. Mupen64Plus: `CONTROL_INFO` by value (= a `CONTROL*`).
#[unsafe(no_mangle)]
pub extern "C" fn InitiateControllers(arg: *mut c_void) {
    guard(|| {
        if arg.is_null() {
            return;
        }
        let sh = shared();
        let (ptr, stride) = if M64P.load(Ordering::Relaxed) {
            let v = CORE_VERSION.load(Ordering::Relaxed);
            (arg as usize, if v >= 0x020600 { 4 } else { 3 })
        } else {
            // SAFETY: Zilmar 1.1 passes a valid CONTROL_INFO pointer.
            let info = unsafe { &*(arg as *const ControlInfo) };
            if !info.header.is_null() {
                // SAFETY: header points at the first 0x40 bytes of the ROM.
                let h = unsafe { std::slice::from_raw_parts(info.header, 0x40) };
                *lock(&ZILMAR_ROM_NAME) = rom_name_from_header(h, info.memory_bswaped != 0);
            }
            (info.controls as usize, 3)
        };
        {
            let mut c = lock(&sh.controls);
            c.ptr = ptr;
            c.stride = stride;
        }
        apply_controls(&sh);
        log(&format!("InitiateControllers stride={stride}"));
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn GetKeys(port: c_int, keys: *mut u32) {
    guard(|| {
        if !(0..PORTS as c_int).contains(&port) || keys.is_null() {
            return;
        }
        let port = port as usize;
        let sh = shared();
        let raw = lock(&sh.controls).raw[port];
        let mut rt = lock(&sh.rt);
        // The emulator calls GetKeys once per VI: use it as the frame clock.
        rt.vi_tick();
        let st = if raw {
            rt.last_state(port)
        } else {
            let snap = lock(&sh.snap);
            rt.poll(port, &snap)
        };
        // SAFETY: valid BUTTONS pointer from the emulator.
        unsafe { keys.write_unaligned(st.keys_value()) };
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ControllerCommand(_port: c_int, _cmd: *mut u8) {
    // Raw commands are handled in ReadController, which sees the same buffer.
}

#[unsafe(no_mangle)]
pub extern "C" fn ReadController(port: c_int, cmd: *mut u8) {
    guard(|| {
        if port == -1 {
            // End of a PIF transaction.
            if let Some(r) = lock(&RUNNING).as_ref() {
                lock(&r.shared.rt).pif_cycle_end();
            }
            return;
        }
        if !(0..PORTS as c_int).contains(&port) || cmd.is_null() {
            return;
        }
        // SAFETY: cmd points at a PIF channel block: [tx][rx][tx bytes][rx bytes].
        let block = unsafe {
            let tx = *cmd as usize;
            let rx = (*cmd.add(1) & 0x3F) as usize;
            std::slice::from_raw_parts_mut(cmd, 2 + tx + rx)
        };
        let sh = shared();
        let mut rt = lock(&sh.rt);
        let snap = lock(&sh.snap);
        rt.pif_command(port as usize, block, &snap);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn RomOpen() -> c_int {
    guard(|| {
        let name = if M64P.load(Ordering::Relaxed) {
            m64p_rom_name()
        } else {
            lock(&ZILMAR_ROM_NAME).clone()
        };
        log(&format!("RomOpen '{name}'"));
        let sh = shared();
        lock(&sh.rt).rom_open(name);
        apply_controls(&sh);
        1
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn RomClosed() {
    guard(|| {
        clear_event_keys();
        if let Some(r) = lock(&RUNNING).as_ref() {
            lock(&r.shared.rt).rom_closed();
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn CloseDLL() {
    guard(shutdown)
}

#[unsafe(no_mangle)]
pub extern "C" fn WM_KeyDown(wparam: usize, _lparam: isize) {
    set_event_key(wparam as u8, true);
}

#[unsafe(no_mangle)]
pub extern "C" fn WM_KeyUp(wparam: usize, _lparam: isize) {
    set_event_key(wparam as u8, false);
}

/// Only used when the emulator handles the rumble pak itself; we always use raw mode.
#[unsafe(no_mangle)]
pub extern "C" fn RumbleCommand(_port: c_int, _on: c_int) {}

#[unsafe(no_mangle)]
pub extern "C" fn DllTest(_hwnd: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn DllAbout(hwnd: *mut c_void) {
    #[cfg(windows)]
    {
        let text = format!(
            "{PLUGIN_NAME}\n\nN64 controller plugin with rumble, Tremor Pak, \
             analog calibration and macros.\nhttps://github.com/tdug/n64_controller_plugin\0"
        );
        let wide: Vec<u16> = text.encode_utf16().collect();
        let title: Vec<u16> = "About\0".encode_utf16().collect();
        // SAFETY: null-terminated UTF-16 strings.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                hwnd as _,
                wide.as_ptr(),
                title.as_ptr(),
                0,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = hwnd;
}

#[unsafe(no_mangle)]
pub extern "C" fn DllConfig(hwnd: *mut c_void) {
    guard(|| {
        open_config_ui(hwnd);
        if let Some(sh) = lock(&RUNNING).as_ref().map(|r| r.shared.clone()) {
            reload_config(&sh);
        }
    })
}

/// Runs the configuration UI in a separate process so its window/event loop can't
/// interfere with the emulator (and it can be opened any number of times).
#[cfg(windows)]
fn open_config_ui(parent: *mut c_void) {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, DispatchMessageW, MSG, MsgWaitForMultipleObjects,
        PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage,
    };
    let Some(dll) = crate::platform::module_path() else {
        return;
    };
    let mut sys = vec![0u16; 260];
    // SAFETY: buffer is large enough for the system directory.
    let n = unsafe { GetSystemDirectoryW(sys.as_mut_ptr(), sys.len() as u32) } as usize;
    let sys = String::from_utf16_lossy(&sys[..n]);
    let cmdline = format!("\"{sys}\\rundll32.exe\" \"{}\",ConfigMain\0", dll.display());
    let mut wide: Vec<u16> = cmdline.encode_utf16().collect();
    // SAFETY: standard CreateProcess/wait pattern with valid, zeroed structs.
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        AllowSetForegroundWindow(ASFW_ANY);
        if CreateProcessW(
            std::ptr::null(),
            wide.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        ) == 0
        {
            log("failed to launch config UI");
            return;
        }
        let parent = parent as windows_sys::Win32::Foundation::HWND;
        if !parent.is_null() {
            EnableWindow(parent, 0);
        }
        // Keep the emulator's window responsive while we wait (like a modal dialog).
        loop {
            let r = MsgWaitForMultipleObjects(1, &pi.hProcess, 0, INFINITE, QS_ALLINPUT);
            if r == WAIT_OBJECT_0 {
                break;
            }
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !parent.is_null() {
            EnableWindow(parent, 1);
            windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(parent);
        }
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
}

#[cfg(not(windows))]
fn open_config_ui(_parent: *mut c_void) {
    // Look for the standalone config tool next to the plugin.
    if let Some(dir) = crate::platform::module_path().and_then(|p| p.parent().map(|d| d.to_owned()))
    {
        let exe = dir.join("tdug-n64-config");
        if let Ok(mut child) = std::process::Command::new(exe).spawn() {
            let _ = child.wait();
        }
    }
}

/// `rundll32 tdug_plugin.dll,ConfigMain` entry point. Never returns.
#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "C" fn ConfigMain(
    _hwnd: *mut c_void,
    _hinst: *mut c_void,
    _cmd: *const c_char,
    _show: c_int,
) {
    // rundll32 calls with stdcall; we exit before returning so the convention mismatch is moot.
    crate::gui::run_native();
    std::process::exit(0);
}

// ---------------------------------------------------------------------------
// Mupen64Plus

type CoreDoCommand = unsafe extern "C" fn(c_int, c_int, *mut c_void) -> c_int;
type GetVersionFn = unsafe extern "C" fn(
    *mut c_int,
    *mut c_int,
    *mut c_int,
    *mut *const c_char,
    *mut c_int,
) -> c_int;

#[cfg(windows)]
unsafe fn core_sym(name: &str) -> *mut c_void {
    let h = CORE_HANDLE.load(Ordering::Relaxed);
    if h == 0 {
        return std::ptr::null_mut();
    }
    let cname = format!("{name}\0");
    // SAFETY: valid module handle from the core, null-terminated name.
    unsafe {
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(h as _, cname.as_ptr())
            .map_or(std::ptr::null_mut(), |f| f as *mut c_void)
    }
}

#[cfg(not(windows))]
unsafe fn core_sym(name: &str) -> *mut c_void {
    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    let h = CORE_HANDLE.load(Ordering::Relaxed);
    if h == 0 {
        return std::ptr::null_mut();
    }
    let cname = format!("{name}\0");
    // SAFETY: valid dlopen handle from the core, null-terminated name.
    unsafe { dlsym(h as *mut c_void, cname.as_ptr() as *const c_char) }
}

fn m64p_rom_name() -> String {
    // SAFETY: CoreDoCommand(M64CMD_ROM_GET_HEADER, sizeof(m64p_rom_header)=64, buf).
    unsafe {
        let f = core_sym("CoreDoCommand");
        if f.is_null() {
            return String::new();
        }
        let f: CoreDoCommand = std::mem::transmute(f);
        let mut buf = [0u8; 64];
        if f(3, 64, buf.as_mut_ptr() as *mut c_void) != 0 {
            return String::new();
        }
        rom_name_from_header(&buf, false)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PluginStartup(
    core: *mut c_void,
    _ctx: *mut c_void,
    _debug: *mut c_void,
) -> c_int {
    guard(|| {
        M64P.store(true, Ordering::Relaxed);
        CORE_HANDLE.store(core as usize, Ordering::Relaxed);
        // SAFETY: core's PluginGetVersion has the standard signature; nulls are allowed.
        unsafe {
            let f = core_sym("PluginGetVersion");
            if !f.is_null() {
                let f: GetVersionFn = std::mem::transmute(f);
                let mut v = 0;
                f(
                    std::ptr::null_mut(),
                    &mut v,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                CORE_VERSION.store(v, Ordering::Relaxed);
                log(&format!("mupen64plus core version {v:#x}"));
            }
        }
        shared();
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PluginShutdown() -> c_int {
    guard(|| {
        shutdown();
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PluginGetVersion(
    kind: *mut c_int,
    version: *mut c_int,
    api: *mut c_int,
    name: *mut *const c_char,
    caps: *mut c_int,
) -> c_int {
    static NAME: &str = concat!("TDug N64 Input ", env!("CARGO_PKG_VERSION"), "\0");
    // SAFETY: each pointer is optional and, if non-null, valid for writes.
    unsafe {
        if !kind.is_null() {
            *kind = 4; // M64PLUGIN_INPUT
        }
        if !version.is_null() {
            let v: Vec<i32> = env!("CARGO_PKG_VERSION")
                .split('.')
                .map(|s| s.parse().unwrap_or(0))
                .collect();
            *version = (v[0] << 16) | (v[1] << 8) | v[2];
        }
        if !api.is_null() {
            *api = 0x020100;
        }
        if !name.is_null() {
            *name = NAME.as_ptr() as *const c_char;
        }
        if !caps.is_null() {
            *caps = 0;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn SDL_KeyDown(_keymod: c_int, keysym: c_int) {
    if let Some(vk) = sdl_scancode_to_vk(keysym) {
        set_event_key(vk, true);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn SDL_KeyUp(_keymod: c_int, keysym: c_int) {
    if let Some(vk) = sdl_scancode_to_vk(keysym) {
        set_event_key(vk, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_name_both_byte_orders() {
        let mut h = [0u8; 64];
        h[0x20..0x34].copy_from_slice(b"SUPER MARIO 64      ");
        assert_eq!(rom_name_from_header(&h, false), "SUPER MARIO 64");
        let mut s = [0u8; 64];
        for i in 0..64 {
            s[i ^ 3] = h[i];
        }
        assert_eq!(rom_name_from_header(&s, true), "SUPER MARIO 64");
    }
}

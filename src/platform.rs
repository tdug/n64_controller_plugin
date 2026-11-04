//! Paths, logging and OS helpers.

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
pub fn log(msg: &str) {
    web_sys_log(msg);
}

#[cfg(target_arch = "wasm32")]
fn web_sys_log(_msg: &str) {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;

    pub const APP_DIR: &str = "tdug-n64";

    /// Where config, paks and logs live.
    /// Portable mode: if a `tdug-n64` folder exists next to the plugin, use it.
    pub fn config_dir() -> PathBuf {
        if let Some(dir) = module_path().and_then(|p| p.parent().map(|d| d.join(APP_DIR))) {
            if dir.is_dir() {
                return dir;
            }
        }
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        };
        base.unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR)
    }

    static LOG: Mutex<()> = Mutex::new(());

    pub fn log(msg: &str) {
        let _g = LOG.lock();
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("log.txt");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
            let _ = std::fs::remove_file(&path);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = writeln!(f, "[{t}] {msg}");
        }
    }

    pub fn open_folder(path: &std::path::Path) {
        let _ = std::fs::create_dir_all(path);
        let cmd = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(cmd).arg(path).spawn();
    }

    pub fn open_file(path: &std::path::Path) {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else {
            std::process::Command::new("xdg-open")
        };
        let _ = cmd.arg(path).spawn();
    }

    /// Path of the plugin library (or executable) containing this code.
    #[cfg(windows)]
    pub fn module_path() -> Option<PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::System::LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            GetModuleFileNameW, GetModuleHandleExW,
        };
        let mut hmod = std::ptr::null_mut();
        // SAFETY: Win32 calls with valid out-pointers; address is a function in this module.
        unsafe {
            if GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                module_path as *const u16,
                &mut hmod,
            ) == 0
            {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let n = GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32) as usize;
            if n == 0 {
                return None;
            }
            Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])))
        }
    }

    #[cfg(not(windows))]
    pub fn module_path() -> Option<PathBuf> {
        // SAFETY: dladdr with a function address in this module and a valid out struct.
        unsafe {
            let mut info: DlInfo = std::mem::zeroed();
            if dladdr(module_path as *const std::ffi::c_void, &mut info) == 0
                || info.fname.is_null()
            {
                return None;
            }
            let s = std::ffi::CStr::from_ptr(info.fname)
                .to_string_lossy()
                .into_owned();
            Some(PathBuf::from(s))
        }
    }

    #[cfg(not(windows))]
    #[repr(C)]
    struct DlInfo {
        fname: *const std::ffi::c_char,
        fbase: *mut std::ffi::c_void,
        sname: *const std::ffi::c_char,
        saddr: *mut std::ffi::c_void,
    }

    #[cfg(not(windows))]
    unsafe extern "C" {
        fn dladdr(addr: *const std::ffi::c_void, info: *mut DlInfo) -> i32;
    }

    /// True if keyboard polling should happen now (our process has focus, or background allowed).
    pub fn keyboard_allowed(background: bool) -> bool {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::GetCurrentProcessId;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetForegroundWindow, GetWindowThreadProcessId,
            };
            if background {
                return true;
            }
            // SAFETY: plain Win32 calls.
            unsafe {
                let mut pid = 0u32;
                GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
                pid == GetCurrentProcessId()
            }
        }
        #[cfg(not(windows))]
        {
            let _ = background;
            true
        }
    }
}

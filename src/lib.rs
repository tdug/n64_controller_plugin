//! TDug N64 Input — N64 controller plugin for Project64 (Zilmar spec) and Mupen64Plus,
//! with rumble, Tremor Pak (memory + rumble), analog calibration, custom curves and macros.

pub mod config;
pub mod gui;
pub mod input;
pub mod n64;
pub mod pak;
pub mod platform;
pub mod runtime;
pub mod stick;

#[cfg(not(target_arch = "wasm32"))]
pub mod plugin;

pub fn config_about() -> &'static str {
    concat!(
        "TDug N64 Input ",
        env!("CARGO_PKG_VERSION"),
        " · Zilmar spec 1.1 (Project64 etc.) + Mupen64Plus input API 2.x"
    )
}

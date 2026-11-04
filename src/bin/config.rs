//! Standalone configuration tool (desktop) / browser test harness (wasm).

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    tdug_plugin::gui::run_native();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    tdug_plugin::gui::run_web();
}

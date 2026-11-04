# AGENTS.md

Guidance for coding agents (Claude Code, Codex, Cursor, …) working in this repo.

## Project
N64 controller plugin in Rust. A single `cdylib` exports both the **Zilmar controller spec 1.1**
(Project64 and similar) and the **Mupen64Plus input API 2.x**. An egui config UI also builds to
wasm for the browser. User-facing docs are in `README.md`.

## Layout
- `src/plugin.rs`: C ABI exports for both APIs, the background input thread, and the out-of-process
  config UI (`rundll32 …,ConfigMain` on Windows).
- `src/runtime.rs`: PIF command handling, macros (advance once per game frame), modifiers, pak
  swapping, and `self_test()`.
- `src/pak.rs`: Controller Pak, Rumble Pak and Tremor Pak; data CRC; mempak format.
- `src/stick.rs`: calibration, deadzone, response curves, N64 gate.
- `src/input.rs`: binding model (`Input`), gilrs backend, keyboard, single-axis hat decoding.
- `src/config.rs`: JSON config (file on desktop, localStorage on web).
- `src/presets.rs` and `presets/controllers.yml`: macro presets and controller mapping presets.
  The YAML is embedded at build time; on desktop, a copy in the user's data folder overrides it.
- `src/gui.rs`: egui app shared by desktop and web. The web-only Debug tab is in `mod web_debug`.
- `tools/gamepad-listen/`: standalone crate that logs controller events. It is not part of the
  root build; see its `AGENTS.md`.
- `tools/hid-listen/`: standalone raw HID report listener, for Switch-mode pads.
- `skills/`: agent skills in SKILL.md format. `.claude/skills/` holds symlinks to them.

## Commands
```sh
cargo test                                               # unit tests, including PIF/pak self-test
cargo clippy --all-targets                               # CI runs this
cargo check --target i686-pc-windows-msvc                # 32-bit Project64 target
cargo build --release --target wasm32-unknown-unknown --bin tdug-n64-config   # web build
trunk serve                                              # web UI at http://127.0.0.1:8080
```
CI (`.github/workflows/ci.yml`) builds Windows x86/x64, Linux, macOS and web, deploys GitHub Pages
from `main`, and publishes release zips for `v*` tags.

## Rules
- **No C toolchain.** Don't add dependencies that need cmake or a C compiler (that's why gilrs is
  used instead of SDL).
- **Platform code.** Anything platform-specific is behind `cfg(windows)`, `cfg(target_arch = "wasm32")`
  and similar. Every change must still pass `cargo check` for `i686-pc-windows-msvc` and
  `wasm32-unknown-unknown`.
- **FFI exports.** Wrap the body of each export in `guard(...)` so panics never unwind into the
  emulator.
- **Browser-only features.** Features the browser build doesn't support are compiled out or hidden
  there (for example rumble, pak files and YAML export).
- **Controller presets.** Never invent button codes. Capture real data with the
  `controller-preset` skill.

## Skills
- `skills/controller-preset/SKILL.md`: capture a controller's codes and write a preset.

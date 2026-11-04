# TDug N64 Input

An N64 controller plugin written in Rust. One library works with **Project64** (Zilmar controller spec 1.1) and **Mupen64Plus** (input plugin API 2.x).

**[Try the config UI in your browser](https://tdug.github.io/n64_controller_plugin/)**: live stick visualization, calibration, response curves, macros and a PIF/pak self-test. No install needed.

## Features

- **4 controllers**, each bound to keyboard keys and/or any gamepad. Gamepads are read through [gilrs](https://gitlab.com/gilrs-project/gilrs): Windows.Gaming.Input on Windows, evdev on Linux, IOKit on macOS.
- **Bindings**: each N64 button and stick direction accepts several inputs. Analog axes, triggers and buttons can all be bound to anything.
- **Quick setup**
  - Controller presets: the 8BitDo 64 in D mode ("8BitDo 64 BT") and S mode ("N64 Controller", which also fits Nintendo's Switch Online N64 pad). When one of these is the selected gamepad, the UI offers a one-click button to apply its preset.
  - **Guided setup**: press each N64 button when prompted. It works with any adapter that has no built-in mapping (Adaptoid, Raphnet, unmapped modes), and it keeps your keyboard keys.
- **Analog stick**
  - Live view of the host stick input and the resulting N64 output.
  - **Range calibration**: record the resting center, then spin the stick around its gate. The recorded outline becomes the stick's 100 % reach in every direction, so a stick that is short in some directions still reaches full range.
  - Deadzone, saturation, output range, and an N64-style octagon gate (or a circle or square).
  - **Response curve**: an exponent, or a **custom curve** with draggable points (double-click adds a point, right-click removes one).
- **Modifiers**: while held (or toggled on), scale the stick. Useful as a walk button.
- **Macros**: frame-by-frame sequences of N64 inputs, where one frame is one controller poll by the game.
  - Triggered by a chord of inputs.
  - Merge with live input, or override it.
  - Play once, loop while held, or toggle a loop.
  - Edit the steps by hand, or record them from your controller.
  - Every step can set both buttons and the analog stick (x, y).
  - A macro advances one step per *game* frame: once per new frame in which the game reads the controller, not once per plugin call. A 30 fps game therefore plays a 10-frame step for 10 of its own frames.
  - **Presets**: built-in "SM64: Min Distance Triple Jump" and "OoT/MM: Quick Spin". Save your own macros as named presets; presets never store trigger bindings.
- **Paks**
  - Controller Pak (memory). Saved as standard 32 KiB `.mpk` files, one per game or one per port.
  - **Rumble Pak**. Drives your gamepad's motors.
  - **Tremor Pak**: memory and rumble in one pak, as the third-party combo paks did. The game sees a working Controller Pak and a working Rumble Pak at the same time.
  - The emulator's own Controller Pak (non-raw mode).
  - A hotkey to swap paks mid-game, such as Memory ↔ Rumble for games that ask you to swap.
- The emulator picks up config changes live, so there is no need to restart the game.

## Install

Download a build from [Releases](../../releases), or from the artifacts of the latest [CI run](../../actions).

| Emulator | File | Where |
|---|---|---|
| Project64 3.x, 1964, other 32-bit Zilmar-spec emulators | `windows-x86/TDug_N64_Input.dll` | `Plugin/Input/` |
| Project64 4.x (64-bit) | `windows-x64/TDug_N64_Input.dll` | `Plugin/Input/` |
| Mupen64Plus on Windows (RMG, ui-console) | the DLL that matches your emulator's bitness | the plugin directory |
| Mupen64Plus on Linux | `mupen64plus-input-tdug.so` | the plugin directory, e.g. `--input mupen64plus-input-tdug.so` |
| Mupen64Plus on macOS | `mupen64plus-input-tdug.dylib` | the plugin directory |

**Configure**
- **Project64**: *Options → Configure Controller Plugin*. The UI opens in its own window and saves straight to the config file.
- **Mupen64Plus**: run `tdug-n64-config` from the same folder.

**Data folder**: config, Controller Pak files and the log live in `%APPDATA%\tdug-n64` on Windows, `~/.config/tdug-n64` on Linux, and `~/Library/Application Support/tdug-n64` on macOS. For portable mode, create a `tdug-n64` folder next to the plugin.

## Rumble support

Rumble goes through whatever force-feedback API gilrs uses on each OS:

- **Windows**: Windows.Gaming.Input vibration. This covers Xbox-compatible controllers.
- **Linux**: evdev force feedback. This covers most pads: Xbox, DualShock/DualSense, Switch Pro and others.
- **macOS and browser**: no rumble.

The UI shows whether the selected gamepad reports rumble support.

## Browser build

The same UI compiles to WebAssembly and uses the browser Gamepad API.

- **Works**: bindings, calibration, curves, macros, and the self-test.
- **Disabled**: rumble (gilrs has no web force feedback), pak files, and the emulator plugin.
- **Debug tab** (browser only) shows three views:
  - the unfiltered Gamepad API: every button and axis index, plus the mapping type;
  - the plugin's view of each pad;
  - **raw WebHID input reports**: connect any HID device and watch its bytes change. Chrome and Edge only.
- **Saving**: config is saved to `localStorage`. Use *Settings → Import / export* to move it to a desktop install.

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
trunk serve            # http://127.0.0.1:8080
```

## Build

```sh
cargo build --release                                   # host OS
cargo build --release --target i686-pc-windows-msvc     # 32-bit Project64
cargo test
```

The build is pure Rust and needs no CMake. Linux also needs `libudev-dev`.

## How it works

- **One library, two plugin APIs.** Both APIs export the same entry points (`InitiateControllers`, `GetKeys`, `ReadController`, …). Mupen64Plus always calls `PluginStartup` first, so the plugin can tell which API it is running under.
- **Raw PIF mode.** The plugin runs in raw mode (`RawData = TRUE`, `PLUGIN_RAW`) and answers controller commands itself: status, poll, and pak read/write with the data CRC. That is how it can present a Tremor Pak and report rumble state no matter what pak types the emulator supports.
- **Background input thread.** A thread polls gamepads at about 1 kHz, drives rumble, hot-reloads the config and flushes pak files.
- **Config UI in a separate process.** On Windows it runs as `rundll32 TDug_N64_Input.dll,ConfigMain`, so its window and event loop never live inside the emulator process.

## Not supported

- **RetroArch**: libretro N64 cores (Mupen64Plus-Next, ParaLLEl-N64) take input from RetroArch's own input layer and can't load external input plugins.
- Transfer Pak and VRU.
- Mouse-as-stick.

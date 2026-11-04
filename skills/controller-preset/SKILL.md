---
name: controller-preset
description: Capture a physical controller's button/axis codes with tools/gamepad-listen and turn them into a preset in presets/controllers.yml. Use when the user wants to add or fix a controller mapping preset, or says a controller's buttons are mapped wrong.
---

# Controller preset capture

Create or fix a controller preset from real input data. Never guess button codes.

## Steps

1. **Build the listener.** It is a standalone crate and not part of the plugin build:
   ```sh
   cd tools/gamepad-listen && cargo build --release
   ```
2. **Start it in the background,** logging to a file in the scratchpad (never in the repo):
   ```sh
   tools/gamepad-listen/target/release/gamepad-listen > <scratchpad>/listen.log 2>&1
   ```
3. **Ask the user for one control at a time.** Send a single message naming one control, wait for
   them to reply, check the log tail (`tail -5 <scratchpad>/listen.log`) to confirm the event
   arrived, record the code, then ask for the next. This keeps every log line tied to a known
   control, with no timestamp guessing. Standard N64 order:
   1. Any button (to wake the controller), then wait 2 s.
   2. A, B, Z, Start, L, R.
   3. D-pad Up, Right, Down, Left.
   4. C Up, Right, Down, Left.
   5. Stick fully Up, Right, Down, Left, releasing each time.
   6. Any extra buttons, which the user names.

   If the controller has mode switches (for example 8BitDo 64 S/D), do one capture per mode and
   ask the user which mode it's in.
4. **Stop the listener** when done (`pkill -f gamepad-listen`). If a control produced no event,
   ask once more, then note it as unmapped. The log format and input forms are in
   `tools/gamepad-listen/README.md`; mapping rules are in `tools/gamepad-listen/AGENTS.md`.
5. **Edit `presets/controllers.yml`.** Add or replace the entry:
   - `names` / `ids` come from the `PAD` line.
   - `platforms` is required when any raw codes are used.
   - Fill in all 14 buttons and the 4 stick directions.
   - Preset input format and the file's own header comments are the reference.
6. **Verify:** run `cargo test presets` in the repo root.
7. **Browser-specific mappings** come from the web build's Debug tab, not this tool. The browser
   numbers buttons and axes differently from gilrs on desktop, and reports a D-pad as a hat axis
   (`raw_hat`). Ask the user to read those values off the Debug tab.

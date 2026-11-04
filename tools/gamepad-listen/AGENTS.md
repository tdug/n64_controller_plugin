# AGENTS.md: tools/gamepad-listen

Instructions for coding agents using this tool to capture controller mappings.

## Purpose
`gamepad-listen` logs gilrs gamepad events, mapped names and raw codes, one per line, to stdout.
Use the log to write or fix entries in `../../presets/controllers.yml`.

## Rules
- This crate is standalone: its `Cargo.toml` has its own `[workspace]` table. Don't add it to the
  root crate, and don't import it from there.
- Build and run it from this directory only: `cargo build --release`, then
  `./target/release/gamepad-listen`.
- Run it in the background, redirecting stdout to a file in your scratchpad, not in the repo. It
  runs until killed, so stop it with `pkill -f gamepad-listen` when finished.
- The user has to press the buttons. Ask for **one control per message**, then confirm it in the
  log tail before asking for the next.
- If your harness can stream a background process's output (for example Claude Code's Monitor
  tool), run `tail -f <log> | grep --line-buffered -E "PRESS|CONNECT|AXIS .* v=[+-](0\.9|1\.0)"`.
  Each press then reaches you as an event, so you can prompt for the next control without
  waiting for the user to reply.
- Without an agent, users can run `--guided`, which prompts them itself and prints a YAML preset.
- An empty log usually means the controller is asleep. Ask the user to press a button to wake it.
  Don't assume the tool is broken.

## Turning a log into a preset
1. Find the `PAD` or `CONNECT` line. Take the device name for `names:` and `vid:pid` for `ids:`.
2. Each `PRESS` or `AXIS` line belongs to the control you asked for just before it.
3. Pick the input form for each control:
   - mapped button → `{type: button, button: <Name>}`
   - mapped axis → `{type: axis, axis: <Name>, positive: <sign of v>}`
   - `Unknown` button → `{type: raw_button, code: <code>}`
   - `Unknown` axis → `{type: raw_axis, code: <code>, positive: <sign>}`
4. If the preset uses any raw codes, set `platforms:` to the OS you captured on.
5. Check the file still parses: run `cargo test presets` in the repo root. The
   `builtin_yaml_parses` and `suggests_8bitdo` tests cover it.

## Never
- Never invent codes for a controller you haven't captured.
- Never commit log files.

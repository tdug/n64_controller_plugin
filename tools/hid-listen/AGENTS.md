# AGENTS.md: tools/hid-listen

Raw HID report listener. Use it when a controller's OS gamepad mapping is wrong or missing,
typically Switch-protocol pads.

## Rules
- This crate is standalone: its `Cargo.toml` has its own `[workspace]` table. Build it from this
  directory with `cargo build --release`.
- Run `target/release/hid-listen <vvvv:pppp>` in the background with stdout going to a scratchpad
  log. Stop it with `pkill -f hid-listen`.
- Stream only button-sized changes. Analog sticks emit many lines, so filter to the byte range of
  the buttons and hat (e.g. `grep --line-buffered -E "byte [0-3]:"`). Read stick behaviour from
  the log afterwards.
- Ask for one control per message. For sticks, ask the user to move slowly to full tilt and say
  "done", then decode the 16-bit values from the log.
- Desktop logs include the report-ID byte (byte 0). Plugin HID bindings number bytes from after
  the ID, so subtract 1.

## Never
- Never send `--full` (or any output report) to a device that isn't a Switch-protocol controller.
- Never commit logs.

# gamepad-listen

A small command-line tool that prints every gamepad event as the plugin sees it. It uses
[gilrs](https://gitlab.com/gilrs-project/gilrs), the same backend as the plugin.

Use it to work out an unfamiliar controller's button and axis codes, so you can write a preset for
[`presets/controllers.yml`](../../presets/controllers.yml).

It is a standalone crate: building the plugin does not build this tool.

## Run

```sh
cd tools/gamepad-listen
cargo run --release | tee listen.log
```

Then press each control on the controller, one at a time.

### Guided mode

```sh
cargo run --release -- --guided
```

Guided mode asks for each N64 control in turn (A, B, Z, …, stick directions) and waits up to 10 s
for each; a control you don't press is skipped. At the end it prints a ready-to-paste YAML preset
entry on stdout; the prompts go to stderr. Each control gets one input; add extra inputs by hand
(for example a second Z trigger).

## Output

At startup, one line per connected pad:

```
PAD 0 name="N64 Controller" os_name="..." vid=Some(057e) pid=Some(2019) mapping=SdlMappings map_name=Some("...") ff=false
```

Then one line per event:

```
   3.214 pad0 PRESS   South code=65537
   3.390 pad0 RELEASE South code=65537
   5.002 pad0 AXIS    RightStickY code=196611 v=+1.000
   6.120 pad0 PRESS   Unknown code=65545
```

- **Mapped name** (`South`, `RightStickY`, …): gilrs recognised the control through its mapping
  database. In a preset, write it as `{type: button, button: South}` or
  `{type: axis, axis: RightStickY, positive: true}`.
- **`Unknown`**: there is no mapping for that control. Use the raw code instead:
  `{type: raw_button, code: 65545}` or `{type: raw_axis, code: N, positive: true|false}`.
  Raw codes depend on the OS, so also set `platforms: [macos]` (or whichever OS you tested on).
- **Stick noise**: axis changes smaller than ±0.3 are not printed.

## Caveats

- **macOS**: no rumble. Some Bluetooth pads only send events after you press a button to wake them.
- **Browser**: this tool can't show what the browser sees. The web build's **Debug** tab
  (https://tdug.github.io/n64_controller_plugin/) shows the Gamepad API and raw WebHID data.
- **Mode switches**: controllers that change mode (for example the 8BitDo 64's S/D switch) report a
  different name, id and codes in each mode. Capture each mode separately.

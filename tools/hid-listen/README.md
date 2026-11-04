# hid-listen

A command-line tool that shows a controller's raw HID input reports and marks every byte and bit
that changes. Use it to map controllers that the OS gamepad layer doesn't handle, for example the
8BitDo 64 in S mode or Nintendo's Switch Online N64 controller.

It is a standalone crate (pure Rust, using [async-hid](https://crates.io/crates/async-hid)), so
building the plugin does not build this tool.

## Run

```sh
cd tools/hid-listen
cargo run --release                         # list HID devices (vendor:product, usage, name)
cargo run --release -- 057e:2019            # watch reports from that device
cargo run --release -- 057e:2019 --full     # first switch a Switch-protocol pad to full report mode
cargo run --release -- 057e:2019 --ignore 4-11   # hide noisy bytes (e.g. analog sticks)
```

## Output

```
   3.120 len=12   3f [02] 00  08  ff  7f  ff  7f  00  80  00  80
         byte 1: 00→02  bit 1 0→1
```

- A line is printed only when a report changes. Changed bytes appear in `[brackets]`, followed by
  each bit that flipped.
- On desktop, byte 0 is the report ID (`0x3f` is Nintendo's basic HID mode). Browsers (WebHID)
  give the report without that ID byte, so the plugin numbers bytes **after** the ID: plugin
  byte = tool byte − 1.
- **Buttons** flip a single bit.
- **Hats** (D-pads) use one byte holding the values 0–7, clockwise from up, with 8 meaning released.
- **Sticks** are 16-bit little-endian values centred at 0x8000.

## 8BitDo 64, S mode (basic report 0x3F, macOS)

Tool byte numbers (subtract 1 for plugin byte numbers):

| Control | Tool byte | Detail |
|---|---|---|
| A | 1 | bit 1 |
| B | 1 | bit 0 |
| Z (left trigger) | 1 | bit 6 |
| Z (right trigger) | 2 | bit 2 |
| Start | 2 | bit 1 |
| L | 1 | bit 4 |
| R | 1 | bit 5 |
| C-Up | 1 | bit 2 |
| C-Right | 2 | bit 0 |
| C-Down | 1 | bit 7 |
| C-Left | 1 | bit 3 |
| Home | 2 | bit 4 |
| Star | 2 | bit 5 |
| D-pad | 3 | hat: 0 up, 2 right, 4 down, 6 left, 8 released |
| Stick X | 4–5 | u16 little-endian, right = higher |
| Stick Y | 6–7 | u16 little-endian, **up = lower** (inverted) |
| unused | 8–11 | always 0x8000 |

Dash and the stick click send nothing in this mode. Basic mode already carries full-resolution
16-bit stick values, so no handshake is needed.

## Caveats

- **macOS**: the controller only appears after you wake it. If the device list shows no keyboards
  or gamepads at all, the terminal may need *Input Monitoring* permission.
- **`--full`**: sends Nintendo subcommand 0x03 0x30. Use it only on Switch-protocol devices.

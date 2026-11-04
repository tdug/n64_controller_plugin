//! N64 controller data types and wire encoding.

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum N64Button {
    A,
    B,
    Z,
    Start,
    DUp,
    DDown,
    DLeft,
    DRight,
    L,
    R,
    CUp,
    CDown,
    CLeft,
    CRight,
}

impl N64Button {
    pub const ALL: [N64Button; 14] = [
        N64Button::A,
        N64Button::B,
        N64Button::Z,
        N64Button::Start,
        N64Button::L,
        N64Button::R,
        N64Button::DUp,
        N64Button::DDown,
        N64Button::DLeft,
        N64Button::DRight,
        N64Button::CUp,
        N64Button::CDown,
        N64Button::CLeft,
        N64Button::CRight,
    ];

    /// Bit in the 16-bit button word, laid out as the two bytes sent on the wire
    /// (`[A B Z Start DU DD DL DR] [Rst - L R CU CD CL CR]`).
    pub fn mask(self) -> u16 {
        use N64Button::*;
        match self {
            A => 0x8000,
            B => 0x4000,
            Z => 0x2000,
            Start => 0x1000,
            DUp => 0x0800,
            DDown => 0x0400,
            DLeft => 0x0200,
            DRight => 0x0100,
            L => 0x0020,
            R => 0x0010,
            CUp => 0x0008,
            CDown => 0x0004,
            CLeft => 0x0002,
            CRight => 0x0001,
        }
    }

    pub fn label(self) -> &'static str {
        use N64Button::*;
        match self {
            A => "A",
            B => "B",
            Z => "Z",
            Start => "Start",
            DUp => "D-Pad Up",
            DDown => "D-Pad Down",
            DLeft => "D-Pad Left",
            DRight => "D-Pad Right",
            L => "L",
            R => "R",
            CUp => "C-Up",
            CDown => "C-Down",
            CLeft => "C-Left",
            CRight => "C-Right",
        }
    }

    pub fn short(self) -> &'static str {
        use N64Button::*;
        match self {
            A => "A",
            B => "B",
            Z => "Z",
            Start => "S",
            DUp => "D↑",
            DDown => "D↓",
            DLeft => "D←",
            DRight => "D→",
            L => "L",
            R => "R",
            CUp => "C↑",
            CDown => "C↓",
            CLeft => "C←",
            CRight => "C→",
        }
    }
}

/// One poll worth of N64 controller state.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct N64State {
    pub buttons: u16,
    pub x: i8,
    pub y: i8,
}

impl N64State {
    /// The 4 response bytes of PIF command 0x01.
    pub fn wire(&self) -> [u8; 4] {
        [
            (self.buttons >> 8) as u8,
            self.buttons as u8,
            self.x as u8,
            self.y as u8,
        ]
    }

    /// Value of the `BUTTONS` union used by `GetKeys` (Zilmar and Mupen64Plus).
    /// Its in-memory (little-endian) layout matches the wire bytes.
    pub fn keys_value(&self) -> u32 {
        u32::from_le_bytes(self.wire())
    }

    pub fn pressed(&self, b: N64Button) -> bool {
        self.buttons & b.mask() != 0
    }
}

//! Controller Pak (memory), Rumble Pak and combined "Tremor" pak emulation.

use crate::config::PakType;
use std::path::PathBuf;

pub const MEMPAK_SIZE: usize = 0x8000;

/// CRC over a 32-byte pak data block (poly 0x85), as computed by the controller.
pub fn data_crc(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for i in 0..=data.len() {
        let byte = data.get(i).copied().unwrap_or(0);
        for bit in (0..8).rev() {
            let xor = if crc & 0x80 != 0 { 0x85 } else { 0 };
            crc <<= 1;
            if i < data.len() && byte & (1 << bit) != 0 {
                crc |= 1;
            }
            crc ^= xor;
        }
    }
    crc
}

/// A freshly formatted 32 KiB Controller Pak image (libultra filesystem layout).
pub fn format_mempak() -> Vec<u8> {
    let mut m = vec![0u8; MEMPAK_SIZE];
    // Page 0 block 0: label area.
    m[0] = 0x81;
    for (i, b) in m[1..0x20].iter_mut().enumerate() {
        *b = (i + 1) as u8;
    }
    // ID block, repeated at the four standard locations.
    let mut id = [0u8; 32];
    id[0..4].copy_from_slice(&[0xFF; 4]); // repaired
    id[4..8].copy_from_slice(&[0x05, 0x1A, 0x5F, 0x13]); // random
    id[16..24].copy_from_slice(&[0xFF; 8]); // serial
    id[24..26].copy_from_slice(&[0xFF, 0xFF]); // device id (bit 0 set)
    id[26] = 0x01; // banks
    id[27] = 0xFF; // version
    let sum: u32 = (0..14)
        .map(|i| u16::from_be_bytes([id[i * 2], id[i * 2 + 1]]) as u32)
        .sum();
    let sum = (sum & 0xFFFF) as u16;
    id[28..30].copy_from_slice(&sum.to_be_bytes());
    id[30..32].copy_from_slice(&0xFFF2u16.wrapping_sub(sum).to_be_bytes());
    for off in [0x20, 0x60, 0x80, 0xC0] {
        m[off..off + 32].copy_from_slice(&id);
    }
    // Pages 1 and 2: inode table and its backup. Pages 0..=4 are system; the rest free (0x0003).
    for page in [0x100, 0x200] {
        for e in 1..128 {
            m[page + e * 2] = 0x00;
            m[page + e * 2 + 1] = 0x03;
        }
        let checksum = m[page + 10..page + 256]
            .iter()
            .fold(0u8, |a, b| a.wrapping_add(*b));
        m[page] = 0;
        m[page + 1] = checksum;
    }
    m
}

pub struct Pak {
    pub kind: PakType,
    mem: Option<Vec<u8>>,
    path: Option<PathBuf>,
    dirty: bool,
}

impl Pak {
    pub fn none() -> Self {
        Pak {
            kind: PakType::None,
            mem: None,
            path: None,
            dirty: false,
        }
    }

    pub fn new(kind: PakType, path: Option<PathBuf>) -> Self {
        let mem = kind.has_memory().then(|| {
            path.as_ref()
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|d| {
                    // Accept raw .mpk or DexDrive-style files with a header in front.
                    (d.len() >= MEMPAK_SIZE).then(|| d[d.len() - MEMPAK_SIZE..].to_vec())
                })
                .unwrap_or_else(format_mempak)
        });
        Pak {
            kind,
            mem,
            path,
            dirty: false,
        }
    }

    pub fn present(&self) -> bool {
        !matches!(self.kind, PakType::None | PakType::EmulatorMemory)
    }

    pub fn read(&self, addr: u16, out: &mut [u8]) {
        let a = addr as usize;
        let fill = |out: &mut [u8], v| out.iter_mut().for_each(|b| *b = v);
        match (&self.mem, a) {
            (Some(m), 0..0x8000) => out.copy_from_slice(&m[a..a + out.len()]),
            (_, 0x8000..0x9000) if self.kind.has_rumble() => fill(out, 0x80),
            _ => fill(out, 0x00),
        }
    }

    /// Returns `Some(on)` if this write changed the rumble motor command.
    pub fn write(&mut self, addr: u16, data: &[u8]) -> Option<bool> {
        let a = addr as usize;
        match a {
            0..0x8000 => {
                if let Some(m) = &mut self.mem {
                    m[a..a + data.len()].copy_from_slice(data);
                    self.dirty = true;
                }
                None
            }
            0xC000..0xD000 if self.kind.has_rumble() => Some(data[0] != 0),
            _ => None,
        }
    }

    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let (Some(m), Some(p)) = (&self.mem, &self.path) {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(p, m) {
                crate::platform::log(&format!("failed to save pak {}: {e}", p.display()));
            }
        }
    }
}

impl Drop for Pak {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_values() {
        assert_eq!(data_crc(&[0u8; 32]), 0x00);
        assert_eq!(data_crc(&[0x80u8; 32]), 0xB8);
    }

    #[test]
    fn format_matches_reference_id_checksum() {
        let m = format_mempak();
        assert_eq!(&m[0x3C..0x40], &[0x66, 0x25, 0x99, 0xCD]);
        assert_eq!(m[0x101], 0x71);
        assert_eq!(m[0x201], 0x71);
    }

    #[test]
    fn tremor_behaves_as_both() {
        let mut p = Pak::new(PakType::Tremor, None);
        let mut buf = [0u8; 32];
        // rumble detect: write 0xFE then read must not echo 0xFE
        p.write(0x8000, &[0xFE; 32]);
        p.read(0x8000, &mut buf);
        assert_eq!(buf, [0x80; 32]);
        assert_eq!(p.write(0xC000, &[1; 32]), Some(true));
        assert_eq!(p.write(0xC000, &[0; 32]), Some(false));
        p.write(0x0200, &[0xAB; 32]);
        p.read(0x0200, &mut buf);
        assert_eq!(buf, [0xAB; 32]);
    }

    #[test]
    fn mempak_is_not_rumble() {
        let p = Pak::new(PakType::Memory, None);
        let mut buf = [1u8; 32];
        p.read(0x8000, &mut buf);
        assert_eq!(buf, [0; 32]);
    }
}

//! 48K memory map: 16K ROM (read-only) + 48K RAM in a flat 64K space, no MMU.
//!
//! Written against a bank-capable shape so wiring 128K paging later is cheap
//! (`docs/01-core-emulator-spec.md` §10), but only the 48K config is wired now.

const ROM_SIZE: usize = 0x4000; // 16K
const RAM_SIZE: usize = 0xC000; // 48K

pub struct Memory {
    rom: [u8; ROM_SIZE],
    ram: [u8; RAM_SIZE],
}

impl Memory {
    /// Construct with the given ROM image. Bytes beyond `rom.len()` (or all of
    /// it, if empty) are zero-filled.
    pub fn new_48k(rom: &[u8]) -> Self {
        let mut r = [0u8; ROM_SIZE];
        let n = rom.len().min(ROM_SIZE);
        r[..n].copy_from_slice(&rom[..n]);
        Self {
            rom: r,
            ram: [0u8; RAM_SIZE],
        }
    }

    #[inline]
    pub fn read(&self, addr: u16) -> u8 {
        let a = addr as usize;
        if a < ROM_SIZE {
            self.rom[a]
        } else {
            self.ram[a - ROM_SIZE]
        }
    }

    #[inline]
    pub fn write(&mut self, addr: u16, val: u8) {
        let a = addr as usize;
        if a >= ROM_SIZE {
            self.ram[a - ROM_SIZE] = val;
        }
        // Writes to the ROM region are ignored on a 48K.
    }

    /// Borrow the 48K RAM (screen lives at the bottom of it: 0x4000..0x5B00).
    #[inline]
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }
}

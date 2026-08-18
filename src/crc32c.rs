//! CRC-32C (Castagnoli), the checksum PostgreSQL stores in `XLogRecord.xl_crc`.
//!
//! Reflected form, polynomial 0x82F63B78, initial and final value 0xFFFFFFFF,
//! matching `INIT_CRC32C` / `COMP_CRC32C` / `FIN_CRC32C` in port/pg_crc32c.h.

const POLY: u32 = 0x82F6_3B78;

const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// Incremental CRC-32C, so a record's body and header can be fed separately
/// in the same order PostgreSQL uses.
pub struct Crc32c(u32);

impl Crc32c {
    /// `INIT_CRC32C`
    pub fn new() -> Self {
        Crc32c(0xFFFF_FFFF)
    }

    /// `COMP_CRC32C`
    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.0;
        for &b in data {
            crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        self.0 = crc;
    }

    /// `FIN_CRC32C`
    pub fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

impl Default for Crc32c {
    fn default() -> Self {
        Self::new()
    }
}

/// One-shot CRC-32C of a single buffer.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut c = Crc32c::new();
    c.update(data);
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_check_values() {
        // The standard CRC-32C check value.
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0x0000_0000);
        assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
        assert_eq!(crc32c(&[0xFFu8; 32]), 0x62A8_AB43);
    }

    #[test]
    fn incremental_matches_one_shot() {
        let data: Vec<u8> = (0u8..=255).collect();
        let mut c = Crc32c::new();
        c.update(&data[..7]);
        c.update(&data[7..100]);
        c.update(&data[100..]);
        assert_eq!(c.finish(), crc32c(&data));
    }
}

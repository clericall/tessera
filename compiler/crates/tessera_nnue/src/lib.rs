//! Binary format for Tessera NNUE weight files (`.tnn`), version 1.
//!
//! Everything is little-endian. The layout is deliberately sequential so the
//! engine can stream it with `fopen` + `fread_i16` / `fread_i32` and no seeking.
//!
//! ```text
//! offset  type         field
//! 0       u8[4]        magic  "TNNU"
//! 4       i32          version (= 1)
//! 8       i32          hsize  (hidden layer width; must match the engine's NH)
//! 12      i32          scale  (output divisor: score = value * scale / 64)
//! 16      i16[hsize]   bias   (layer-1 bias, seeded into the accumulator)
//! ...     i16[hsize]   w2     (output-layer weights, one per hidden unit)
//! ...     i16[768]     psqt   (per-feature bonus, side-to-move perspective)
//! ...     i16[768*h]   w1w    (white-view transformer: feature * hsize + unit)
//! ...     i16[768*h]   w1b    (black-view transformer: feature * hsize + unit)
//! ```
//!
//! Feature index for a piece of type `pt` (1..6) on square `sq`, from the side
//! to move's point of view:
//!
//! ```text
//! rel = (stm == 0) ? sq : 63 - sq            // mirror when black is to move
//! idx = (own ? pt - 1 : 6 + (pt - 1)) * 64 + rel
//! ```
//!
//! Evaluation:
//!
//! ```text
//! acc[i]   = clip_i16(bias[i] + sum over active features f of w1[f * hsize + i])
//! hidden   = clip_i16( sum_i acc[i] * w2[i] )
//! value    = clip_i16( sum over active features f of psqt[f] + hidden )
//! score    = value * scale / 64
//! ```

pub const MAGIC: &[u8; 4] = b"TNNU";
pub const VERSION: i32 = 1;
pub const FEATURES: usize = 768;

/// Total size in bytes of a weight file with the given hidden width.
pub fn file_len(hsize: i32) -> usize {
    16 + hsize as usize * 2 * 2 + FEATURES * 2 + 2 * FEATURES * hsize as usize * 2
}

fn push_i16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Writes a valid, deterministic weight file. The weights come from a tiny
/// xorshift PRNG, so the file is reproducible and obviously *not* a trained
/// network -- it exists so the inference path can be exercised end to end.
pub fn write_fake(path: &str, hsize: i32, seed: u64) -> std::io::Result<usize> {
    let mut state = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut bytes: Vec<u8> = Vec::with_capacity(file_len(hsize));
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&hsize.to_le_bytes());
    bytes.extend_from_slice(&400i32.to_le_bytes()); // scale
    for _ in 0..hsize {
        push_i16(&mut bytes, (next() % 64) as i16);
    }
    for _ in 0..hsize {
        push_i16(&mut bytes, 64 + (next() % 64) as i16);
    }
    for _ in 0..FEATURES {
        push_i16(&mut bytes, (next() % 64) as i16 - 32);
    }
    for _ in 0..2 * FEATURES * hsize as usize {
        push_i16(&mut bytes, (next() % 64) as i16 - 32);
    }
    std::fs::write(path, &bytes)?;
    Ok(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(name);
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn fake_file_has_the_advertised_size() {
        let h = 32i32;
        let path = tmp("tessera_nnue_size.tnn");
        let n = write_fake(&path, h, 7).unwrap();
        assert_eq!(n, file_len(h));
        assert_eq!(std::fs::metadata(&path).unwrap().len() as usize, n);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn header_is_magic_version_hsize_scale() {
        let path = tmp("tessera_nnue_header.tnn");
        write_fake(&path, 32, 1).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], MAGIC);
        assert_eq!(i32::from_le_bytes(bytes[4..8].try_into().unwrap()), VERSION);
        assert_eq!(i32::from_le_bytes(bytes[8..12].try_into().unwrap()), 32);
        assert_eq!(i32::from_le_bytes(bytes[12..16].try_into().unwrap()), 400);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn weight_table_starts_after_the_psqt_block() {
        // The last table must be able to hold 2 * 768 * hsize i16 values.
        let h = 8i32;
        assert_eq!(file_len(h), 16 + h as usize * 4 + 768 * 2 + 2 * 768 * h as usize * 2);
    }
}

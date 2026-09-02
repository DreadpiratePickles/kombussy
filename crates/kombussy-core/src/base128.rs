//! WOFF2 `UIntBase128`: big-endian, seven bits per byte, high bit continues.
//! The specification rejects overlong and non-minimal encodings, so we do too.

use crate::error::{FontError, Result};
use crate::read::Reader;

const MAX_BYTES: usize = 5;

pub fn read(r: &mut Reader<'_>) -> Result<u32> {
    let mut value: u32 = 0;
    for i in 0..MAX_BYTES {
        let byte = r.u8("UIntBase128 value")?;
        // A leading 0x80 encodes a redundant high zero group.
        if i == 0 && byte == 0x80 {
            return Err(FontError::BadBase128 {
                detail: "leading zero group",
            });
        }
        // Five groups of seven bits exceed 32 unless the top group is small.
        if value > (u32::MAX >> 7) {
            return Err(FontError::BadBase128 {
                detail: "value exceeds u32",
            });
        }
        value = (value << 7) | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(FontError::BadBase128 {
        detail: "more than 5 bytes",
    })
}

pub fn write(out: &mut Vec<u8>, value: u32) {
    // Emit the minimal number of seven-bit groups, most significant first.
    let mut groups = [0u8; MAX_BYTES];
    let mut count = 0usize;
    let mut v = value;
    loop {
        groups[count] = (v & 0x7f) as u8;
        count += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..count).rev() {
        let is_last = i == 0;
        out.push(groups[i] | if is_last { 0 } else { 0x80 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: u32) -> u32 {
        let mut buf = Vec::new();
        write(&mut buf, v);
        read(&mut Reader::new(&buf)).expect("decodes")
    }

    #[test]
    fn roundtrips_boundary_values() {
        for v in [
            0u32,
            1,
            127,
            128,
            16_383,
            16_384,
            1 << 21,
            1 << 28,
            u32::MAX,
        ] {
            assert_eq!(roundtrip(v), v, "value {v}");
        }
    }

    #[test]
    fn minimal_encoding_lengths() {
        let mut buf = Vec::new();
        write(&mut buf, 127);
        assert_eq!(buf.len(), 1);
        buf.clear();
        write(&mut buf, 128);
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn rejects_leading_zero_group() {
        let err = read(&mut Reader::new(&[0x80, 0x01])).unwrap_err();
        assert!(matches!(err, FontError::BadBase128 { .. }));
    }

    #[test]
    fn rejects_overlong_encoding() {
        let err = read(&mut Reader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0x7f])).unwrap_err();
        assert!(matches!(err, FontError::BadBase128 { .. }));
    }
}

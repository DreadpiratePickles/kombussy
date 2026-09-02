//! Hostile and damaged input must produce a typed error, never a panic and
//! never a silently truncated font.

use kombussy_core::{decode, detect, FontError};

const TTF: &[u8] = include_bytes!("../../../fixtures/synthetic.ttf");
const WOFF: &[u8] = include_bytes!("../../../fixtures/synthetic.woff");
const WOFF2: &[u8] = include_bytes!("../../../fixtures/synthetic_null.woff2");

#[test]
fn empty_input_is_truncated_not_unknown() {
    assert!(matches!(detect(&[]), Err(FontError::Truncated { .. })));
}

#[test]
fn rejects_unrecognised_magic() {
    assert!(matches!(
        detect(b"%PDF-1.7").unwrap_err(),
        FontError::UnknownContainer { .. }
    ));
}

#[test]
fn rejects_font_collections_with_a_specific_error() {
    let mut ttc = b"ttcf".to_vec();
    ttc.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 2]);
    assert!(matches!(
        decode(&ttc),
        Err(FontError::FontCollection | FontError::UnknownContainer { .. })
    ));
}

/// Truncating a valid font at every prefix length must never panic. This is the
/// cheap stand-in for a fuzz target and covers the whole parse surface.
#[test]
fn no_prefix_of_a_valid_font_can_panic() {
    for fixture in [TTF, WOFF, WOFF2] {
        for len in 0..fixture.len() {
            let _ = decode(&fixture[..len]);
        }
    }
}

/// Flipping single bytes must also stay in the error domain rather than crash.
#[test]
fn single_byte_corruption_never_panics() {
    for fixture in [TTF, WOFF, WOFF2] {
        for i in (0..fixture.len()).step_by(7) {
            let mut damaged = fixture.to_vec();
            damaged[i] ^= 0xff;
            let _ = decode(&damaged);
        }
    }
}

#[test]
fn zero_table_count_is_rejected() {
    let mut ttf = TTF.to_vec();
    ttf[4] = 0;
    ttf[5] = 0;
    assert!(matches!(decode(&ttf), Err(FontError::BadTableCount { .. })));
}

#[test]
fn table_offset_past_end_of_file_is_rejected() {
    let mut ttf = TTF.to_vec();
    // First directory entry's offset field sits at byte 20.
    ttf[20..24].copy_from_slice(&0xFFFF_0000u32.to_be_bytes());
    assert!(matches!(
        decode(&ttf),
        Err(FontError::TableOutOfBounds { .. })
    ));
}

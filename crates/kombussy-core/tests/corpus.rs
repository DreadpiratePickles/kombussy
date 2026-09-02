//! Tests against fixtures written by fontTools.
//!
//! These are the tests that matter: agreeing with an independent, mature
//! implementation is evidence, whereas a round trip through our own encoder and
//! decoder would pass even if both sides shared the same misreading of the spec.

use kombussy_core::{decode, detect, encode, sfnt::Font, Format};

const TTF: &[u8] = include_bytes!("../../../fixtures/synthetic.ttf");
const OTF: &[u8] = include_bytes!("../../../fixtures/synthetic.otf");
const WOFF: &[u8] = include_bytes!("../../../fixtures/synthetic.woff");
const WOFF_CFF: &[u8] = include_bytes!("../../../fixtures/synthetic_cff.woff");
const WOFF2_NULL: &[u8] = include_bytes!("../../../fixtures/synthetic_null.woff2");
const WOFF2_CFF: &[u8] = include_bytes!("../../../fixtures/synthetic_cff.woff2");
const WOFF2_TRANSFORMED: &[u8] = include_bytes!("../../../fixtures/synthetic_transformed.woff2");

/// Byte offsets inside `head` that legitimately differ between two encodings
/// of the same font, and so must be masked before comparing table bytes:
///
/// * `checkSumAdjustment` (8..12) is derived from the finished file, so any
///   change in table order or padding changes it by definition.
/// * `flags` bit 11 (16..18) is the "lossless transform" marker that the WOFF2
///   specification requires a producer to set. A WOFF2 of a font therefore
///   always differs here from the plain sfnt it came from.
///
/// Everything else is compared byte for byte.
const HEAD_CHECKSUM_ADJUSTMENT: std::ops::Range<usize> = 8..12;
const HEAD_FLAGS: std::ops::Range<usize> = 16..18;
const HEAD_FLAG_LOSSLESS: u16 = 1 << 11;

fn normalise_head(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    if let Some(field) = out.get_mut(HEAD_CHECKSUM_ADJUSTMENT) {
        field.fill(0);
    }
    if let Some(field) = out.get_mut(HEAD_FLAGS) {
        let masked = u16::from_be_bytes([field[0], field[1]]) & !HEAD_FLAG_LOSSLESS;
        field.copy_from_slice(&masked.to_be_bytes());
    }
    out
}

/// Tables as a sorted (tag, bytes) list, so comparisons ignore the storage
/// order a container happened to choose.
fn contents(font: &Font) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<_> = font
        .tables
        .iter()
        .map(|t| {
            let tag = String::from_utf8_lossy(&t.tag).to_string();
            let data = if t.tag == *b"head" {
                normalise_head(&t.data)
            } else {
                t.data.clone()
            };
            (tag, data)
        })
        .collect();
    v.sort();
    v
}

fn assert_same_font(left: &Font, right: &Font, context: &str) {
    assert_eq!(
        left.flavor, right.flavor,
        "{context}: outline flavor differs"
    );
    let (l, r) = (contents(left), contents(right));
    let l_tags: Vec<_> = l.iter().map(|(t, _)| t.clone()).collect();
    let r_tags: Vec<_> = r.iter().map(|(t, _)| t.clone()).collect();
    assert_eq!(l_tags, r_tags, "{context}: table inventory differs");
    for ((tag, a), (_, b)) in l.iter().zip(r.iter()) {
        assert_eq!(a, b, "{context}: table '{tag}' content differs");
    }
}

#[test]
fn detects_every_fixture_container() {
    assert_eq!(detect(TTF).unwrap(), Format::Sfnt);
    assert_eq!(detect(OTF).unwrap(), Format::Sfnt);
    assert_eq!(detect(WOFF).unwrap(), Format::Woff1);
    assert_eq!(detect(WOFF2_NULL).unwrap(), Format::Woff2);
}

#[test]
fn reads_fonttools_woff1_identically_to_the_source_ttf() {
    assert_same_font(
        &decode(WOFF).unwrap(),
        &decode(TTF).unwrap(),
        "woff1 vs ttf",
    );
}

#[test]
fn reads_fonttools_woff1_cff_identically_to_the_source_otf() {
    assert_same_font(
        &decode(WOFF_CFF).unwrap(),
        &decode(OTF).unwrap(),
        "woff1 cff vs otf",
    );
}

#[test]
fn reads_fonttools_null_transform_woff2_identically_to_the_source_ttf() {
    assert_same_font(
        &decode(WOFF2_NULL).unwrap(),
        &decode(TTF).unwrap(),
        "woff2 vs ttf",
    );
}

#[test]
fn reads_fonttools_null_transform_woff2_cff() {
    assert_same_font(
        &decode(WOFF2_CFF).unwrap(),
        &decode(OTF).unwrap(),
        "woff2 cff vs otf",
    );
}

#[test]
fn reconstructs_a_transformed_glyf_table() {
    // fontTools transforms glyf/loca by default, which is what real WOFF2 files
    // in the wild look like. Reconstruction discards the original's flag and
    // coordinate packing, so glyf/loca bytes are not comparable; every other
    // table must survive untouched, and the outlines are checked against
    // fontTools in fixtures/verify_interop.py.
    let rebuilt = decode(WOFF2_TRANSFORMED).expect("transformed glyf should reconstruct");
    let source = decode(TTF).unwrap();

    let mut rebuilt_tags: Vec<_> = rebuilt.tables.iter().map(|t| t.tag).collect();
    let mut source_tags: Vec<_> = source.tables.iter().map(|t| t.tag).collect();
    rebuilt_tags.sort();
    source_tags.sort();
    assert_eq!(
        rebuilt_tags, source_tags,
        "table inventory changed during reconstruction"
    );

    for table in &rebuilt.tables {
        if table.tag == *b"glyf" || table.tag == *b"loca" || table.tag == *b"head" {
            continue;
        }
        let original = source.table(&table.tag).expect("table present in source");
        assert_eq!(
            table.data,
            original.data,
            "table '{}' altered",
            String::from_utf8_lossy(&table.tag)
        );
    }

    let glyf = rebuilt.table(b"glyf").expect("glyf rebuilt");
    let loca = rebuilt.table(b"loca").expect("loca rebuilt");
    assert!(!glyf.data.is_empty(), "reconstructed glyf is empty");

    // maxp.numGlyphs sits at byte 4; loca must hold one offset per glyph plus a
    // terminator, in whichever width the transform header selected.
    let maxp = rebuilt.table(b"maxp").expect("maxp present");
    let num_glyphs = u16::from_be_bytes([maxp.data[4], maxp.data[5]]) as usize;
    let entries = num_glyphs + 1;
    assert!(
        loca.data.len() == entries * 2 || loca.data.len() == entries * 4,
        "loca length {} matches neither short nor long format for {num_glyphs} glyphs",
        loca.data.len()
    );
}

#[test]
fn a_reconstructed_font_can_be_re_encoded() {
    // The reconstructed tables have to be good enough to feed straight back
    // into every encoder, not just to inspect.
    let rebuilt = decode(WOFF2_TRANSFORMED).unwrap();
    for target in [Format::Sfnt, Format::Woff1, Format::Woff2] {
        let bytes = encode(&rebuilt, target).unwrap();
        let round_tripped = decode(&bytes).unwrap();
        assert_eq!(
            round_tripped.table(b"glyf").map(|t| t.data.clone()),
            rebuilt.table(b"glyf").map(|t| t.data.clone()),
            "glyf changed re-encoding to {target:?}"
        );
    }
}

#[test]
fn preserves_table_content_across_every_conversion_target() {
    for (name, source) in [("ttf", TTF), ("otf", OTF)] {
        let original = decode(source).unwrap();
        for target in [Format::Sfnt, Format::Woff1, Format::Woff2] {
            let bytes = encode(&original, target).unwrap();
            let back = decode(&bytes).unwrap();
            assert_same_font(&original, &back, &format!("{name} -> {target:?} -> decode"));
        }
    }
}

#[test]
fn converts_between_woff_variants_without_touching_the_ttf() {
    let from_woff1 = decode(WOFF).unwrap();
    let as_woff2 = encode(&from_woff1, Format::Woff2).unwrap();
    assert_same_font(
        &decode(&as_woff2).unwrap(),
        &decode(TTF).unwrap(),
        "woff1 -> woff2",
    );
}

#[test]
fn sfnt_serialisation_is_byte_stable() {
    // Re-encoding an already-normalised font must be a fixed point, otherwise
    // checksum patching or table ordering is non-deterministic.
    let once = encode(&decode(TTF).unwrap(), Format::Sfnt).unwrap();
    let twice = encode(&decode(&once).unwrap(), Format::Sfnt).unwrap();
    assert_eq!(once, twice, "sfnt encoding is not idempotent");
}

#[test]
fn woff2_is_smaller_than_woff1_for_the_same_font() {
    let font = decode(TTF).unwrap();
    let w1 = encode(&font, Format::Woff1).unwrap().len();
    let w2 = encode(&font, Format::Woff2).unwrap().len();
    assert!(w2 < w1, "expected woff2 ({w2}) to beat woff1 ({w1})");
}

#[test]
fn extension_follows_the_outline_flavor() {
    assert_eq!(Format::Sfnt.extension(decode(TTF).unwrap().flavor), "ttf");
    assert_eq!(Format::Sfnt.extension(decode(OTF).unwrap().flavor), "otf");
    assert_eq!(
        Format::Woff2.extension(decode(OTF).unwrap().flavor),
        "woff2"
    );
}

#[test]
fn woff2_encoder_sets_the_lossless_transform_flag() {
    // Required of WOFF2 producers, and the reason `normalise_head` exists.
    let font = decode(TTF).unwrap();
    let woff2 = encode(&font, Format::Woff2).unwrap();
    let head = decode(&woff2).unwrap().table(b"head").unwrap().data.clone();
    let flags = u16::from_be_bytes([head[16], head[17]]);
    assert_eq!(
        flags & HEAD_FLAG_LOSSLESS,
        HEAD_FLAG_LOSSLESS,
        "head.flags bit 11 not set"
    );
}

#[test]
fn plain_sfnt_output_does_not_claim_a_lossless_transform() {
    let font = decode(TTF).unwrap();
    let ttf = encode(&font, Format::Sfnt).unwrap();
    let head = decode(&ttf).unwrap().table(b"head").unwrap().data.clone();
    let flags = u16::from_be_bytes([head[16], head[17]]);
    assert_eq!(
        flags & HEAD_FLAG_LOSSLESS,
        0,
        "sfnt output must not set the WOFF2 producer flag"
    );
}

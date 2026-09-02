//! Reconstruction of the WOFF2 `glyf`/`loca` transform.
//!
//! Real-world WOFF2 files almost always transform `glyf`: it is where the
//! format's size advantage comes from. The transform replaces the table with
//! seven parallel substreams and a triplet coordinate encoding, so recovering a
//! font means rebuilding the outlines rather than decompressing bytes.
//!
//! The transform is lossless for *outlines*, not for bytes: the original
//! glyph's choice of flag repetition and coordinate widths is discarded by the
//! encoder, so a reconstructed `glyf` is byte-comparable only after
//! re-normalisation. Tests therefore compare glyph coordinates, not raw bytes.

use crate::error::{FontError, Result};
use crate::read::Reader;

const TAG_GLYF: [u8; 4] = *b"glyf";
const TRANSFORM_HEADER_LEN: usize = 36;

// Composite glyph component flags, from the OpenType `glyf` table.
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;

// Simple glyph point flags, from the OpenType `glyf` table.
const FLAG_ON_CURVE: u8 = 0x01;
const FLAG_X_SHORT: u8 = 0x02;
const FLAG_Y_SHORT: u8 = 0x04;
const FLAG_X_SAME_OR_POSITIVE: u8 = 0x10;
const FLAG_Y_SAME_OR_POSITIVE: u8 = 0x20;

/// The rebuilt tables.
pub struct Reconstructed {
    pub glyf: Vec<u8>,
    pub loca: Vec<u8>,
}

/// A decoded outline point in absolute font units.
#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
    on_curve: bool,
}

fn err(detail: &'static str) -> FontError {
    FontError::Decompress {
        tag: Some(TAG_GLYF),
        detail,
    }
}

/// WOFF2 `255UInt16`: small values in one byte, with two escape codes for the
/// next two ranges and a third for a full 16-bit value.
fn read_255_u16(r: &mut Reader<'_>) -> Result<u16> {
    const WORD_CODE: u8 = 253;
    const ONE_MORE_BYTE_CODE2: u8 = 254;
    const ONE_MORE_BYTE_CODE1: u8 = 255;
    const LOWEST_U_CODE: u16 = 253;

    let code = r.u8("255UInt16 code")?;
    match code {
        WORD_CODE => r.u16("255UInt16 word"),
        ONE_MORE_BYTE_CODE1 => Ok(u16::from(r.u8("255UInt16 byte")?) + LOWEST_U_CODE),
        ONE_MORE_BYTE_CODE2 => Ok(u16::from(r.u8("255UInt16 byte")?) + LOWEST_U_CODE * 2),
        other => Ok(u16::from(other)),
    }
}

/// The triplet encoding packs a point's flag byte and one to four data bytes
/// into a signed delta pair. The bracket boundaries are normative.
fn decode_triplet(flag: u8, data: &[u8]) -> (i32, i32) {
    let sign = |bit: u8, value: i32| if bit & 1 == 1 { value } else { -value };
    let b = |i: usize| data.get(i).copied().unwrap_or(0) as i32;

    if flag < 10 {
        (0, sign(flag, ((i32::from(flag) & 14) << 7) + b(0)))
    } else if flag < 20 {
        (sign(flag, (((i32::from(flag) - 10) & 14) << 7) + b(0)), 0)
    } else if flag < 84 {
        let b0 = i32::from(flag) - 20;
        let b1 = b(0);
        (
            sign(flag, 1 + (b0 & 0x30) + (b1 >> 4)),
            sign(flag >> 1, 1 + ((b0 & 0x0c) << 2) + (b1 & 0x0f)),
        )
    } else if flag < 120 {
        let b0 = i32::from(flag) - 84;
        (
            sign(flag, 1 + ((b0 / 12) << 8) + b(0)),
            sign(flag >> 1, 1 + (((b0 % 12) >> 2) << 8) + b(1)),
        )
    } else if flag < 124 {
        let b1 = b(1);
        (
            sign(flag, (b(0) << 4) + (b1 >> 4)),
            sign(flag >> 1, ((b1 & 0x0f) << 8) + b(2)),
        )
    } else {
        (
            sign(flag, (b(0) << 8) + b(1)),
            sign(flag >> 1, (b(2) << 8) + b(3)),
        )
    }
}

/// How many data bytes the triplet for this flag consumes.
fn triplet_byte_count(flag: u8) -> usize {
    match flag {
        0..=83 => 1,
        84..=119 => 2,
        120..=123 => 3,
        _ => 4,
    }
}

struct Substreams<'a> {
    n_contour: Reader<'a>,
    n_points: Reader<'a>,
    flags: Reader<'a>,
    glyph: Reader<'a>,
    composite: Reader<'a>,
    bbox_bitmap: &'a [u8],
    bbox_values: Reader<'a>,
    instruction: Reader<'a>,
}

/// Split the transformed table into its seven substreams, bounds-checked.
fn split<'a>(input: &'a [u8], num_glyphs: u16, sizes: [u32; 7]) -> Result<Substreams<'a>> {
    let mut offset = TRANSFORM_HEADER_LEN;
    let mut slice = |len: u32| -> Result<&'a [u8]> {
        let start = offset;
        let end = start
            .checked_add(len as usize)
            .filter(|e| *e <= input.len())
            .ok_or(FontError::Truncated {
                needed: len as usize,
                available: input.len().saturating_sub(start),
                while_reading: "glyf transform substream",
            })?;
        offset = end;
        input
            .get(start..end)
            .ok_or(err("substream slice out of range"))
    };

    let n_contour = slice(sizes[0])?;
    let n_points = slice(sizes[1])?;
    let flags = slice(sizes[2])?;
    let glyph = slice(sizes[3])?;
    let composite = slice(sizes[4])?;
    let bbox = slice(sizes[5])?;
    let instruction = slice(sizes[6])?;

    // The bbox substream opens with one presence bit per glyph, then the
    // explicit bounding boxes for the glyphs whose bit is set.
    let bitmap_len = (num_glyphs as usize).div_ceil(8);
    let bbox_bitmap = bbox
        .get(..bitmap_len)
        .ok_or(err("bbox bitmap is truncated"))?;
    let bbox_values = bbox
        .get(bitmap_len..)
        .ok_or(err("bbox values are truncated"))?;

    Ok(Substreams {
        n_contour: Reader::new(n_contour),
        n_points: Reader::new(n_points),
        flags: Reader::new(flags),
        glyph: Reader::new(glyph),
        composite: Reader::new(composite),
        bbox_bitmap,
        bbox_values: Reader::new(bbox_values),
        instruction: Reader::new(instruction),
    })
}

fn has_bbox(bitmap: &[u8], glyph_index: u16) -> bool {
    let byte = (glyph_index / 8) as usize;
    let bit = 7 - (glyph_index % 8);
    bitmap.get(byte).is_some_and(|b| (b >> bit) & 1 == 1)
}

/// Measure a composite glyph's component data, which is already in final form
/// and only needs copying.
fn composite_length(r: &mut Reader<'_>) -> Result<(usize, bool)> {
    let start = r.position();
    let mut have_instructions = false;
    loop {
        let flags = r.u16("composite component flags")?;
        r.skip(2, "composite glyph index")?;
        r.skip(
            if flags & ARG_1_AND_2_ARE_WORDS != 0 {
                4
            } else {
                2
            },
            "composite arguments",
        )?;
        if flags & WE_HAVE_A_SCALE != 0 {
            r.skip(2, "composite scale")?;
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            r.skip(4, "composite x/y scale")?;
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            r.skip(8, "composite 2x2 transform")?;
        }
        have_instructions |= flags & WE_HAVE_INSTRUCTIONS != 0;
        if flags & MORE_COMPONENTS == 0 {
            return Ok((r.position() - start, have_instructions));
        }
    }
}

/// Emit a simple glyph in standard `glyf` form. Flag repetition is not used;
/// the output is a valid, plainly-encoded glyph rather than a minimal one.
fn write_simple_glyph(
    out: &mut Vec<u8>,
    points: &[Point],
    ends: &[u16],
    instructions: &[u8],
    bbox: [i16; 4],
) {
    out.extend_from_slice(&(ends.len() as i16).to_be_bytes());
    for v in bbox {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for e in ends {
        out.extend_from_slice(&e.to_be_bytes());
    }
    out.extend_from_slice(&(instructions.len() as u16).to_be_bytes());
    out.extend_from_slice(instructions);

    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    let (mut prev_x, mut prev_y) = (0i32, 0i32);
    for p in points {
        let mut flag = if p.on_curve { FLAG_ON_CURVE } else { 0 };
        let dx = p.x - prev_x;
        let dy = p.y - prev_y;

        if dx == 0 {
            flag |= FLAG_X_SAME_OR_POSITIVE;
        } else if (-255..=255).contains(&dx) {
            flag |= FLAG_X_SHORT;
            if dx > 0 {
                flag |= FLAG_X_SAME_OR_POSITIVE;
            }
            xs.push((dx.unsigned_abs() as u8).to_be_bytes()[0]);
        } else {
            xs.extend_from_slice(&(dx as i16).to_be_bytes());
        }

        if dy == 0 {
            flag |= FLAG_Y_SAME_OR_POSITIVE;
        } else if (-255..=255).contains(&dy) {
            flag |= FLAG_Y_SHORT;
            if dy > 0 {
                flag |= FLAG_Y_SAME_OR_POSITIVE;
            }
            ys.push((dy.unsigned_abs() as u8).to_be_bytes()[0]);
        } else {
            ys.extend_from_slice(&(dy as i16).to_be_bytes());
        }

        out.push(flag);
        prev_x = p.x;
        prev_y = p.y;
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
}

/// Rebuild `glyf` and `loca` from a transformed `glyf` table.
pub fn reconstruct(input: &[u8]) -> Result<Reconstructed> {
    let mut header = Reader::new(input);
    let version = header.u32("glyf transform version")?;
    if version != 0 {
        return Err(FontError::UnsupportedTransform {
            tag: TAG_GLYF,
            version: version as u8,
        });
    }
    let num_glyphs = header.u16("glyf transform numGlyphs")?;
    let index_format = header.u16("glyf transform indexFormat")?;
    let mut sizes = [0u32; 7];
    for size in &mut sizes {
        *size = header.u32("glyf transform substream size")?;
    }
    let mut s = split(input, num_glyphs, sizes)?;

    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(num_glyphs as usize + 1);
    offsets.push(0u32);

    for glyph_index in 0..num_glyphs {
        let n_contours = s.n_contour.u16("glyf contour count")? as i16;

        if n_contours == 0 {
            // An empty glyph occupies no bytes; loca simply repeats the offset.
            offsets.push(glyf.len() as u32);
            continue;
        }

        if n_contours < 0 {
            // Composite: component data is already final, only instructions move.
            let data_start = s.composite.position();
            let (len, have_instructions) = composite_length(&mut s.composite)?;
            if !has_bbox(s.bbox_bitmap, glyph_index) {
                return Err(err("composite glyph is missing its required bounding box"));
            }
            let bbox = [
                s.bbox_values.u16("composite bbox")? as i16,
                s.bbox_values.u16("composite bbox")? as i16,
                s.bbox_values.u16("composite bbox")? as i16,
                s.bbox_values.u16("composite bbox")? as i16,
            ];
            glyf.extend_from_slice(&(-1i16).to_be_bytes());
            for v in bbox {
                glyf.extend_from_slice(&v.to_be_bytes());
            }
            let component_bytes = s
                .composite
                .slice_at(data_start, len)
                .ok_or(err("composite component data out of range"))?;
            glyf.extend_from_slice(component_bytes);

            if have_instructions {
                let instruction_len = read_255_u16(&mut s.glyph)? as usize;
                let instructions = s
                    .instruction
                    .take_bytes(instruction_len, "composite instructions")?;
                glyf.extend_from_slice(&(instruction_len as u16).to_be_bytes());
                glyf.extend_from_slice(instructions);
            }
            glyf.resize((glyf.len() + 3) & !3, 0);
            offsets.push(glyf.len() as u32);
            continue;
        }

        // Simple glyph: points per contour, then one flag and one triplet each.
        let contour_count = n_contours as usize;
        let mut ends = Vec::with_capacity(contour_count);
        let mut total_points = 0usize;
        for _ in 0..contour_count {
            let n = read_255_u16(&mut s.n_points)? as usize;
            total_points = total_points
                .checked_add(n)
                .ok_or(err("point count overflow"))?;
            if total_points > u16::MAX as usize {
                return Err(err(
                    "glyph declares more points than a glyf table can address",
                ));
            }
            ends.push((total_points as u16).wrapping_sub(1));
        }

        let mut points = Vec::with_capacity(total_points);
        let (mut x, mut y) = (0i32, 0i32);
        for _ in 0..total_points {
            let flag_byte = s.flags.u8("glyf point flag")?;
            let on_curve = flag_byte & 0x80 == 0;
            let flag = flag_byte & 0x7f;
            let data = s
                .glyph
                .take_bytes(triplet_byte_count(flag), "glyf triplet")?;
            let (dx, dy) = decode_triplet(flag, data);
            x += dx;
            y += dy;
            points.push(Point { x, y, on_curve });
        }

        let instruction_len = read_255_u16(&mut s.glyph)? as usize;
        let instructions = s
            .instruction
            .take_bytes(instruction_len, "glyf instructions")?
            .to_vec();

        // An explicit bbox overrides the computed one; otherwise derive it.
        let bbox = if has_bbox(s.bbox_bitmap, glyph_index) {
            [
                s.bbox_values.u16("glyph bbox")? as i16,
                s.bbox_values.u16("glyph bbox")? as i16,
                s.bbox_values.u16("glyph bbox")? as i16,
                s.bbox_values.u16("glyph bbox")? as i16,
            ]
        } else {
            let min_x = points.iter().map(|p| p.x).min().unwrap_or(0);
            let min_y = points.iter().map(|p| p.y).min().unwrap_or(0);
            let max_x = points.iter().map(|p| p.x).max().unwrap_or(0);
            let max_y = points.iter().map(|p| p.y).max().unwrap_or(0);
            [min_x as i16, min_y as i16, max_x as i16, max_y as i16]
        };

        write_simple_glyph(&mut glyf, &points, &ends, &instructions, bbox);
        glyf.resize((glyf.len() + 3) & !3, 0);
        offsets.push(glyf.len() as u32);
    }

    // `loca` stores the offsets in the width the transform header selected.
    let mut loca = Vec::new();
    if index_format == 0 {
        for offset in &offsets {
            if offset % 2 != 0 || offset / 2 > u32::from(u16::MAX) {
                return Err(err(
                    "glyph offset cannot be expressed in the short loca format",
                ));
            }
            loca.extend_from_slice(&((offset / 2) as u16).to_be_bytes());
        }
    } else {
        for offset in &offsets {
            loca.extend_from_slice(&offset.to_be_bytes());
        }
    }
    Ok(Reconstructed { glyf, loca })
}

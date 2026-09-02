//! WOFF 2.0: one brotli stream covering every table, plus a compact directory.
//!
//! The specification defines an optional `glyf`/`loca` transform. Transform
//! version 3 is the null transform and is fully conformant, so the encoder here
//! emits untransformed tables and leans on brotli for the size win. The decoder
//! reads null-transformed files; a transformed `glyf` is reported as
//! `UnsupportedTransform` rather than silently producing a corrupt font.

use crate::base128;
use crate::error::{FontError, Result};
use crate::read::Reader;
use crate::sfnt::{align4, Font, Table};
use crate::tags::{index_for_tag, tag_for_index, ARBITRARY_TAG};
use std::io::{Read, Write};

pub const SIGNATURE: [u8; 4] = *b"wOF2";
const HEADER_LEN: usize = 48;
const MAX_DECOMPRESSED_BYTES: usize = 512 * 1024 * 1024;
const BROTLI_QUALITY: u32 = 11;
const BROTLI_WINDOW_BITS: u32 = 22;
const BROTLI_BUFFER: usize = 4096;

/// Null transform for `glyf`/`loca`; any other value there means transformed data.
const TRANSFORM_NULL_GLYF: u8 = 3;
/// Null transform for every other table.
const TRANSFORM_NULL_DEFAULT: u8 = 0;

const TAG_GLYF: [u8; 4] = *b"glyf";
const TAG_LOCA: [u8; 4] = *b"loca";

/// Offset of `head.flags`, and the bit meaning "this font data is the lossless
/// result of an optimising transformation". WOFF2 producers are required to set
/// it, so the encoder does.
const HEAD_FLAGS_OFFSET: usize = 16;
const HEAD_FLAG_LOSSLESS: u16 = 1 << 11;

pub fn is_woff2(input: &[u8]) -> bool {
    input.len() >= 4 && input[..4] == SIGNATURE
}

/// Which transform value means "no transform" for a given tag.
fn null_transform_for(tag: &[u8; 4]) -> u8 {
    if tag == &TAG_GLYF || tag == &TAG_LOCA {
        TRANSFORM_NULL_GLYF
    } else {
        TRANSFORM_NULL_DEFAULT
    }
}

struct DirectoryEntry {
    tag: [u8; 4],
    transform_version: u8,
    orig_length: u32,
    /// Length of this table inside the decompressed stream, which differs from
    /// `orig_length` whenever a transform is applied.
    stream_length: u32,
    transformed: bool,
}

pub fn decode(input: &[u8]) -> Result<Font> {
    let mut r = Reader::new(input);
    let magic = r.tag("WOFF2 signature")?;
    if magic != SIGNATURE {
        return Err(FontError::UnknownContainer { magic });
    }
    let flavor = r.u32("WOFF2 flavor")?;
    Font::check_flavor(flavor)?;
    r.skip(4, "WOFF2 length")?;
    let num_tables = r.u16("WOFF2 table count")?;
    if num_tables == 0 {
        return Err(FontError::BadTableCount { count: num_tables });
    }
    r.skip(2, "WOFF2 reserved")?;
    r.skip(4, "WOFF2 totalSfntSize")?;
    let total_compressed_size = r.u32("WOFF2 totalCompressedSize")?;
    r.skip(4, "WOFF2 version")?;
    r.skip(12 + 8, "WOFF2 metadata and private blocks")?;

    let mut entries = Vec::with_capacity(num_tables as usize);
    for _ in 0..num_tables {
        let flags = r.u8("WOFF2 table flags")?;
        let tag_index = flags & 0x3f;
        let transform_version = (flags >> 6) & 0x03;
        let tag = if tag_index == ARBITRARY_TAG {
            r.tag("WOFF2 arbitrary table tag")?
        } else {
            tag_for_index(tag_index).ok_or(FontError::BadBase128 {
                detail: "table tag index out of range",
            })?
        };
        let orig_length = base128::read(&mut r)?;

        // transformLength is present only when the table is actually transformed.
        let transformed = transform_version != null_transform_for(&tag);
        let stream_length = if transformed {
            base128::read(&mut r)?
        } else {
            orig_length
        };

        // The glyf/loca transform is reconstructed below; nothing else is defined.
        if transformed && tag != TAG_GLYF && tag != TAG_LOCA {
            return Err(FontError::UnsupportedTransform {
                tag,
                version: transform_version,
            });
        }
        entries.push(DirectoryEntry {
            tag,
            transform_version,
            orig_length,
            stream_length,
            transformed,
        });
    }

    // The compressed block runs from the end of the directory to the declared length.
    let data_start = r.position();
    let data_end = data_start
        .checked_add(total_compressed_size as usize)
        .filter(|end| *end <= input.len())
        .ok_or(FontError::Truncated {
            needed: total_compressed_size as usize,
            available: input.len().saturating_sub(data_start),
            while_reading: "WOFF2 compressed font data",
        })?;

    let expected: usize = entries.iter().map(|e| e.stream_length as usize).sum();
    if expected > MAX_DECOMPRESSED_BYTES {
        return Err(FontError::Decompress {
            tag: None,
            detail: "declared table sizes exceed the decode limit",
        });
    }
    let block = brotli_decompress(&input[data_start..data_end], expected)?;
    if block.len() < expected {
        return Err(FontError::Decompress {
            tag: None,
            detail: "decompressed stream is shorter than the directory declares",
        });
    }

    let mut tables = Vec::with_capacity(entries.len());
    let mut transformed_glyf: Option<Vec<u8>> = None;
    let mut cursor = 0usize;
    for e in &entries {
        let len = e.stream_length as usize;
        let slice = block
            .get(cursor..cursor + len)
            .ok_or(FontError::Decompress {
                tag: Some(e.tag),
                detail: "table extends past the decompressed stream",
            })?;
        cursor += len;

        if e.transformed {
            // A transformed loca carries no payload; it is regenerated together
            // with glyf, so both are filled in once reconstruction has run.
            if e.tag == TAG_GLYF {
                transformed_glyf = Some(slice.to_vec());
            } else if e.stream_length != 0 {
                return Err(FontError::UnsupportedTransform {
                    tag: e.tag,
                    version: e.transform_version,
                });
            }
            tables.push(Table {
                tag: e.tag,
                data: Vec::new(),
            });
            continue;
        }

        if slice.len() != e.orig_length as usize {
            return Err(FontError::LengthMismatch {
                tag: e.tag,
                declared: e.orig_length,
                actual: slice.len(),
            });
        }
        tables.push(Table {
            tag: e.tag,
            data: slice.to_vec(),
        });
    }

    if let Some(payload) = transformed_glyf {
        let rebuilt = crate::glyf_transform::reconstruct(&payload)?;
        for t in tables.iter_mut() {
            if t.tag == TAG_GLYF {
                t.data = rebuilt.glyf.clone();
            } else if t.tag == TAG_LOCA {
                t.data = rebuilt.loca.clone();
            }
        }
    }

    Font::reject_duplicate_tags(&tables)?;
    Ok(Font { flavor, tables })
}

/// Set `head.flags` bit 11 in place, as WOFF2 requires of its producers.
fn mark_lossless_transform(tables: &mut [Table]) {
    let Some(head) = tables.iter_mut().find(|t| t.tag == crate::sfnt::TAG_HEAD) else {
        return;
    };
    let Some(field) = head.data.get_mut(HEAD_FLAGS_OFFSET..HEAD_FLAGS_OFFSET + 2) else {
        return;
    };
    let flags = u16::from_be_bytes([field[0], field[1]]) | HEAD_FLAG_LOSSLESS;
    field.copy_from_slice(&flags.to_be_bytes());
}

pub fn encode(font: &Font) -> Result<Vec<u8>> {
    let mut tables = font.tables.clone();
    tables.sort_by_key(|t| t.tag);
    mark_lossless_transform(&mut tables);
    let n = tables.len();

    let mut directory = Vec::new();
    let mut block = Vec::new();
    for t in &tables {
        let transform_version = null_transform_for(&t.tag);
        let flags = match index_for_tag(&t.tag) {
            Some(index) => (transform_version << 6) | index,
            None => (transform_version << 6) | ARBITRARY_TAG,
        };
        directory.push(flags);
        if index_for_tag(&t.tag).is_none() {
            directory.extend_from_slice(&t.tag);
        }
        base128::write(&mut directory, t.data.len() as u32);
        // Null transform, so transformLength is omitted by definition.
        block.extend_from_slice(&t.data);
    }

    let compressed = brotli_compress(&block)?;
    let total_sfnt_size = 12 + 16 * n + tables.iter().map(|t| align4(t.data.len())).sum::<usize>();
    let total_length = HEADER_LEN + directory.len() + compressed.len();

    let mut out = Vec::with_capacity(total_length);
    out.extend_from_slice(&SIGNATURE);
    out.extend_from_slice(&font.flavor.to_be_bytes());
    out.extend_from_slice(&(total_length as u32).to_be_bytes());
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // reserved
    out.extend_from_slice(&(total_sfnt_size as u32).to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
    out.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
    out.extend_from_slice(&[0u8; 12]); // no extended metadata
    out.extend_from_slice(&[0u8; 8]); // no private data
    out.extend_from_slice(&directory);
    out.extend_from_slice(&compressed);
    Ok(out)
}

fn brotli_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(
            &mut out,
            BROTLI_BUFFER,
            BROTLI_QUALITY,
            BROTLI_WINDOW_BITS,
        );
        writer.write_all(data).map_err(|_| FontError::Decompress {
            tag: None,
            detail: "brotli encoder rejected the input",
        })?;
    }
    Ok(out)
}

fn brotli_decompress(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    // Allow one byte beyond the expectation so an oversized stream is detected
    // rather than silently truncated.
    let limit = expected.saturating_add(1).min(MAX_DECOMPRESSED_BYTES);
    let mut out = Vec::with_capacity(expected.min(1 << 20));
    brotli::Decompressor::new(data, BROTLI_BUFFER)
        .take(limit as u64)
        .read_to_end(&mut out)
        .map_err(|_| FontError::Decompress {
            tag: None,
            detail: "brotli stream is invalid",
        })?;
    Ok(out)
}

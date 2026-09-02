//! WOFF 1.0: an sfnt whose tables are individually zlib-compressed.

use crate::error::{FontError, Result};
use crate::read::Reader;
use crate::sfnt::{align4, table_checksum, Font, Table};
use miniz_oxide::deflate::compress_to_vec_zlib;
use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

pub const SIGNATURE: [u8; 4] = *b"wOFF";
const HEADER_LEN: usize = 44;
const ENTRY_LEN: usize = 20;
/// Ceiling on any single decompressed table, so a hostile file cannot ask us
/// to allocate unbounded memory.
const MAX_TABLE_BYTES: usize = 256 * 1024 * 1024;
const ZLIB_LEVEL: u8 = 9;

pub fn is_woff1(input: &[u8]) -> bool {
    input.len() >= 4 && input[..4] == SIGNATURE
}

pub fn decode(input: &[u8]) -> Result<Font> {
    let mut r = Reader::new(input);
    let magic = r.tag("WOFF signature")?;
    if magic != SIGNATURE {
        return Err(FontError::UnknownContainer { magic });
    }
    let flavor = r.u32("WOFF flavor")?;
    Font::check_flavor(flavor)?;
    r.skip(4, "WOFF length")?;
    let num_tables = r.u16("WOFF table count")?;
    if num_tables == 0 {
        return Err(FontError::BadTableCount { count: num_tables });
    }
    // reserved + totalSfntSize + version + metadata block + private block
    r.skip(2 + 4 + 4 + 12 + 8, "WOFF header remainder")?;

    let mut tables = Vec::with_capacity(num_tables as usize);
    for _ in 0..num_tables {
        let tag = r.tag("WOFF directory entry")?;
        let offset = r.u32("WOFF table offset")?;
        let comp_length = r.u32("WOFF compressed length")?;
        let orig_length = r.u32("WOFF original length")?;
        let _orig_checksum = r.u32("WOFF table checksum")?;

        let start = offset as usize;
        let end = start
            .checked_add(comp_length as usize)
            .ok_or(FontError::TableOutOfBounds {
                tag,
                offset,
                length: comp_length,
                file_len: input.len(),
            })?;
        if end > input.len() {
            return Err(FontError::TableOutOfBounds {
                tag,
                offset,
                length: comp_length,
                file_len: input.len(),
            });
        }
        if orig_length as usize > MAX_TABLE_BYTES {
            return Err(FontError::LengthMismatch {
                tag,
                declared: orig_length,
                actual: MAX_TABLE_BYTES,
            });
        }
        let raw = &input[start..end];

        // Equal lengths mean the encoder found compression unprofitable and stored the table verbatim.
        let data = if comp_length == orig_length {
            raw.to_vec()
        } else {
            decompress_to_vec_zlib_with_limit(raw, MAX_TABLE_BYTES).map_err(|_| {
                FontError::Decompress {
                    tag: Some(tag),
                    detail: "zlib stream is invalid",
                }
            })?
        };
        if data.len() != orig_length as usize {
            return Err(FontError::LengthMismatch {
                tag,
                declared: orig_length,
                actual: data.len(),
            });
        }
        tables.push(Table { tag, data });
    }
    Font::reject_duplicate_tags(&tables)?;
    Ok(Font { flavor, tables })
}

pub fn encode(font: &Font) -> Vec<u8> {
    let mut tables = font.tables.clone();
    tables.sort_by_key(|t| t.tag);
    let n = tables.len();

    // Compress first: the header needs the final file length, which depends on
    // whether each table compressed profitably.
    let payloads: Vec<Vec<u8>> = tables
        .iter()
        .map(|t| {
            let squeezed = compress_to_vec_zlib(&t.data, ZLIB_LEVEL);
            // The spec requires storing the table raw when compression does not help.
            if squeezed.len() < t.data.len() {
                squeezed
            } else {
                t.data.clone()
            }
        })
        .collect();

    let mut offset = HEADER_LEN + ENTRY_LEN * n;
    let mut entries = Vec::with_capacity(n);
    for (t, payload) in tables.iter().zip(&payloads) {
        entries.push((
            t.tag,
            offset as u32,
            payload.len() as u32,
            t.data.len() as u32,
            table_checksum(&t.tag, &t.data),
        ));
        offset += align4(payload.len());
    }
    let total_length = offset;

    let mut out = Vec::with_capacity(total_length);
    out.extend_from_slice(&SIGNATURE);
    out.extend_from_slice(&font.flavor.to_be_bytes());
    out.extend_from_slice(&(total_length as u32).to_be_bytes());
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // reserved
    out.extend_from_slice(&(font.sfnt_size() as u32).to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
    out.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
    out.extend_from_slice(&[0u8; 12]); // no extended metadata
    out.extend_from_slice(&[0u8; 8]); // no private data

    for (tag, off, comp_len, orig_len, checksum) in &entries {
        out.extend_from_slice(tag);
        out.extend_from_slice(&off.to_be_bytes());
        out.extend_from_slice(&comp_len.to_be_bytes());
        out.extend_from_slice(&orig_len.to_be_bytes());
        out.extend_from_slice(&checksum.to_be_bytes());
    }
    for payload in &payloads {
        out.extend_from_slice(payload);
        out.resize(align4(out.len()), 0);
    }
    out
}

//! The plain `sfnt` container shared by TTF and OTF, plus the checksum rules
//! that every other container in this crate has to reproduce.

use crate::error::{FontError, Result};
use crate::read::Reader;

pub const FLAVOR_TRUETYPE: u32 = 0x0001_0000;
pub const FLAVOR_CFF: u32 = 0x4F54_544F; // 'OTTO'
pub const FLAVOR_TRUE: u32 = 0x7472_7565; // 'true'
const MAGIC_TTCF: u32 = 0x7474_6366; // 'ttcf'

/// `head.checkSumAdjustment` is defined as this constant minus the whole-font checksum.
const CHECKSUM_MAGIC: u32 = 0xB1B0_AFBA;
pub const TAG_HEAD: [u8; 4] = *b"head";
const HEAD_ADJUSTMENT_OFFSET: usize = 8;

/// One table, held decompressed. `data` is the logical table content with no
/// trailing container padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub tag: [u8; 4],
    pub data: Vec<u8>,
}

/// A single font: an outline flavor plus its tables. Every container in this
/// crate decodes into, and encodes from, this one type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    pub flavor: u32,
    pub tables: Vec<Table>,
}

/// Round `n` up to the next multiple of four.
#[inline]
pub fn align4(n: usize) -> usize {
    n.wrapping_add(3) & !3
}

/// The sfnt checksum: big-endian u32 words, zero-padded to a 4-byte boundary,
/// summed with wrapping arithmetic.
pub fn checksum(data: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        // `chunks_exact` guarantees the width, so this conversion cannot fail.
        let word = <[u8; 4]>::try_from(c).unwrap_or([0; 4]);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut word = [0u8; 4];
        if let Some(head) = word.get_mut(..rest.len()) {
            head.copy_from_slice(rest);
        }
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

/// The checksum a table contributes to the font. For `head` the mutable
/// `checkSumAdjustment` field reads as zero, otherwise the value would depend
/// on itself.
pub fn table_checksum(tag: &[u8; 4], data: &[u8]) -> u32 {
    if tag != &TAG_HEAD {
        return checksum(data);
    }
    let mut patched = data.to_vec();
    if let Some(field) = patched.get_mut(HEAD_ADJUSTMENT_OFFSET..HEAD_ADJUSTMENT_OFFSET + 4) {
        field.fill(0);
    }
    checksum(&patched)
}

impl Font {
    /// Validate a flavor read from any container.
    pub fn check_flavor(flavor: u32) -> Result<()> {
        match flavor {
            FLAVOR_TRUETYPE | FLAVOR_CFF | FLAVOR_TRUE => Ok(()),
            MAGIC_TTCF => Err(FontError::FontCollection),
            other => Err(FontError::UnknownFlavor { flavor: other }),
        }
    }

    pub fn table(&self, tag: &[u8; 4]) -> Option<&Table> {
        self.tables.iter().find(|t| &t.tag == tag)
    }

    /// Reject a directory that names the same table twice; downstream code
    /// assumes tags are unique.
    pub fn reject_duplicate_tags(tables: &[Table]) -> Result<()> {
        for (i, t) in tables.iter().enumerate() {
            if tables[..i].iter().any(|p| p.tag == t.tag) {
                return Err(FontError::DuplicateTable { tag: t.tag });
            }
        }
        Ok(())
    }

    /// Size this font occupies once written as sfnt, which WOFF headers must declare.
    pub fn sfnt_size(&self) -> usize {
        12 + 16 * self.tables.len()
            + self
                .tables
                .iter()
                .map(|t| align4(t.data.len()))
                .sum::<usize>()
    }

    /// Parse a bare TTF/OTF.
    pub fn parse_sfnt(input: &[u8]) -> Result<Self> {
        let mut r = Reader::new(input);
        let flavor = r.u32("sfnt version")?;
        Self::check_flavor(flavor)?;
        let num_tables = r.u16("table count")?;
        if num_tables == 0 {
            return Err(FontError::BadTableCount { count: num_tables });
        }
        r.skip(6, "sfnt search parameters")?;

        let mut tables = Vec::with_capacity(num_tables as usize);
        for _ in 0..num_tables {
            let tag = r.tag("table directory entry")?;
            let _checksum = r.u32("table checksum")?;
            let offset = r.u32("table offset")?;
            let length = r.u32("table length")?;
            let start = offset as usize;
            let end = start
                .checked_add(length as usize)
                .ok_or(FontError::TableOutOfBounds {
                    tag,
                    offset,
                    length,
                    file_len: input.len(),
                })?;
            if end > input.len() {
                return Err(FontError::TableOutOfBounds {
                    tag,
                    offset,
                    length,
                    file_len: input.len(),
                });
            }
            tables.push(Table {
                tag,
                data: input[start..end].to_vec(),
            });
        }
        Self::reject_duplicate_tags(&tables)?;
        Ok(Font { flavor, tables })
    }

    /// Serialise to a bare TTF/OTF, recomputing every checksum.
    pub fn to_sfnt(&self) -> Vec<u8> {
        let mut tables = self.tables.clone();
        // The directory must be sorted by tag; table data order is free but we
        // keep it aligned with the directory for a deterministic byte output.
        tables.sort_by_key(|t| t.tag);

        let n = tables.len();
        let entry_selector = if n == 0 {
            0
        } else {
            (usize::BITS - 1 - n.leading_zeros()) as u16
        };
        let search_range = (1u32 << entry_selector) * 16;
        let range_shift = (n as u32 * 16).saturating_sub(search_range);

        let mut out = Vec::with_capacity(self.sfnt_size());
        out.extend_from_slice(&self.flavor.to_be_bytes());
        out.extend_from_slice(&(n as u16).to_be_bytes());
        out.extend_from_slice(&(search_range as u16).to_be_bytes());
        out.extend_from_slice(&entry_selector.to_be_bytes());
        out.extend_from_slice(&(range_shift as u16).to_be_bytes());

        let mut offset = 12 + 16 * n;
        let mut head_data_offset = None;
        for t in &tables {
            out.extend_from_slice(&t.tag);
            out.extend_from_slice(&table_checksum(&t.tag, &t.data).to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(t.data.len() as u32).to_be_bytes());
            if t.tag == TAG_HEAD {
                head_data_offset = Some(offset);
            }
            offset += align4(t.data.len());
        }
        for t in &tables {
            out.extend_from_slice(&t.data);
            out.resize(align4(out.len()), 0);
        }

        // checkSumAdjustment is defined over the finished file, so it is patched last.
        if let Some(head_at) = head_data_offset {
            let field = head_at + HEAD_ADJUSTMENT_OFFSET;
            if let Some(slot) = out.get_mut(field..field + 4) {
                slot.fill(0);
            }
            let adjustment = CHECKSUM_MAGIC.wrapping_sub(checksum(&out));
            if let Some(slot) = out.get_mut(field..field + 4) {
                slot.copy_from_slice(&adjustment.to_be_bytes());
            }
        }
        out
    }
}

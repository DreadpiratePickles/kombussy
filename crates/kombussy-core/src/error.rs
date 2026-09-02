use std::fmt;

/// Every failure mode this codec can produce. No stringly-typed errors, no panics
/// on malformed input: all parse paths return `Err` with the offending context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// Input ended before a structure could be read.
    Truncated {
        needed: usize,
        available: usize,
        while_reading: &'static str,
    },
    /// The container magic did not match any format we support.
    UnknownContainer { magic: [u8; 4] },
    /// A TrueType Collection was supplied; v0.1 handles single fonts only.
    FontCollection,
    /// `sfntVersion` / `flavor` is not a recognised outline format.
    UnknownFlavor { flavor: u32 },
    /// Table count of zero, or beyond what the container can address.
    BadTableCount { count: u16 },
    /// A directory entry points outside the file.
    TableOutOfBounds {
        tag: [u8; 4],
        offset: u32,
        length: u32,
        file_len: usize,
    },
    /// zlib/brotli stream did not decode.
    Decompress {
        tag: Option<[u8; 4]>,
        detail: &'static str,
    },
    /// Decompressed size disagreed with the size declared in the directory.
    LengthMismatch {
        tag: [u8; 4],
        declared: u32,
        actual: usize,
    },
    /// A UIntBase128 value was non-minimal, overlong, or overflowed u32.
    BadBase128 { detail: &'static str },
    /// WOFF2 table transform we do not implement yet.
    UnsupportedTransform { tag: [u8; 4], version: u8 },
    /// Two directory entries claimed the same tag.
    DuplicateTable { tag: [u8; 4] },
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn tag(t: &[u8; 4]) -> String {
            t.iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect()
        }
        match self {
            Self::Truncated { needed, available, while_reading } => write!(
                f, "truncated input while reading {while_reading}: needed {needed} bytes, {available} available"),
            Self::UnknownContainer { magic } => write!(f, "unrecognised container magic {:?} ({})", magic, tag(magic)),
            Self::FontCollection => write!(f, "TrueType Collection (ttcf) is not supported; extract a single font first"),
            Self::UnknownFlavor { flavor } => write!(f, "unrecognised outline flavor 0x{flavor:08x}"),
            Self::BadTableCount { count } => write!(f, "invalid table count {count}"),
            Self::TableOutOfBounds { tag: t, offset, length, file_len } => write!(
                f, "table '{}' at offset {offset} length {length} exceeds file length {file_len}", tag(t)),
            Self::Decompress { tag: t, detail } => match t {
                Some(t) => write!(f, "decompression failed for table '{}': {detail}", tag(t)),
                None => write!(f, "decompression failed: {detail}"),
            },
            Self::LengthMismatch { tag: t, declared, actual } => write!(
                f, "table '{}' declared {declared} bytes but produced {actual}", tag(t)),
            Self::BadBase128 { detail } => write!(f, "malformed UIntBase128: {detail}"),
            Self::UnsupportedTransform { tag: t, version } => write!(
                f, "table '{}' uses transform version {version}, which is not implemented", tag(t)),
            Self::DuplicateTable { tag: t } => write!(f, "duplicate table tag '{}'", tag(t)),
        }
    }
}

impl std::error::Error for FontError {}

pub type Result<T> = std::result::Result<T, FontError>;

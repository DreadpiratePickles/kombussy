//! Kombussy — a font container codec for OpenType, WOFF and WOFF2.
//!
//! Every supported container decodes into [`Font`], and every container is
//! written back out from that same type, so conversion is always
//! `decode -> Font -> encode` with no format-specific special cases.
//!
//! ```no_run
//! use kombussy_core::{convert, Format};
//! # fn main() -> Result<(), kombussy_core::FontError> {
//! # let ttf_bytes: Vec<u8> = Vec::new();
//! let woff2 = convert(&ttf_bytes, Format::Woff2)?;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]
// Panics are the failure mode that matters for a parser fed hostile input, so
// the library denies the constructs that cause them. `indexing_slicing` stays a
// warning rather than a denial: correctness here comes from `Reader`, which
// bounds-checks every read, and the remaining index sites are constant offsets
// into fixed-size arrays that the type system already proves in range.
#![deny(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

mod base128;
pub(crate) mod glyf_transform;
mod read;
mod tags;

pub mod error;
pub mod sfnt;
pub mod woff1;
pub mod woff2;

pub use error::{FontError, Result};
pub use sfnt::{Font, Table};

/// The container formats this crate can read and write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Bare `sfnt`: `.ttf` or `.otf`, depending on the outline flavor.
    Sfnt,
    Woff1,
    Woff2,
}

impl Format {
    /// The file extension a converted font should be given. OTF and TTF share a
    /// container, so the outline flavor decides between them.
    pub fn extension(self, flavor: u32) -> &'static str {
        match self {
            Self::Sfnt if flavor == sfnt::FLAVOR_CFF => "otf",
            Self::Sfnt => "ttf",
            Self::Woff1 => "woff",
            Self::Woff2 => "woff2",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Sfnt => "font/otf",
            Self::Woff1 => "font/woff",
            Self::Woff2 => "font/woff2",
        }
    }
}

/// Identify a container from its leading magic bytes.
pub fn detect(input: &[u8]) -> Result<Format> {
    if woff1::is_woff1(input) {
        return Ok(Format::Woff1);
    }
    if woff2::is_woff2(input) {
        return Ok(Format::Woff2);
    }
    let magic = input.get(..4).ok_or(FontError::Truncated {
        needed: 4,
        available: input.len(),
        while_reading: "container magic",
    })?;
    let magic: [u8; 4] = [magic[0], magic[1], magic[2], magic[3]];
    match u32::from_be_bytes(magic) {
        sfnt::FLAVOR_TRUETYPE | sfnt::FLAVOR_CFF | sfnt::FLAVOR_TRUE => Ok(Format::Sfnt),
        _ => Err(FontError::UnknownContainer { magic }),
    }
}

/// Decode any supported container into the shared [`Font`] model.
pub fn decode(input: &[u8]) -> Result<Font> {
    match detect(input)? {
        Format::Sfnt => Font::parse_sfnt(input),
        Format::Woff1 => woff1::decode(input),
        Format::Woff2 => woff2::decode(input),
    }
}

/// Write a [`Font`] out in the requested container.
pub fn encode(font: &Font, target: Format) -> Result<Vec<u8>> {
    match target {
        Format::Sfnt => Ok(font.to_sfnt()),
        Format::Woff1 => Ok(woff1::encode(font)),
        Format::Woff2 => woff2::encode(font),
    }
}

/// Convert between any two supported containers in one call.
pub fn convert(input: &[u8], target: Format) -> Result<Vec<u8>> {
    encode(&decode(input)?, target)
}

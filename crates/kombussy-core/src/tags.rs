//! The WOFF2 "known table tags" list. A directory entry may reference a tag by
//! its five-bit index here instead of spelling out four bytes, so the order of
//! this table is normative and must not be rearranged.

pub const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern",
    b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty",
    b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

/// Index reserved for "an arbitrary four-byte tag follows this flags byte".
pub const ARBITRARY_TAG: u8 = 63;

pub fn tag_for_index(index: u8) -> Option<[u8; 4]> {
    KNOWN_TAGS.get(index as usize).map(|t| **t)
}

pub fn index_for_tag(tag: &[u8; 4]) -> Option<u8> {
    KNOWN_TAGS.iter().position(|k| *k == tag).map(|i| i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tag_list_has_63_entries() {
        assert_eq!(KNOWN_TAGS.len(), ARBITRARY_TAG as usize);
    }

    #[test]
    fn index_and_tag_are_inverse() {
        for (i, tag) in KNOWN_TAGS.iter().enumerate() {
            assert_eq!(index_for_tag(tag), Some(i as u8));
            assert_eq!(tag_for_index(i as u8).as_ref(), Some(*tag));
        }
    }

    #[test]
    fn spot_check_normative_indices() {
        // These four anchor the list; a shifted table would corrupt every file.
        assert_eq!(index_for_tag(b"cmap"), Some(0));
        assert_eq!(index_for_tag(b"glyf"), Some(10));
        assert_eq!(index_for_tag(b"loca"), Some(11));
        assert_eq!(index_for_tag(b"Sill"), Some(62));
    }

    #[test]
    fn unknown_tag_has_no_index() {
        assert_eq!(index_for_tag(b"ZZZZ"), None);
    }
}

//! A bounds-checked cursor. Every multi-byte read names what it was reading so
//! a truncated file produces an actionable error instead of a panic.

use crate::error::{FontError, Result};

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize, while_reading: &'static str) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(FontError::Truncated {
            needed: n,
            available: self.remaining(),
            while_reading,
        })?;
        if end > self.data.len() {
            return Err(FontError::Truncated {
                needed: n,
                available: self.remaining(),
                while_reading,
            });
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Read exactly `N` bytes into an array. `take` has already proved the
    /// length, so no indexing or fallible conversion is needed downstream.
    fn array<const N: usize>(&mut self, ctx: &'static str) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N, ctx)?);
        Ok(out)
    }

    pub fn u8(&mut self, ctx: &'static str) -> Result<u8> {
        let [b] = self.array::<1>(ctx)?;
        Ok(b)
    }

    pub fn u16(&mut self, ctx: &'static str) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array(ctx)?))
    }

    pub fn u32(&mut self, ctx: &'static str) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array(ctx)?))
    }

    pub fn tag(&mut self, ctx: &'static str) -> Result<[u8; 4]> {
        self.array(ctx)
    }

    /// Consume and return `n` bytes.
    pub fn take_bytes(&mut self, n: usize, ctx: &'static str) -> Result<&'a [u8]> {
        self.take(n, ctx)
    }

    /// Borrow an already-consumed range without moving the cursor.
    pub fn slice_at(&self, start: usize, len: usize) -> Option<&'a [u8]> {
        self.data.get(start..start.checked_add(len)?)
    }

    pub fn skip(&mut self, n: usize, ctx: &'static str) -> Result<()> {
        self.take(n, ctx).map(|_| ())
    }
}

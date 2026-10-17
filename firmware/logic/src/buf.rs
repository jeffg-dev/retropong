//! A tiny fixed-capacity byte buffer that implements [`core::fmt::Write`].
//!
//! Both the firmware binary and this crate's own tests need somewhere to
//! `write!`/`writeln!` a response line without an allocator. `FixedBuf<N>` is
//! that somewhere: a `[u8; N]` plus a length, with `write_str` failing
//! (returning [`core::fmt::Error`]) instead of panicking or silently
//! truncating when the buffer is too small.

/// A `[u8; N]`-backed buffer usable as a `core::fmt::Write` sink.
#[derive(Clone, Copy)]
pub struct FixedBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedBuf<N> {
    pub const fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// The bytes written so far, as a `str`.
    ///
    /// Everything written through the `Write` impl is UTF-8 (it only ever
    /// receives `&str` slices), so this can't actually fail; on the
    /// unreachable error path it returns `""` rather than panicking.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl<const N: usize> Default for FixedBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> core::fmt::Write for FixedBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let end = self.len.checked_add(bytes.len()).ok_or(core::fmt::Error)?;
        if end > N {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    #[test]
    fn writes_and_reads_back() {
        let mut b: FixedBuf<16> = FixedBuf::new();
        write!(b, "hi {}", 42).unwrap();
        assert_eq!(b.as_str(), "hi 42");
        assert_eq!(b.len(), 5);
    }

    #[test]
    fn overflow_errors_without_panicking() {
        let mut b: FixedBuf<4> = FixedBuf::new();
        assert!(write!(b, "toolong").is_err());
    }

    #[test]
    fn clear_resets_len() {
        let mut b: FixedBuf<8> = FixedBuf::new();
        write!(b, "ab").unwrap();
        b.clear();
        assert!(b.is_empty());
        write!(b, "cd").unwrap();
        assert_eq!(b.as_str(), "cd");
    }
}

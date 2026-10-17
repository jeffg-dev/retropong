//! Byte-at-a-time line assembly for the serial command protocol.
//!
//! Feeds raw CDC-ACM bytes (which may arrive split across packets, or with
//! several lines packed into one packet) and reassembles `\n`-terminated
//! lines, tolerating a preceding `\r` (so both bare-LF and CRLF line endings
//! work) exactly like `retrocontroller_protocol::serial::parse_command`'s own
//! trimming. Lines longer than `serial::MAX_LINE` bytes are dropped and
//! reported as [`FedLine::TooLong`] instead of being silently truncated or
//! panicking.

use retrocontroller_protocol::serial::MAX_LINE;

/// A line's bytes, captured at the moment `\n` completed it. Kept as an owned
/// fixed-size copy (rather than a borrow into the assembler's internal
/// buffer) so the assembler is free to reset and keep accumulating the next
/// line without fighting the borrow checker.
#[derive(Clone, Copy)]
pub struct LineBuf {
    buf: [u8; MAX_LINE],
    len: usize,
}

impl LineBuf {
    /// The line's bytes as `&str`, or `None` if they weren't valid UTF-8
    /// (which can't happen for ASCII command bytes, but a stray non-ASCII
    /// byte on the wire shouldn't panic).
    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.buf[..self.len]).ok()
    }
}

/// Result of feeding a byte that completed a line.
#[derive(Clone, Copy)]
pub enum FedLine {
    /// A complete, in-limit line.
    Ready(LineBuf),
    /// A line exceeded [`MAX_LINE`] bytes and was dropped.
    TooLong,
}

/// Text sent back for a [`FedLine::TooLong`] line. Framing errors like this
/// aren't part of `retrocontroller_protocol::serial::ParseError` (that enum
/// only covers *parsed* command syntax), so this crate defines its own fixed
/// response text for the one framing-level error case.
pub const ERR_LINE_TOO_LONG: &str = "ERR line too long";

/// Reassembles `\n`-terminated lines from a byte stream.
#[derive(Clone, Copy)]
pub struct LineAssembler {
    buf: [u8; MAX_LINE],
    len: usize,
    overflow: bool,
}

impl LineAssembler {
    pub const fn new() -> Self {
        Self { buf: [0; MAX_LINE], len: 0, overflow: false }
    }

    /// Feeds one byte. Returns `Some(_)` when a line was just completed by a
    /// `\n`; otherwise `None` (including for every `\r`, which is consumed
    /// silently in anticipation of a following `\n`).
    pub fn feed(&mut self, byte: u8) -> Option<FedLine> {
        match byte {
            b'\n' => {
                let result = if self.overflow {
                    FedLine::TooLong
                } else {
                    let mut lb = LineBuf { buf: [0; MAX_LINE], len: self.len };
                    lb.buf[..self.len].copy_from_slice(&self.buf[..self.len]);
                    FedLine::Ready(lb)
                };
                self.len = 0;
                self.overflow = false;
                Some(result)
            }
            b'\r' => None,
            b => {
                if self.len < self.buf.len() {
                    self.buf[self.len] = b;
                    self.len += 1;
                } else {
                    self.overflow = true;
                }
                None
            }
        }
    }

    /// Feeds a whole byte slice (e.g. one CDC-ACM packet), calling `on_line`
    /// for each completed line in order. A convenience wrapper around
    /// repeated [`Self::feed`] calls.
    pub fn feed_slice(&mut self, bytes: &[u8], mut on_line: impl FnMut(FedLine)) {
        for &b in bytes {
            if let Some(line) = self.feed(b) {
                on_line(line);
            }
        }
    }
}

impl Default for LineAssembler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_str(a: &mut LineAssembler, s: &str) -> heapless_stack::Collected {
        let mut collected = heapless_stack::Collected::new();
        for &b in s.as_bytes() {
            if let Some(line) = a.feed(b) {
                collected.push(line);
            }
        }
        collected
    }

    /// A tiny stack-only `Vec`-of-results substitute for these tests (max 4
    /// lines per test), so the test module itself stays `no_std`-clean.
    mod heapless_stack {
        use super::FedLine;

        pub struct Collected {
            items: [Option<FedLine>; 4],
            len: usize,
        }
        impl Collected {
            pub fn new() -> Self {
                Self { items: [None, None, None, None], len: 0 }
            }
            pub fn push(&mut self, line: FedLine) {
                self.items[self.len] = Some(line);
                self.len += 1;
            }
            pub fn as_slice(&self) -> &[Option<FedLine>] {
                &self.items[..self.len]
            }
        }
        impl Default for Collected {
            fn default() -> Self {
                Self::new()
            }
        }
    }

    fn assert_ready_eq(line: &FedLine, expected: &str) {
        match line {
            FedLine::Ready(lb) => assert_eq!(lb.as_str(), Some(expected)),
            FedLine::TooLong => panic!("expected Ready({expected:?}), got TooLong"),
        }
    }

    #[test]
    fn single_line_lf_only() {
        let mut a = LineAssembler::new();
        let out = feed_str(&mut a, "INFO\n");
        assert_eq!(out.as_slice().len(), 1);
        assert_ready_eq(out.as_slice()[0].as_ref().unwrap(), "INFO");
    }

    #[test]
    fn crlf_strips_the_cr() {
        let mut a = LineAssembler::new();
        let out = feed_str(&mut a, "RAW\r\n");
        assert_eq!(out.as_slice().len(), 1);
        assert_ready_eq(out.as_slice()[0].as_ref().unwrap(), "RAW");
    }

    #[test]
    fn multiple_lines_in_one_packet() {
        let mut a = LineAssembler::new();
        let out = feed_str(&mut a, "INFO\nRAW\nVAL\n");
        let s = out.as_slice();
        assert_eq!(s.len(), 3);
        assert_ready_eq(s[0].as_ref().unwrap(), "INFO");
        assert_ready_eq(s[1].as_ref().unwrap(), "RAW");
        assert_ready_eq(s[2].as_ref().unwrap(), "VAL");
    }

    #[test]
    fn line_split_across_two_feeds() {
        let mut a = LineAssembler::new();
        let first = feed_str(&mut a, "STRE");
        assert_eq!(first.as_slice().len(), 0, "no line completed yet");
        let second = feed_str(&mut a, "AM ON\n");
        assert_eq!(second.as_slice().len(), 1);
        assert_ready_eq(second.as_slice()[0].as_ref().unwrap(), "STREAM ON");
    }

    #[test]
    fn empty_line_is_ready_with_empty_string() {
        let mut a = LineAssembler::new();
        let out = feed_str(&mut a, "\n");
        assert_eq!(out.as_slice().len(), 1);
        assert_ready_eq(out.as_slice()[0].as_ref().unwrap(), "");
    }

    #[test]
    fn overlong_line_is_dropped_as_too_long() {
        let mut a = LineAssembler::new();
        // MAX_LINE (64) 'x's fit exactly and should NOT overflow.
        let exact: heapless_stack::Collected = {
            let s = [b'x'; MAX_LINE];
            let mut coll = heapless_stack::Collected::new();
            for &b in &s {
                if let Some(line) = a.feed(b) {
                    coll.push(line);
                }
            }
            if let Some(line) = a.feed(b'\n') {
                coll.push(line);
            }
            coll
        };
        assert_eq!(exact.as_slice().len(), 1);
        match exact.as_slice()[0].as_ref().unwrap() {
            FedLine::Ready(lb) => assert_eq!(lb.len, MAX_LINE),
            FedLine::TooLong => panic!("exactly MAX_LINE bytes must not overflow"),
        }

        // One byte over MAX_LINE must be dropped as TooLong.
        let mut b = LineAssembler::new();
        for _ in 0..(MAX_LINE + 1) {
            assert!(b.feed(b'y').is_none());
        }
        match b.feed(b'\n').unwrap() {
            FedLine::TooLong => {}
            FedLine::Ready(_) => panic!("MAX_LINE + 1 bytes must overflow"),
        }
    }

    #[test]
    fn assembler_recovers_after_an_overlong_line() {
        let mut a = LineAssembler::new();
        for _ in 0..(MAX_LINE + 5) {
            assert!(a.feed(b'z').is_none());
        }
        match a.feed(b'\n').unwrap() {
            FedLine::TooLong => {}
            FedLine::Ready(_) => panic!("expected TooLong"),
        }
        // The assembler must be usable again for the next line.
        let out = feed_str(&mut a, "INFO\n");
        assert_eq!(out.as_slice().len(), 1);
        assert_ready_eq(out.as_slice()[0].as_ref().unwrap(), "INFO");
    }

    #[test]
    fn feed_slice_convenience_matches_byte_by_byte() {
        let mut a = LineAssembler::new();
        let mut count = 0;
        a.feed_slice(b"INFO\nRAW\n", |line| {
            count += 1;
            if count == 1 {
                assert_ready_eq(&line, "INFO");
            } else {
                assert_ready_eq(&line, "RAW");
            }
        });
        assert_eq!(count, 2);
    }
}

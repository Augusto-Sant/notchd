//! Process plumbing: bounded line reading and signalling a process group.

use std::io::{self, BufRead};

/// What one read of a line-delimited stream produced.
#[derive(Debug, PartialEq)]
pub enum ReadLine {
    Line(Vec<u8>),
    /// A line longer than the cap: its bytes were skipped, this is its length.
    Overlong(usize),
    Eof,
}

/// Reads lines of at most `max` bytes without ever buffering more: an overlong
/// line is skipped to its newline and reported, not stored.
pub struct LineReader<R> {
    reader: R,
    max: usize,
}

impl<R: BufRead> LineReader<R> {
    pub fn new(reader: R, max: usize) -> Self {
        LineReader { reader, max }
    }

    pub fn next_line(&mut self) -> io::Result<ReadLine> {
        let mut buf: Vec<u8> = Vec::new();
        let mut total = 0usize;
        let mut over = false;
        loop {
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                return Ok(if total == 0 {
                    ReadLine::Eof
                } else if over {
                    ReadLine::Overlong(total)
                } else {
                    ReadLine::Line(buf)
                });
            }
            let (end, done) = match chunk.iter().position(|b| *b == b'\n') {
                Some(i) => (i, true),
                None => (chunk.len(), false),
            };
            total += end;
            if !over {
                if total > self.max {
                    over = true;
                    buf = Vec::new();
                } else {
                    buf.extend_from_slice(&chunk[..end]);
                }
            }
            let used = if done { end + 1 } else { end };
            self.reader.consume(used);
            if done {
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                return Ok(if over {
                    ReadLine::Overlong(total)
                } else {
                    ReadLine::Line(buf)
                });
            }
        }
    }
}

pub const SIGTERM: i32 = 15;
pub const SIGKILL: i32 = 9;

unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// Signals every process of the group `pgid` (the module was spawned as the
/// leader of its own group, so this reaches what it started too).
pub fn signal_group(pgid: u32, sig: i32) {
    if pgid > 1 {
        // SAFETY: kill(2) takes two integers and touches no memory.
        unsafe {
            kill(-(pgid as i32), sig);
        }
    }
}

/// Whether process `pid` still exists (signal 0).
pub fn alive(pid: u32) -> bool {
    // SAFETY: as above.
    unsafe { kill(pid as i32, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn all(data: &[u8], max: usize) -> Vec<ReadLine> {
        let mut r = LineReader::new(Cursor::new(data.to_vec()), max);
        let mut out = Vec::new();
        loop {
            match r.next_line().unwrap() {
                ReadLine::Eof => return out,
                l => out.push(l),
            }
        }
    }

    #[test]
    fn splits_lines_and_handles_a_missing_final_newline() {
        assert_eq!(
            all(b"a\nbc\r\nd", 10),
            vec![
                ReadLine::Line(b"a".to_vec()),
                ReadLine::Line(b"bc".to_vec()),
                ReadLine::Line(b"d".to_vec())
            ]
        );
    }

    #[test]
    fn skips_an_overlong_line_and_keeps_going() {
        assert_eq!(
            all(b"ok\n0123456789abcdef\nnext\n", 8),
            vec![
                ReadLine::Line(b"ok".to_vec()),
                ReadLine::Overlong(16),
                ReadLine::Line(b"next".to_vec())
            ]
        );
    }

    #[test]
    fn overlong_across_small_buffers() {
        let mut data = vec![b'x'; 5000];
        data.extend_from_slice(b"\nz\n");
        let r = io::BufReader::with_capacity(16, Cursor::new(data));
        let mut r = LineReader::new(r, 100);
        assert_eq!(r.next_line().unwrap(), ReadLine::Overlong(5000));
        assert_eq!(r.next_line().unwrap(), ReadLine::Line(b"z".to_vec()));
    }
}

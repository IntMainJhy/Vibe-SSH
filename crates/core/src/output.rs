use std::collections::VecDeque;

use vibe_ssh_protocol::Stream;

/// Bounded log addressed by absolute byte offset, so cursors survive rotation honestly.
pub(crate) struct OutputLog {
    bytes: VecDeque<u8>,
    limit: usize,
    total: u64,
    /// No more bytes will arrive; a dangling partial character is then delivered as-is.
    pub closed: bool,
}

impl OutputLog {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            limit,
            total: 0,
            closed: false,
        }
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn retained(&self, limit: usize) -> Vec<u8> {
        self.bytes(self.total.saturating_sub(limit as u64), limit).0
    }

    pub fn restored(bytes: Vec<u8>, total: u64, closed: bool) -> Self {
        Self {
            limit: bytes.len(),
            bytes: bytes.into(),
            total,
            closed,
        }
    }

    pub fn append(&mut self, data: &[u8]) {
        self.total += data.len() as u64;
        let data = &data[data.len().saturating_sub(self.limit)..];
        let overflow = (self.bytes.len() + data.len()).saturating_sub(self.limit);
        self.bytes.drain(..overflow);
        self.bytes.extend(data);
    }

    /// Reads up to `max` bytes at `from`. Returns the text and the offset to continue from.
    pub fn read(&self, from: u64, max: usize) -> (Stream, u64) {
        self.window(from, max, false)
    }

    /// The last `max` bytes, flagged truncated when earlier output exists.
    pub fn tail(&self, max: usize) -> (Stream, u64) {
        let from = self.total.saturating_sub(max as u64);
        self.window(from, max, from > 0)
    }

    /// Raw bytes at `from`: `(bytes, next offset, a gap precedes them)`.
    pub fn bytes(&self, from: u64, max: usize) -> (Vec<u8>, u64, bool) {
        let oldest = self.total - self.bytes.len() as u64;
        let start = from.clamp(oldest, self.total);
        let skip = (start - oldest) as usize;
        let bytes: Vec<u8> = self.bytes.iter().skip(skip).take(max).copied().collect();
        let next = start + bytes.len() as u64;
        (bytes, next, start > from)
    }

    fn window(&self, from: u64, max: usize, cut: bool) -> (Stream, u64) {
        let (mut bytes, mut next, gap) = self.bytes(from, max);
        let cut = cut || gap;
        if cut {
            // A rotated or tail window may begin inside a code point.
            let lead = bytes
                .iter()
                .position(|b| b & 0xc0 != 0x80)
                .unwrap_or(bytes.len());
            bytes.drain(..lead);
        }
        // Hold back a trailing partial character unless the stream has ended there.
        if next < self.total || !self.closed {
            let partial = partial_suffix(&bytes);
            bytes.truncate(bytes.len() - partial);
            next -= partial as u64;
        }
        let stream = Stream {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            total_bytes: self.total,
            // A cursor beyond the end is reported, never silently accepted.
            truncated: cut || from > self.total,
            invalid_utf8: std::str::from_utf8(&bytes).is_err(),
        };
        (stream, next)
    }
}

/// Length of an incomplete UTF-8 sequence at the end of `bytes`.
pub(crate) fn partial_suffix(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xc0 == 0x80 {
            continue;
        }
        let need = match byte {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        return if need > back { back } else { 0 };
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_reads_never_split_a_character_across_packets_or_budgets() {
        let mut log = OutputLog::new(1024);
        let text = "部署完成🚀 done";
        let mut seen = String::new();
        let mut cursor = 0;
        for byte in text.as_bytes() {
            log.append(&[*byte]);
            let (stream, next) = log.read(cursor, 4);
            assert!(!stream.invalid_utf8 && !stream.truncated);
            seen.push_str(&stream.text);
            cursor = next;
        }
        assert_eq!(seen, text);
        assert_eq!(cursor, log.total());
    }

    #[test]
    fn rotation_reports_the_gap_and_resumes_on_a_character_boundary() {
        let mut log = OutputLog::new(8);
        log.append("很多历史内容🚀成功".as_bytes());
        let (stream, next) = log.read(0, 64);
        assert!(stream.truncated);
        assert_eq!(stream.text, "成功");
        assert_eq!(next, log.total());
        let (again, _) = log.read(next, 64);
        assert!(again.text.is_empty() && !again.truncated);
    }

    #[test]
    fn tail_flags_earlier_output_and_keeps_unicode_intact() {
        let mut log = OutputLog::new(1024);
        log.append("第一行\n第二行🚀".as_bytes());
        let (stream, next) = log.tail(8);
        assert!(stream.truncated && !stream.invalid_utf8);
        assert_eq!(stream.text, "行🚀");
        assert_eq!(next, log.total());
    }

    #[test]
    fn invalid_bytes_are_flagged_and_a_closed_stream_still_drains() {
        let mut log = OutputLog::new(64);
        log.append(&[b'a', 0xff, 0xf0]);
        let (open, next) = log.read(0, 64);
        assert!(open.invalid_utf8);
        assert_eq!(next, 2, "a dangling lead byte waits for more data");
        log.closed = true;
        let (rest, end) = log.read(next, 64);
        assert_eq!(end, 3);
        assert!(rest.invalid_utf8);
        assert!(
            log.read(99, 64).0.truncated,
            "a future cursor is not silently accepted"
        );
    }
}

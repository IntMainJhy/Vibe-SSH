use std::collections::VecDeque;

use vibe_ssh_protocol::CapturedOutput;

/// Stores a byte tail; network packet boundaries must not affect UTF-8 decoding.
pub(crate) struct OutputTail {
    bytes: VecDeque<u8>,
    limit: usize,
    total: u64,
}

impl OutputTail {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(limit),
            limit,
            total: 0,
        }
    }

    pub fn append(&mut self, data: &[u8]) {
        self.total = self.total.saturating_add(data.len() as u64);
        if data.len() >= self.limit {
            self.bytes.clear();
            self.bytes.extend(&data[data.len() - self.limit..]);
            return;
        }
        let overflow = (self.bytes.len() + data.len()).saturating_sub(self.limit);
        self.bytes.drain(..overflow);
        self.bytes.extend(data);
    }

    pub fn snapshot(&self) -> CapturedOutput {
        let bytes: Vec<_> = self.bytes.iter().copied().collect();
        let truncated = self.total > bytes.len() as u64;
        // A retained tail may begin in the middle of a valid code point.
        let start = if truncated {
            bytes
                .iter()
                .position(|b| b & 0xc0 != 0x80)
                .unwrap_or(bytes.len())
        } else {
            0
        };
        let bytes = &bytes[start..];
        CapturedOutput {
            text: String::from_utf8_lossy(bytes).into_owned(),
            total_bytes: self.total,
            retained_bytes: bytes.len(),
            truncated,
            invalid_utf8: std::str::from_utf8(bytes).is_err(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassembles_unicode_split_between_packets() {
        let mut tail = OutputTail::new(16);
        for byte in "部署完成🚀".as_bytes() {
            tail.append(&[*byte]);
        }
        let output = tail.snapshot();
        assert_eq!(output.text, "部署完成🚀");
        assert!(!output.invalid_utf8);
        assert!(!output.truncated);
    }

    #[test]
    fn bounds_large_packets_and_drops_partial_leading_character() {
        let mut tail = OutputTail::new(8);
        tail.append("很多历史内容🚀成功".as_bytes());
        let output = tail.snapshot();
        assert_eq!(output.text, "成功");
        assert!(output.truncated);
        assert!(output.retained_bytes <= 8);
        assert!(!output.invalid_utf8);
    }

    #[test]
    fn flags_invalid_remote_bytes_instead_of_claiming_utf8() {
        let mut tail = OutputTail::new(8);
        tail.append(&[0xff, b'a']);
        assert!(tail.snapshot().invalid_utf8);
        assert_eq!(tail.snapshot().total_bytes, 2);
    }
}

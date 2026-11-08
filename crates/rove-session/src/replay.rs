//! Bounded buffer of recent output used to repaint a re-attached view.

use std::collections::VecDeque;

use bytes::Bytes;

/// Keeps the most recent `capacity` bytes of output.
///
/// Trimming drops whole leading bytes and may split a UTF-8 character or an escape sequence at the
/// start of the buffer; terminal emulators tolerate a damaged first sequence.
#[derive(Debug)]
pub(crate) struct ReplayBuffer {
    capacity: usize,
    data: VecDeque<u8>,
}

impl ReplayBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            data: VecDeque::with_capacity(capacity.min(64 * 1024)),
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) {
        if self.capacity == 0 {
            return;
        }
        if chunk.len() >= self.capacity {
            self.data.clear();
            self.data.extend(&chunk[chunk.len() - self.capacity..]);
            return;
        }
        let overflow = (self.data.len() + chunk.len()).saturating_sub(self.capacity);
        self.data.drain(..overflow);
        self.data.extend(chunk);
    }

    pub(crate) fn snapshot(&self) -> Bytes {
        let (front, back) = self.data.as_slices();
        let mut out = Vec::with_capacity(self.data.len());
        out.extend_from_slice(front);
        out.extend_from_slice(back);
        Bytes::from(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_most_recent_bytes() {
        let mut buffer = ReplayBuffer::new(5);
        buffer.push(b"abc");
        buffer.push(b"defg");
        assert_eq!(&buffer.snapshot()[..], b"cdefg");
        buffer.push(b"0123456789");
        assert_eq!(&buffer.snapshot()[..], b"56789");
    }

    #[test]
    fn zero_capacity_keeps_nothing() {
        let mut buffer = ReplayBuffer::new(0);
        buffer.push(b"abc");
        assert!(buffer.snapshot().is_empty());
    }
}

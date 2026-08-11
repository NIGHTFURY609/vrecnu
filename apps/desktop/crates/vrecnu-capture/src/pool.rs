use std::sync::{Arc, Mutex};

/// A fixed set of reusable frame buffers.
///
/// At 1080p a BGRA frame is 8.29 MB, and thirty of them per second is ~249 MB/s. Allocating
/// that per frame means a quarter of a gigabyte per second of allocator churn and page faults
/// — exactly the kind of avoidable cost that turns into dropped frames. The pool is filled
/// once, before capture starts, and never grows during a session.
#[derive(Debug)]
pub struct FramePool {
    free: Arc<Mutex<Vec<Vec<u8>>>>,
    frame_len: usize,
    capacity: usize,
}

impl FramePool {
    /// Preallocates `capacity` buffers of `frame_len` bytes each.
    #[must_use]
    pub fn new(frame_len: usize, capacity: usize) -> Self {
        let mut buffers = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffers.push(vec![0u8; frame_len]);
        }
        Self {
            free: Arc::new(Mutex::new(buffers)),
            frame_len,
            capacity,
        }
    }

    /// Takes a buffer, or `None` when every one is in flight.
    ///
    /// `None` *is* the backpressure signal. Allocating a fresh buffer instead would turn a
    /// stalled encoder into an out-of-memory kill in about twenty seconds.
    #[must_use]
    pub fn take(&self) -> Option<PooledFrame> {
        let buffer = self.free.lock().expect("frame pool mutex poisoned").pop()?;
        Some(PooledFrame {
            buffer,
            pool: Arc::clone(&self.free),
        })
    }

    /// Buffers currently available.
    #[must_use]
    pub fn available(&self) -> usize {
        self.free.lock().expect("frame pool mutex poisoned").len()
    }

    /// Total buffers in the pool.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Size of each buffer in bytes.
    #[must_use]
    pub const fn frame_len(&self) -> usize {
        self.frame_len
    }
}

/// A buffer borrowed from a [`FramePool`], returned to it on drop.
#[derive(Debug)]
pub struct PooledFrame {
    buffer: Vec<u8>,
    pool: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl PooledFrame {
    /// Writable view of the buffer.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    /// Readable view of the buffer.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }
}

impl Drop for PooledFrame {
    fn drop(&mut self) {
        // `mem::take` leaves an empty Vec behind, so the buffer is *moved* back to the pool
        // with its allocation intact rather than being cloned or freed.
        let buffer = std::mem::take(&mut self.buffer);
        if let Ok(mut free) = self.pool.lock() {
            free.push(buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_return_to_the_pool_on_drop() {
        let pool = FramePool::new(1024, 2);
        assert_eq!(pool.available(), 2);
        {
            let _held = pool.take().expect("pool should have a buffer");
            assert_eq!(pool.available(), 1);
        }
        assert_eq!(pool.available(), 2);
    }

    #[test]
    fn exhaustion_reports_none_rather_than_allocating() {
        // This is the backpressure signal; growing here would be an OOM in ~20 seconds.
        let pool = FramePool::new(64, 1);
        let _held = pool.take().expect("first take should succeed");
        assert!(pool.take().is_none());
    }

    #[test]
    fn returned_buffers_keep_their_capacity() {
        let pool = FramePool::new(4096, 1);
        {
            let mut frame = pool.take().expect("first take");
            assert_eq!(frame.as_mut_slice().len(), 4096);
        }
        assert_eq!(pool.take().expect("second take").as_slice().len(), 4096);
    }

    #[test]
    fn a_1080p_pool_reserves_the_expected_bytes() {
        let frame_len = vrecnu_types::Size {
            width: 1920,
            height: 1080,
        }
        .bgra_byte_len();
        let pool = FramePool::new(frame_len, 6);
        assert_eq!(pool.frame_len(), 8_294_400);
        assert_eq!(pool.capacity(), 6);
    }
}

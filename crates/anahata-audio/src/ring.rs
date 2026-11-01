use rtrb::{Consumer, Producer, RingBuffer};

/// Ring buffer holding interleaved f32 audio samples between the decode worker and audio callback.
pub struct AudioRingBuffer;

impl AudioRingBuffer {
    /// Creates a lock-free, single-producer single-consumer ring buffer with the given sample capacity.
    /// Capacity is rounded or sized in terms of f32 samples.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(capacity_samples: usize) -> (Producer<f32>, Consumer<f32>) {
        RingBuffer::<f32>::new(capacity_samples)
    }
}

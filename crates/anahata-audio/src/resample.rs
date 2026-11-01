//! Streaming stereo sample-rate converter.
//!
//! Uses 4-point Catmull-Rom (cubic) interpolation. It runs on the decode worker
//! thread, never in the audio callback, so allocation here is fine. When the
//! input and output rates match it is a straight copy.

/// Converts interleaved stereo f32 audio from one sample rate to another,
/// keeping state between calls so packets can be fed one at a time.
pub struct StereoResampler {
    step: f64,
    pos: f64,
    buf: Vec<f32>,
    primed: bool,
    passthrough: bool,
}

#[inline]
fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

impl StereoResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let in_rate = in_rate.max(1);
        let out_rate = out_rate.max(1);
        Self {
            step: in_rate as f64 / out_rate as f64,
            pos: 0.0,
            buf: Vec::new(),
            primed: false,
            passthrough: in_rate == out_rate,
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.passthrough
    }

    /// Appends `input` (interleaved stereo) and writes every output frame that can be
    /// computed so far to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        if input.len() < 2 {
            return;
        }
        if !self.primed {
            // Index 0 is a pad copy of the first frame, so the first real frame sits at 1.0.
            self.buf.push(input[0]);
            self.buf.push(input[1]);
            self.pos = 1.0;
            self.primed = true;
        }
        self.buf.extend_from_slice(input);
        self.drain(out);
    }

    /// Emits the tail of the stream. Call once at end of file.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        if self.passthrough || !self.primed || self.buf.len() < 2 {
            return;
        }
        let n = self.buf.len();
        let (l, r) = (self.buf[n - 2], self.buf[n - 1]);
        for _ in 0..2 {
            self.buf.push(l);
            self.buf.push(r);
        }
        self.drain(out);
    }

    /// Forgets all history (used after a seek).
    pub fn reset(&mut self) {
        self.buf.clear();
        self.pos = 0.0;
        self.primed = false;
    }

    fn drain(&mut self, out: &mut Vec<f32>) {
        let frames = self.buf.len() / 2;
        loop {
            let i = self.pos.floor() as usize;
            if i + 2 >= frames {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            for ch in 0..2 {
                let p0 = self.buf[(i - 1) * 2 + ch];
                let p1 = self.buf[i * 2 + ch];
                let p2 = self.buf[(i + 1) * 2 + ch];
                let p3 = self.buf[(i + 2) * 2 + ch];
                out.push(catmull_rom(p0, p1, p2, p3, t));
            }
            self.pos += self.step;
        }
        // Drop frames we will never need again, keeping one frame of history.
        let consumed = (self.pos.floor() as usize).saturating_sub(1);
        if consumed > 0 {
            self.buf.drain(..consumed * 2);
            self.pos -= consumed as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, frames: usize) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = (i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin() * 0.5;
            v.push(s);
            v.push(s);
        }
        v
    }

    fn run(in_rate: u32, out_rate: u32, input: &[f32], chunk: usize) -> Vec<f32> {
        let mut r = StereoResampler::new(in_rate, out_rate);
        let mut out = Vec::new();
        for c in input.chunks(chunk) {
            r.process(c, &mut out);
        }
        r.flush(&mut out);
        out
    }

    #[test]
    fn passthrough_is_identity() {
        let input = sine(48000, 440.0, 1000);
        assert_eq!(run(48000, 48000, &input, 256), input);
    }

    #[test]
    fn output_length_scales_with_ratio() {
        let input = sine(44100, 440.0, 44100);
        let out = run(44100, 48000, &input, 1152);
        let frames = out.len() / 2;
        assert!((frames as i64 - 48000).abs() <= 4, "got {frames} frames");
    }

    #[test]
    fn downsampling_length() {
        let input = sine(96000, 440.0, 96000);
        let out = run(96000, 48000, &input, 4096);
        let frames = out.len() / 2;
        assert!((frames as i64 - 48000).abs() <= 4, "got {frames} frames");
    }

    #[test]
    fn pitch_is_preserved() {
        // A 440 Hz tone must still be 440 Hz after 44.1k -> 48k: count rising zero crossings.
        let input = sine(44100, 440.0, 44100);
        let out = run(44100, 48000, &input, 1000);
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        let crossings = left.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((crossings as i64 - 440).abs() <= 2, "got {crossings} cycles");
    }

    #[test]
    fn chunk_size_does_not_change_result() {
        let input = sine(44100, 1000.0, 5000);
        let a = run(44100, 48000, &input, 64);
        let b = run(44100, 48000, &input, 4999);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-5);
        }
    }

    #[test]
    fn reset_restarts_cleanly() {
        let input = sine(44100, 440.0, 2000);
        let mut r = StereoResampler::new(44100, 48000);
        let mut first = Vec::new();
        r.process(&input, &mut first);
        r.reset();
        let mut second = Vec::new();
        r.process(&input, &mut second);
        assert_eq!(first, second);
    }
}

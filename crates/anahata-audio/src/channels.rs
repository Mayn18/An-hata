/// Maps or downmixes any channel count to stereo interleaved samples [L, R, L, R, ...].
///
/// Implements standard ITU coefficients for multi-channel downmixing:
/// - Mono (1 channel) -> duplicated to L and R.
/// - Stereo (2 channels) -> passthrough.
/// - 3 channels (L, R, C) -> L + 0.7071 * C, R + 0.7071 * C.
/// - 4 channels (quad) -> L + 0.7071 * RL, R + 0.7071 * RR.
/// - 5.1 / 6 channels -> standard ITU downmix:
///   L_out = (L + 0.7071 * C + 0.7071 * Ls) * 0.7071
///   R_out = (R + 0.7071 * C + 0.7071 * Rs) * 0.7071
pub fn downmix_to_stereo(input: &[f32], channels: usize, output: &mut Vec<f32>) {
    if channels == 0 || input.is_empty() {
        return;
    }

    let frame_count = input.len() / channels;
    output.clear();
    output.reserve(frame_count * 2);

    match channels {
        1 => {
            for &sample in input {
                output.push(sample);
                output.push(sample);
            }
        }
        2 => {
            output.extend_from_slice(input);
        }
        3 => {
            // L, R, C
            let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
            for chunk in input.chunks_exact(3) {
                let l = chunk[0];
                let r = chunk[1];
                let c = chunk[2];
                output.push((l + c * inv_sqrt2).clamp(-1.0, 1.0));
                output.push((r + c * inv_sqrt2).clamp(-1.0, 1.0));
            }
        }
        4 => {
            // L, R, RL, RR (Quad)
            let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
            for chunk in input.chunks_exact(4) {
                let l = chunk[0];
                let r = chunk[1];
                let rl = chunk[2];
                let rr = chunk[3];
                output.push(((l + rl * inv_sqrt2) * inv_sqrt2).clamp(-1.0, 1.0));
                output.push(((r + rr * inv_sqrt2) * inv_sqrt2).clamp(-1.0, 1.0));
            }
        }
        6 => {
            // 5.1 Surround: L, R, C, LFE, Ls, Rs
            let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
            for chunk in input.chunks_exact(6) {
                let l = chunk[0];
                let r = chunk[1];
                let c = chunk[2];
                // LFE chunk[3] discarded
                let ls = chunk[4];
                let rs = chunk[5];

                let l_mix = (l + c * inv_sqrt2 + ls * inv_sqrt2) * 0.61237;
                let r_mix = (r + c * inv_sqrt2 + rs * inv_sqrt2) * 0.61237;
                output.push(l_mix.clamp(-1.0, 1.0));
                output.push(r_mix.clamp(-1.0, 1.0));
            }
        }
        _ => {
            // Generic fallback: average all even channels to Left, odd to Right
            for chunk in input.chunks_exact(channels) {
                let mut l_sum = 0.0f32;
                let mut r_sum = 0.0f32;
                let mut l_count = 0usize;
                let mut r_count = 0usize;

                for (i, &sample) in chunk.iter().enumerate() {
                    if i % 2 == 0 {
                        l_sum += sample;
                        l_count += 1;
                    } else {
                        r_sum += sample;
                        r_count += 1;
                    }
                }

                let l = if l_count > 0 { l_sum / l_count as f32 } else { 0.0 };
                let r = if r_count > 0 { r_sum / r_count as f32 } else { l };
                output.push(l.clamp(-1.0, 1.0));
                output.push(r.clamp(-1.0, 1.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mono_to_stereo() {
        let input = vec![0.5, -0.5, 0.0];
        let mut out = Vec::new();
        downmix_to_stereo(&input, 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5, 0.0, 0.0]);
    }

    #[test]
    fn test_stereo_passthrough() {
        let input = vec![0.1, 0.2, -0.3, -0.4];
        let mut out = Vec::new();
        downmix_to_stereo(&input, 2, &mut out);
        assert_eq!(out, input);
    }
}

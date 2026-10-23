//! Noise suppression (RNNoise via `nnnoiseless`), feature `audio`.
//!
//! Works on mono 48 kHz f32 samples in -1.0..=1.0. Audio APIs deliver
//! arbitrary block sizes (128-sample AudioWorklet quanta, 10 ms native
//! callbacks), RNNoise needs 480-sample frames — so this is a streaming FIFO
//! with 10 ms of algorithmic latency. Used by the desktop app's capture
//! pipeline and, through wasm-bindgen, by an AudioWorklet on the web.

use std::collections::VecDeque;

use nnnoiseless::DenoiseState;

pub const FRAME: usize = DenoiseState::FRAME_SIZE; // 480 samples = 10 ms @ 48 kHz
const SCALE: f32 = 32767.0;

pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    input: VecDeque<f32>,
    output: VecDeque<f32>,
    frame_in: Vec<f32>,
    frame_out: Vec<f32>,
    /// Exponential average of the voice-activity probability.
    vad: f32,
    first: bool,
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser {
    pub fn new() -> Self {
        let mut output = VecDeque::with_capacity(FRAME * 4);
        // Prime with one frame of silence so `process` can always fill its output.
        output.extend(std::iter::repeat_n(0.0, FRAME));
        Self {
            state: DenoiseState::new(),
            input: VecDeque::with_capacity(FRAME * 4),
            output,
            frame_in: vec![0.0; FRAME],
            frame_out: vec![0.0; FRAME],
            vad: 0.0,
            first: true,
        }
    }

    /// Denoise `samples` in place (any length). Output is delayed by one frame.
    pub fn process(&mut self, samples: &mut [f32]) {
        self.input.extend(samples.iter().map(|s| s * SCALE));
        while self.input.len() >= FRAME {
            for (dst, src) in self.frame_in.iter_mut().zip(self.input.drain(..FRAME)) {
                *dst = src;
            }
            let p = self
                .state
                .process_frame(&mut self.frame_out, &self.frame_in);
            self.vad = 0.9 * self.vad + 0.1 * p;
            if self.first {
                // RNNoise's very first frame contains fade-in artifacts.
                self.first = false;
                self.output.extend(std::iter::repeat_n(0.0, FRAME));
            } else {
                self.output
                    .extend(self.frame_out.iter().map(|s| (s / SCALE).clamp(-1.0, 1.0)));
            }
        }
        for s in samples.iter_mut() {
            *s = self.output.pop_front().unwrap_or(0.0);
        }
    }

    /// Smoothed probability (0..1) that the current audio is speech.
    pub fn voice_probability(&self) -> f32 {
        self.vad
    }
}

#[cfg(feature = "wasm")]
mod web {
    use wasm_bindgen::prelude::*;

    /// `new WasmDenoiser()`; call `process(Float32Array)` from an AudioWorklet.
    #[wasm_bindgen]
    pub struct WasmDenoiser(super::Denoiser);

    #[wasm_bindgen]
    impl WasmDenoiser {
        #[wasm_bindgen(constructor)]
        pub fn new() -> WasmDenoiser {
            WasmDenoiser(super::Denoiser::new())
        }

        pub fn process(&mut self, samples: &mut [f32]) -> f32 {
            self.0.process(samples);
            self.0.voice_probability()
        }
    }

    impl Default for WasmDenoiser {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: &mut u64, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 7;
                *seed ^= *seed << 17;
                ((*seed >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32
    }

    #[test]
    fn arbitrary_block_sizes_preserve_length() {
        let mut d = Denoiser::new();
        for len in [128, 128, 37, 480, 1000, 1] {
            let mut buf = vec![0.1; len];
            d.process(&mut buf);
            assert_eq!(buf.len(), len);
            assert!(buf.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        }
    }

    #[test]
    fn suppresses_stationary_noise() {
        let mut d = Denoiser::new();
        let mut seed = 0x9E37_79B9_7F4A_7C15;
        // 2 s of low-frequency ("brown") noise at a background level — fans, hum, traffic.
        // (RNNoise attenuates spectrally flat white noise much less, about −6 dB.)
        let white = noise(48_000 * 2, &mut seed, 0.02);
        let mut lp = 0.0f32;
        let input: Vec<f32> = white
            .iter()
            .map(|x| {
                lp = 0.97 * lp + x * 2.0;
                lp
            })
            .collect();
        let mut out = input.clone();
        for chunk in out.chunks_mut(128) {
            d.process(chunk);
        }
        // Compare the second second (after adaptation).
        let tail_in = energy(&input[48_000..]);
        let tail_out = energy(&out[48_000..]);
        eprintln!(
            "noise energy {tail_in} → {tail_out} ({:.1} dB)",
            10.0 * (tail_out / tail_in).log10()
        );
        assert!(
            tail_out < tail_in * 0.01,
            "expected ≥ 20 dB suppression: {tail_in} → {tail_out}"
        );
        assert!(d.voice_probability() < 0.5);
    }
}

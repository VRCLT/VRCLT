use super::{Decimator, FRAME};
use anyhow::Result;
use ort::{session::Session, value::TensorRef};
use std::time::Duration;

/// Classify the original game audio. Silence travels through the same delay as
/// speech, including when the restoration worker falls back to its CPU path.
pub(super) struct SourceGate {
    detector: Session,
    state: [f32; 256],
    resampler: Decimator,
    window: [f32; 576],
    offset: usize,
    delayed: [i16; FRAME],
    threshold: f32,
    hold_frames: usize,
    remaining: usize,
}

impl SourceGate {
    pub fn new(threshold: f32, hangover: Duration) -> Result<Self> {
        Ok(Self {
            detector: Session::builder()?
                .with_intra_threads(1)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .with_inter_threads(1)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .commit_from_memory(include_bytes!("models/silero_vad.onnx"))?,
            state: [0.; 256],
            resampler: Decimator::new(16000),
            window: [0.; 576],
            offset: 64,
            delayed: [0; FRAME],
            threshold,
            hold_frames: (hangover.as_secs_f64() / 0.016).ceil() as usize,
            remaining: 0,
        })
    }

    fn probability(&mut self) -> Result<f32> {
        // Same 64 samples of context, recurrent state and input scaling as
        // vrclt/audio/vad.py. Both are carried across callback boundaries.
        let output = self.detector.run(ort::inputs![
            "input" => TensorRef::from_array_view(([1, 576], &self.window[..]))?,
            "state" => TensorRef::from_array_view(([2, 1, 128], &self.state[..]))?,
            "sr" => TensorRef::from_array_view(([] as [usize; 0], &[16000_i64][..]))?,
        ])?;
        let probability = output["output"].try_extract_tensor::<f32>()?.1[0];
        self.state
            .copy_from_slice(output["stateN"].try_extract_tensor::<f32>()?.1);
        self.window.copy_within(512..576, 0);
        Ok(probability)
    }

    /// Returns whether the output belongs to an admitted window, and whether
    /// this inference actually detected speech (a hangover is not new speech).
    pub fn process(&mut self, frame: &mut [i16; FRAME]) -> Result<(bool, bool)> {
        let mut samples = [0; 256];
        self.resampler.process(frame, &mut samples);
        for sample in samples {
            self.window[self.offset] = f32::from(sample) / 32768.;
            self.offset += 1;
        }
        let mut detected = false;
        if self.offset == self.window.len() {
            detected = self.probability()? >= self.threshold;
            self.offset = 64;
            if detected {
                // Include both halves of the classified window and the trailing hold.
                self.remaining = 2 + self.hold_frames;
            }
        }
        std::mem::swap(frame, &mut self.delayed);
        let open = self.remaining > 0;
        if open {
            self.remaining -= 1;
        } else {
            frame.fill(0);
        }
        Ok((open, detected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_legacy_probabilities_across_silence_and_onset() {
        // Reference values from vrclt.audio.vad.SileroVAD on identical PCM.
        let reference = [
            0.0016698, 0.0068839, 0.0089107, 0.4750721, 0.5225841, 0.4708572, 0.3507577, 0.3393370,
            0.3028964, 0.2605564,
        ];
        let mut gate = SourceGate::new(0.5, Duration::from_millis(350)).unwrap();
        for (step, expected) in reference.into_iter().enumerate() {
            for (n, v) in gate.window[64..].iter_mut().enumerate() {
                let x = (step * 512 + n) as i32;
                *v = if step < 3 {
                    0.
                } else {
                    (((x % 100) - 50) * 110 + ((x % 31) - 15) * 75) as f32 / 32768.
                };
            }
            let actual = gate.probability().unwrap();
            assert!(
                (actual - expected).abs() < 0.0001,
                "window {step}: {actual} vs {expected}"
            );
        }
    }
}

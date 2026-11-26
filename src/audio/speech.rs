use anyhow::Result;
use deep_filter_rt::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;

const BLOCK: usize = 480;

#[derive(Default)]
pub(super) struct SpeechOnset {
    consecutive: u8,
}
impl SpeechOnset {
    /// Four consecutive capture frames to start a turn. The existing preroll
    /// includes the onset, and the hangover keeps gaps inside a turn audible.
    pub fn accept(&mut self, detected: bool, during_turn: bool) -> bool {
        self.consecutive = if detected {
            self.consecutive.saturating_add(1)
        } else {
            0
        };
        self.consecutive >= if during_turn { 2 } else { 4 }
    }
}

/// Stream 48 kHz mono PCM through DeepFilterNet3 across arbitrary capture packet boundaries.
/// Keep one block of output ready while collecting the next input block.
pub(super) struct SpeechFilter {
    model: DfTract,
    input: Array2<f32>,
    output: Array2<f32>,
    output_snr: f32,
    offset: usize,
}
impl SpeechFilter {
    pub fn new() -> Result<Self> {
        Ok(Self {
            model: DfTract::new(DfParams::default(), &RuntimeParams::default())?,
            input: Array2::zeros((1, BLOCK)),
            output: Array2::zeros((1, BLOCK)),
            output_snr: -15.,
            offset: 0,
        })
    }

    pub fn process(&mut self, mut samples: &mut [i16]) -> Result<bool> {
        let mut peak_snr = f32::NEG_INFINITY;
        while !samples.is_empty() {
            let count = samples.len().min(BLOCK - self.offset);
            let (chunk, rest) = samples.split_at_mut(count);
            peak_snr = peak_snr.max(self.output_snr);
            for (index, sample) in (self.offset..self.offset + count).zip(chunk) {
                self.input[[0, index]] = f32::from(*sample) / 32768.;
                *sample = (self.output[[0, index]] * 32768.).round() as i16;
            }
            self.offset += count;
            if self.offset == BLOCK {
                self.output_snr = self
                    .model
                    .process(self.input.view(), self.output.view_mut())?;
                self.offset = 0;
            }
            samples = rest;
        }
        // A speech detector can mistake pitched instruments for syllables. Require
        // some estimated voice energy as well, while allowing speech below the backing track.
        Ok(peak_snr >= -10.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{Decimator, FRAME, rms};

    #[test]
    fn short_bursts_do_not_open_a_new_turn() {
        let mut onset = SpeechOnset::default();
        for detected in [true, true, true, false, true, true, true] {
            assert!(!onset.accept(detected, false));
        }
        assert!(onset.accept(true, false));
        assert!(!onset.accept(false, true));
        assert!(!onset.accept(true, true));
        assert!(onset.accept(true, true));
    }

    #[test]
    fn suppresses_steady_noise_across_capture_packet_boundaries() {
        let mut seed = 7u32;
        let mut noise: Vec<i16> = (0..48_000)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 16) as i16) / 8
            })
            .collect();
        let mut reference = noise.clone();
        let mut aligned = SpeechFilter::new().unwrap();
        for block in reference.chunks_mut(BLOCK) {
            aligned.process(block).unwrap();
        }
        let before = rms(&noise[24_000..]);
        let mut streaming = SpeechFilter::new().unwrap();
        for packet in noise.chunks_mut(FRAME) {
            streaming.process(packet).unwrap();
        }
        assert_eq!(noise, reference);
        assert!(rms(&noise[24_000..]) < before * 0.32);
    }

    #[test]
    #[ignore = "requires VRCLT_SPEECH_PCM, clean mono signed 16-bit PCM at 48 kHz"]
    fn speech_over_background_music() {
        let bytes = std::fs::read(std::env::var_os("VRCLT_SPEECH_PCM").unwrap()).unwrap();
        let speech: Vec<f32> = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32)
            .collect();
        assert!(speech.len() >= 48_000 * 3);
        let mut music = Vec::with_capacity(speech.len());
        for i in 0..speech.len() {
            let t = i as f32 / 48_000.;
            let beat = (t / 0.375) as usize;
            let note = [261.63, 329.63, 392., 523.25, 220., 329.63, 440., 293.66][beat % 8];
            let age = t % 0.375;
            let phase = std::f32::consts::TAU * age * note;
            let piano = (phase.sin() + 0.35 * (phase * 2.).sin() + 0.18 * (phase * 3.).sin())
                * (-4. * age).exp();
            let bass = (std::f32::consts::TAU * (note * 0.5) * age).sin() * (-2. * age).exp();
            music.push(1900. * piano + 1300. * bass);
        }
        let mixture: Vec<_> = speech
            .iter()
            .zip(&music)
            .map(|(s, m)| (s + m) as i16)
            .collect();
        let run = |input: &[i16]| {
            let mut samples = input.to_vec();
            let mut focus = SpeechFilter::new().unwrap();
            let mut vad = earshot::Detector::default_boxed();
            let mut down = Decimator::new(16_000);
            let mut speech_frames = 0;
            let mut starts = SpeechOnset::default();
            let mut last_speech = None;
            for (i, block) in samples.chunks_exact_mut(FRAME).enumerate() {
                let voice_energy = focus.process(block).unwrap();
                let mut narrow = [0; 256];
                down.process((&*block).try_into().unwrap(), &mut narrow);
                let detected = vad.predict_i16(&narrow) >= 0.5 && voice_energy;
                if starts.accept(
                    detected,
                    last_speech.is_some_and(|at: usize| (i - at) * 16 <= 350),
                ) {
                    speech_frames += 1;
                    last_speech = Some(i);
                }
            }
            (samples, speech_frames)
        };
        let (background, music_frames) = run(&music.iter().map(|&v| v as i16).collect::<Vec<_>>());
        let (enhanced, mixed_frames) = run(&mixture);
        let (_, clean_frames) = run(&speech.iter().map(|&v| v as i16).collect::<Vec<_>>());
        // The stream adapter adds one 10 ms block to the model's overlap and lookahead.
        let score = |signal: &[i16], delay: usize| {
            let n = (speech.len() / FRAME * FRAME) - delay;
            let pairs = || {
                speech[..n]
                    .iter()
                    .zip(&signal[delay..delay + n])
                    .map(|(&s, &y)| (f64::from(s), f64::from(y)))
            };
            let power: f64 = pairs().map(|(s, _)| s * s).sum();
            let gain = pairs().map(|(s, y)| s * y).sum::<f64>() / power;
            let error: f64 = pairs().map(|(s, y)| (y - gain * s).powi(2)).sum();
            10. * (gain * gain * power / error).log10()
        };
        let input_score = score(&mixture, 0);
        let output_score = score(
            &enhanced,
            BLOCK + SpeechFilter::new().unwrap().model.delay(),
        );
        let skip = 48_000;
        let end = background.len() / FRAME * FRAME;
        let background_rms =
            (music[skip..end].iter().map(|v| v * v).sum::<f32>() / (end - skip) as f32).sqrt();
        let attenuation = 20. * (rms(&background[skip..end]) / background_rms).log10();
        println!(
            "Instrumental level {attenuation:.1} dB, SI-SDR {input_score:.1} -> {output_score:.1} dB, VAD frames: music {music_frames}, mixture {mixed_frames}, clean {clean_frames}"
        );
        assert!(attenuation < -6. && output_score > input_score);
        assert!(
            clean_frames > 0 && music_frames < clean_frames / 4 && mixed_frames > clean_frames / 2
        );
    }
}

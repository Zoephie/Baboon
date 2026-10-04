//! A decoded sound and the summaries its waveform is drawn from.
//! It owns the peak summaries; drawing them belongs to the sound player.
//!
//! The summaries follow Audacity's two levels: a min, max and mean square for
//! every 256 frames and every 65,536 frames of each channel. A pixel column
//! covering fewer than 256 frames reads the samples themselves, so a short
//! sound or a close zoom is exact. For three minutes of 48 kHz stereo that is
//! about 67,500 fine entries a channel, 1 MiB in all, built in one pass.

use std::sync::Arc;

use blam_tags::audio::DecodedPcm;

/// Frames per entry of the fine summary.
pub(in crate::app) const FINE_BLOCK: u64 = 256;
/// Frames per entry of the coarse summary.
pub(in crate::app) const COARSE_BLOCK: u64 = 65_536;

/// The extremes and mean square of a run of samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct Peak {
    pub(in crate::app) min: i16,
    pub(in crate::app) max: i16,
    /// Mean of the squared samples, in `i16` units squared.
    pub(in crate::app) mean_square: f32,
}

impl Peak {
    const EMPTY: Peak = Peak {
        min: i16::MAX,
        max: i16::MIN,
        mean_square: 0.0,
    };

    /// Root mean square, as a fraction of full scale.
    pub(in crate::app) fn rms(self) -> f32 {
        self.mean_square.sqrt() / 32768.0
    }
}

/// Folds peaks (or samples) together, weighting each mean square by the
/// frames it covers.
#[derive(Clone, Copy)]
struct Fold {
    min: i16,
    max: i16,
    square_sum: f64,
    frames: u64,
}

impl Fold {
    fn new() -> Self {
        Self {
            min: i16::MAX,
            max: i16::MIN,
            square_sum: 0.0,
            frames: 0,
        }
    }

    fn sample(&mut self, sample: i16) {
        self.min = self.min.min(sample);
        self.max = self.max.max(sample);
        self.square_sum += f64::from(sample) * f64::from(sample);
        self.frames += 1;
    }

    fn peak(&mut self, peak: Peak, frames: u64) {
        self.min = self.min.min(peak.min);
        self.max = self.max.max(peak.max);
        self.square_sum += f64::from(peak.mean_square) * frames as f64;
        self.frames += frames;
    }

    fn finish(self) -> Option<Peak> {
        (self.frames > 0).then(|| Peak {
            min: self.min,
            max: self.max,
            mean_square: (self.square_sum / self.frames as f64) as f32,
        })
    }
}

/// A decoded sound with its summaries. The audio is shared, not copied: this
/// is what the cache holds and what a voice plays.
pub(in crate::app) struct Waveform {
    pcm: Arc<DecodedPcm>,
    /// `fine[channel][block]`, one per [`FINE_BLOCK`] frames.
    fine: Vec<Vec<Peak>>,
    /// `coarse[channel][block]`, one per [`COARSE_BLOCK`] frames.
    coarse: Vec<Vec<Peak>>,
}

impl Waveform {
    /// Summarise `pcm`. One pass over the samples; run it off the UI thread
    /// for anything long.
    pub(in crate::app) fn new(pcm: Arc<DecodedPcm>) -> Self {
        let channels = pcm.channels as usize;
        let frames = pcm.frame_count();
        let mut fine = vec![Vec::with_capacity(frames.div_ceil(FINE_BLOCK as usize)); channels];
        for (channel, blocks) in fine.iter_mut().enumerate() {
            for block in 0..frames.div_ceil(FINE_BLOCK as usize) {
                let start = block * FINE_BLOCK as usize;
                let end = (start + FINE_BLOCK as usize).min(frames);
                let mut fold = Fold::new();
                for frame in start..end {
                    fold.sample(pcm.samples[frame * channels + channel]);
                }
                blocks.push(fold.finish().unwrap_or(Peak::EMPTY));
            }
        }
        let per_coarse = (COARSE_BLOCK / FINE_BLOCK) as usize;
        let coarse = fine
            .iter()
            .map(|blocks| {
                blocks
                    .chunks(per_coarse)
                    .enumerate()
                    .map(|(index, chunk)| {
                        let mut fold = Fold::new();
                        for (offset, peak) in chunk.iter().enumerate() {
                            let start = ((index * per_coarse + offset) as u64) * FINE_BLOCK;
                            let covered = FINE_BLOCK.min(frames as u64 - start);
                            fold.peak(*peak, covered);
                        }
                        fold.finish().unwrap_or(Peak::EMPTY)
                    })
                    .collect()
            })
            .collect();
        Self { pcm, fine, coarse }
    }

    pub(in crate::app) fn pcm(&self) -> &Arc<DecodedPcm> {
        &self.pcm
    }

    pub(in crate::app) fn channels(&self) -> u16 {
        self.pcm.channels
    }

    pub(in crate::app) fn frames(&self) -> u64 {
        self.pcm.frame_count() as u64
    }

    pub(in crate::app) fn duration_secs(&self) -> f64 {
        self.pcm.duration_secs() as f64
    }

    /// The memory it holds, for the cache's budget.
    pub(in crate::app) fn bytes(&self) -> usize {
        let peaks: usize = self.fine.iter().chain(&self.coarse).map(Vec::len).sum();
        self.pcm.samples.len() * std::mem::size_of::<i16>() + peaks * std::mem::size_of::<Peak>()
    }

    /// The peak of frames `start..end` of `channel`, from the samples when the
    /// run is short and from the coarsest summary that still gives a few
    /// entries otherwise. Summary blocks overlapping either end count whole.
    pub(in crate::app) fn peak(&self, channel: usize, start: u64, end: u64) -> Option<Peak> {
        let end = end.min(self.frames());
        if channel >= self.channels() as usize || start >= end {
            return None;
        }
        let span = end - start;
        let mut fold = Fold::new();
        if span < FINE_BLOCK * 2 {
            let channels = self.channels() as usize;
            for frame in start..end {
                fold.sample(self.pcm.samples[frame as usize * channels + channel]);
            }
        } else {
            let (level, block) = if span < COARSE_BLOCK * 2 {
                (&self.fine[channel], FINE_BLOCK)
            } else {
                (&self.coarse[channel], COARSE_BLOCK)
            };
            let first = (start / block) as usize;
            let last = (end.div_ceil(block) as usize).min(level.len());
            for (index, peak) in level[first..last].iter().enumerate() {
                let block_start = (first + index) as u64 * block;
                fold.peak(*peak, block.min(self.frames() - block_start));
            }
        }
        fold.finish()
    }
}

/// The usual label for each channel of a `channels`-channel layout, in WAVE
/// order (which FMOD, Wwise and the Halo encoders all keep).
pub(in crate::app) fn channel_labels(channels: u16) -> Vec<String> {
    let named: &[&str] = match channels {
        1 => &["M"],
        2 => &["L", "R"],
        3 => &["L", "R", "C"],
        4 => &["L", "R", "Ls", "Rs"],
        5 => &["L", "R", "C", "Ls", "Rs"],
        6 => &["L", "R", "C", "LFE", "Ls", "Rs"],
        8 => &["L", "R", "C", "LFE", "Ls", "Rs", "Lb", "Rb"],
        _ => &[],
    };
    if named.len() == channels as usize {
        named.iter().map(|label| (*label).to_owned()).collect()
    } else {
        (1..=channels).map(|channel| channel.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo(
        frames: usize,
        left: impl Fn(usize) -> i16,
        right: impl Fn(usize) -> i16,
    ) -> Waveform {
        Waveform::new(Arc::new(DecodedPcm {
            samples: (0..frames).flat_map(|f| [left(f), right(f)]).collect(),
            channels: 2,
            sample_rate: 48_000,
        }))
    }

    /// Every level answers what the samples say: a run's extremes, and its
    /// RMS, whichever level the span picks.
    #[test]
    fn each_level_agrees_with_the_samples() {
        // Left: a ramp; right: silence with one spike deep in the clip.
        let frames = 300_000;
        let wave = stereo(
            frames,
            |f| ((f % 2000) as i32 - 1000) as i16,
            |f| if f == 250_123 { -32000 } else { 0 },
        );
        for (start, end) in [(10, 200), (1000, 9000), (0, 300_000), (130_000, 299_999)] {
            for channel in 0..2 {
                let mut min = i16::MAX;
                let mut max = i16::MIN;
                for f in start..end {
                    let s = wave.pcm.samples[f * 2 + channel];
                    min = min.min(s);
                    max = max.max(s);
                }
                let peak = wave.peak(channel, start as u64, end as u64).unwrap();
                assert!(
                    peak.min <= min && peak.max >= max,
                    "{channel} {start}..{end}: {peak:?} vs {min}..{max}"
                );
            }
        }
        // The spike shows at the coarsest level, and nowhere it isn't.
        assert_eq!(wave.peak(1, 0, 300_000).unwrap().min, -32000);
        // Three whole coarse blocks end at 196,608, short of it. (A block
        // that overlaps the end counts whole, so 0..200,000 would include it.)
        assert_eq!(wave.peak(1, 0, 3 * COARSE_BLOCK).unwrap().min, 0);
        assert_eq!(wave.peak(1, 0, 200_000).unwrap().min, -32000);
        // A whole-clip RMS from summaries matches one from the samples.
        let exact = {
            let sum: f64 = (0..frames)
                .map(|f| {
                    let s = f64::from(wave.pcm.samples[f * 2]);
                    s * s
                })
                .sum();
            ((sum / frames as f64).sqrt() / 32768.0) as f32
        };
        let summarised = wave.peak(0, 0, frames as u64).unwrap().rms();
        assert!((exact - summarised).abs() < 1e-4, "{exact} vs {summarised}");
    }

    #[test]
    fn short_runs_read_the_samples_themselves() {
        let wave = stereo(1000, |f| f as i16, |_| 0);
        assert_eq!(
            wave.peak(0, 10, 13),
            Some(Peak {
                min: 10,
                max: 12,
                mean_square: (100.0 + 121.0 + 144.0) / 3.0
            })
        );
        assert_eq!(
            wave.peak(0, 999, 2000).unwrap().max,
            999,
            "the end is clamped"
        );
        assert_eq!(wave.peak(0, 1000, 1001), None);
        assert_eq!(wave.peak(2, 0, 10), None);
    }

    #[test]
    fn channels_are_labelled_in_wave_order() {
        assert_eq!(channel_labels(1), ["M"]);
        assert_eq!(channel_labels(6), ["L", "R", "C", "LFE", "Ls", "Rs"]);
        assert_eq!(channel_labels(7), ["1", "2", "3", "4", "5", "6", "7"]);
    }

    /// What the summaries cost to build, and what one frame's worth of pixel
    /// columns costs to read, on three minutes of 48 kHz stereo. Run with
    /// `cargo test --release waveform_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn waveform_cost() {
        let frames = 3 * 60 * 48_000;
        let pcm = Arc::new(DecodedPcm {
            samples: (0..frames * 2).map(|i| (i % 65_536) as i16).collect(),
            channels: 2,
            sample_rate: 48_000,
        });
        let start = std::time::Instant::now();
        let wave = Waveform::new(pcm);
        let built = start.elapsed();
        let columns = 2400; // a 1200-point timeline at 2x
        let start = std::time::Instant::now();
        let mut sink = 0i64;
        for channel in 0..2 {
            for column in 0..columns {
                let a = (column * frames / columns) as u64;
                let b = ((column + 1) * frames / columns) as u64;
                sink += i64::from(wave.peak(channel, a, b).unwrap().max);
            }
        }
        let frame = start.elapsed();
        eprintln!(
            "built in {built:?}; one frame of {columns} columns x 2 lanes in {frame:?}; {} KiB of summaries ({sink})",
            (wave.bytes() - frames * 4) / 1024
        );
    }
}

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
mod tests;

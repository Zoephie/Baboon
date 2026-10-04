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

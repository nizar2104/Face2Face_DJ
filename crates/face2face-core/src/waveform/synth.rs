//! Synthetic 3-band waveforms for Milestone 0, in the real wire format:
//! 150 frames per second, 3 bytes per frame (low, mid, high), plus a
//! 1,200-point overview. The shape imitates a dance track (kick on the beat,
//! 16-bar sections, a breakdown) so scrolling and tempo are easy to judge by eye.
//! The TypeScript twin is `tools/mock-sender/synth.ts`.

pub const DETAIL_FPS: u32 = 150;
pub const BYTES_PER_FRAME: usize = 3;
pub const OVERVIEW_POINTS: usize = 1200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waveform {
    pub frames: u32,
    /// `frames * 3` bytes: low, mid, high per frame.
    pub detail: Vec<u8>,
    /// `OVERVIEW_POINTS * 3` bytes.
    pub overview: Vec<u8>,
}

/// FNV-1a, 32 bit, over the UTF-8 bytes. Used for mock ids and seeds.
pub fn fnv1a32(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// The hash that track and waveform ids are built from until audio files are
/// hashed (Milestone 2). The mock seeds its waveform with it too.
pub fn track_hash(artist: &str, title: &str) -> u32 {
    fnv1a32(&format!("{artist}\0{title}"))
}

/// `[kick, mid, high]` gain per 16-bar section: intro, build, drop, break, drop, outro.
const SECTIONS: [[f64; 3]; 6] = [
    [0.8, 0.25, 0.3],
    [0.85, 0.5, 0.6],
    [1.0, 0.9, 0.8],
    [0.0, 0.7, 0.35],
    [1.0, 1.0, 0.9],
    [0.8, 0.3, 0.4],
];

pub fn synth_waveform(duration_s: f64, bpm: f64, seed: u32) -> Waveform {
    let fps = f64::from(DETAIL_FPS);
    // A float-to-integer `as` cast saturates, so odd durations cannot overflow.
    let frames = ((duration_s * fps).round() as usize).max(1);
    let beat_len = 60.0 / if bpm > 0.0 { bpm } else { 120.0 };
    let mut values = vec![0f32; frames * BYTES_PER_FRAME];
    let mut rng: u32 = if seed != 0 { seed } else { 0x9e37_79b9 }; // xorshift32 must not start at 0
    let mut peak = 0f32;

    for i in 0..frames {
        let beat = i as f64 / fps / beat_len;
        let [kick_gain, mid_gain, high_gain] = SECTIONS[(beat / 64.0) as usize % SECTIONS.len()];

        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let noise = f64::from(rng) / 4_294_967_296.0;

        let kick = (-beat.fract() * 7.0).exp();
        let bass = (-(beat + 0.5).fract() * 5.0).exp();
        let hat = (-(beat * 4.0).fract() * 9.0).exp();

        let low = kick_gain * kick + 0.15 * mid_gain * bass;
        let mid = mid_gain * (0.55 * bass + 0.25 + 0.2 * noise);
        let high = high_gain * (0.6 * hat + 0.4 * noise);

        for (band, value) in [low, mid, high].into_iter().enumerate() {
            let value = value as f32;
            values[i * BYTES_PER_FRAME + band] = value;
            if value > peak {
                peak = value;
            }
        }
    }

    // Normalise so the loudest band value in the track is 255 (brief 7.5).
    let scale = if peak > 0.0 { 255.0 / peak } else { 0.0 };
    let detail: Vec<u8> = values.iter().map(|v| (v * scale).round() as u8).collect();
    let overview = build_overview(&detail);
    Waveform { frames: frames as u32, detail, overview }
}

/// Overview point `j` holds the per-band maximum of the detail frames it covers.
pub fn build_overview(detail: &[u8]) -> Vec<u8> {
    let frames = detail.len() / BYTES_PER_FRAME;
    let mut overview = vec![0u8; OVERVIEW_POINTS * BYTES_PER_FRAME];
    if frames == 0 {
        return overview;
    }
    for point in 0..OVERVIEW_POINTS {
        let start = (point * frames / OVERVIEW_POINTS).min(frames - 1);
        let end = ((point + 1) * frames / OVERVIEW_POINTS).max(start + 1).min(frames);
        for band in 0..BYTES_PER_FRAME {
            let mut max = 0u8;
            for frame in start..end {
                max = max.max(detail[frame * BYTES_PER_FRAME + band]);
            }
            overview[point * BYTES_PER_FRAME + band] = max;
        }
    }
    overview
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a32_matches_the_published_test_vectors() {
        assert_eq!(fnv1a32(""), 0x811c_9dc5);
        assert_eq!(fnv1a32("a"), 0xe40c_292c);
        assert_eq!(fnv1a32("foobar"), 0xbf9c_f968);
    }

    #[test]
    fn has_the_wire_format_peaks_at_255_and_is_repeatable() {
        let wf = synth_waveform(372.4, 124.0, 42);
        assert_eq!(wf.frames, 55_860);
        assert_eq!(wf.detail.len(), 55_860 * 3);
        assert_eq!(wf.overview.len(), 3_600);
        assert_eq!(wf.detail.iter().copied().max(), Some(255));
        assert_eq!(synth_waveform(372.4, 124.0, 42), wf);
        assert_ne!(synth_waveform(372.4, 124.0, 43).detail, wf.detail);
    }

    #[test]
    fn has_a_kick_on_every_beat_outside_the_breakdown() {
        // 120 BPM = 75 frames per beat at 150 fps. The first 100 beats are intro and build.
        let wf = synth_waveform(60.0, 120.0, 1);
        for beat in 0..100 {
            let on_beat = wf.detail[beat * 75 * 3];
            let off_beat = wf.detail[(beat * 75 + 60) * 3];
            assert!(on_beat > 150 && off_beat < on_beat / 2, "beat {beat}: {on_beat} vs {off_beat}");
        }
    }

    #[test]
    fn overview_holds_the_per_band_maximum() {
        let frames = 2400; // two frames per overview point
        let mut detail = vec![0u8; frames * 3];
        detail[0] = 10; // frame 0, low
        detail[3] = 200; // frame 1, low -> point 0 low = 200
        detail[2399 * 3 + 2] = 77; // last frame, high -> last point high = 77
        let overview = build_overview(&detail);
        assert_eq!(overview[0], 200);
        assert_eq!(overview[1199 * 3 + 2], 77);

        // Shorter than the overview: every point still maps to a real frame.
        let tiny = build_overview(&[5, 6, 7]);
        assert_eq!(&tiny[..3], &[5, 6, 7]);
        assert_eq!(&tiny[3597..], &[5, 6, 7]);
        assert_eq!(build_overview(&[]), vec![0u8; 3600]);
    }

    #[test]
    fn odd_inputs_do_not_panic() {
        for (duration, bpm) in [(0.0, 120.0), (-5.0, 120.0), (f64::NAN, 120.0), (1.0, 0.0), (1.0, f64::NAN)] {
            let wf = synth_waveform(duration, bpm, 0);
            assert!(wf.frames >= 1);
            assert_eq!(wf.detail.len(), wf.frames as usize * 3);
        }
    }
}

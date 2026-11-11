//! Waveforms. Milestone 0 only synthesises them; decoding, 3-band analysis and
//! the on-disk cache arrive in Milestone 2 and will produce the same format.

pub mod synth;

pub use synth::{Waveform, BYTES_PER_FRAME, DETAIL_FPS, OVERVIEW_POINTS};

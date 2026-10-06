//! Lock-free, lossy audio-to-editor telemetry. An effect has no developer MIDI channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub struct Telemetry {
    peak: AtomicU32,
    wet: AtomicU32,
    clipped: AtomicBool,
    /// The host tempo in force, so a synced pre-delay or mod rate reads its division.
    pub tempo: mxm_tempo::TempoCell,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            peak: AtomicU32::new(0),
            wet: AtomicU32::new(0),
            clipped: AtomicBool::new(false),
            tempo: mxm_tempo::TempoCell::new(),
        }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    fn publish_max(slot: &AtomicU32, value: f32) {
        let value = value.abs();
        let bits = value.to_bits();
        let mut current = slot.load(Ordering::Relaxed);
        while f32::from_bits(current) < value {
            match slot.compare_exchange_weak(current, bits, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(seen) => current = seen,
            }
        }
    }

    /// Once per block: the output's peak, and the largest distance between output and input — the
    /// wet the reverb actually added.
    pub fn publish(&self, peak: f32, wet: f32) {
        Self::publish_max(&self.peak, peak);
        Self::publish_max(&self.wet, wet);
        if peak >= 1.0 {
            self.clipped.store(true, Ordering::Relaxed);
        }
    }

    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }

    pub fn take_wet(&self) -> f32 {
        f32::from_bits(self.wet.swap(0, Ordering::Relaxed))
    }

    pub fn clipped(&self) -> bool {
        self.clipped.load(Ordering::Relaxed)
    }

    pub fn clear_clip(&self) {
        self.clipped.store(false, Ordering::Relaxed);
    }
}

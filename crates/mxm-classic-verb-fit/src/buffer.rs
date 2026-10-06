//! **Every buffer the response's length sizes is reserved here, and fallibly.**
//!
//! A fit runs inside a plugin, on the host's background thread, and an allocation that fails the
//! ordinary way aborts the whole process — the host and every other plugin in it. The input domain
//! bounds a response at thirty seconds of two channels at 192 kHz, which is 46 MB of `f32` and twice
//! that for each `f64` copy the analysis takes, so a reservation that cannot be made is
//! [`OutOfMemory`] — [`Refusal::OutOfMemory`] from the analyser and `FitRefusal::OutOfMemory` from the
//! fit, named like every other refusal — and the caller keeps what it had.
//!
//! **What comes through here**: the combined energy, each band's filtered copy and energy, each decay
//! curve, every render and its mono fold, the search's early and combined excerpts, and the
//! verification's scaled copy. **What does not**, because a constant bounds it rather than the
//! response's length: the tail texture's segments (at most 1.4 s), the echo density profile and the
//! early envelope (0.5 s at one point a millisecond), Lundeby's block levels (one per 10 ms or
//! longer), the early reflections (peaks within the early window, at most 24 kept), the width match's
//! mid, side and trial channels (its render is 1.08 s after the direct sound at most, and is reserved
//! here like every render) and anything counted in bands, taps or candidates.

use crate::refusal::Refusal;

/// A reservation that could not be made, and how many bytes it asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutOfMemory {
    pub bytes: usize,
}

impl From<OutOfMemory> for Refusal {
    fn from(error: OutOfMemory) -> Self {
        Refusal::OutOfMemory { bytes: error.bytes }
    }
}

/// `len` copies of `value`.
#[cfg_attr(test, track_caller)]
pub(crate) fn filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>, OutOfMemory> {
    let mut buffer = Vec::new();
    reserve(&mut buffer, len)?;
    buffer.resize(len, value);
    Ok(buffer)
}

/// `items`, `len` of them, into a buffer reserved for that many before the first is taken.
#[cfg_attr(test, track_caller)]
pub(crate) fn collected<T>(
    len: usize,
    items: impl IntoIterator<Item = T>,
) -> Result<Vec<T>, OutOfMemory> {
    let mut buffer = Vec::new();
    reserve(&mut buffer, len)?;
    buffer.extend(items);
    Ok(buffer)
}

/// The seam: exactly `len` elements, or the refusal naming how much could not be had.
#[cfg_attr(test, track_caller)]
fn reserve<T>(buffer: &mut Vec<T>, len: usize) -> Result<(), OutOfMemory> {
    let bytes = len.saturating_mul(core::mem::size_of::<T>());
    #[cfg(test)]
    if seam::refuses(std::panic::Location::caller()) {
        return Err(OutOfMemory { bytes });
    }
    buffer
        .try_reserve_exact(len)
        .map_err(|_| OutOfMemory { bytes })
}

/// **Test builds only**: which reservations a call asked for, and a refusal injected at one of them.
///
/// Thread-local, because the harness runs tests in parallel in one process and a fit runs on the
/// thread that called it.
#[cfg(test)]
pub(crate) mod seam {
    use std::cell::RefCell;
    use std::panic::Location;

    #[derive(Default)]
    struct Watch {
        sites: Vec<&'static Location<'static>>,
        refuse: Option<usize>,
    }

    thread_local! {
        static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    }

    /// Runs `body`, refusing the reservation numbered `refuse` (from zero, in the order they are asked
    /// for) if one is given, and returns what it returned with the call site of every reservation it
    /// asked for, the refused one included.
    pub(crate) fn watching<R>(
        refuse: Option<usize>,
        body: impl FnOnce() -> R,
    ) -> (R, Vec<&'static Location<'static>>) {
        WATCH.with(|watch| {
            *watch.borrow_mut() = Some(Watch {
                refuse,
                ..Watch::default()
            })
        });
        let result = body();
        let sites = WATCH
            .with(|watch| watch.borrow_mut().take())
            .map(|watch| watch.sites)
            .unwrap_or_default();
        (result, sites)
    }

    pub(super) fn refuses(site: &'static Location<'static>) -> bool {
        WATCH.with(|watch| {
            watch.borrow_mut().as_mut().is_some_and(|watch| {
                watch.sites.push(site);
                watch.refuse == Some(watch.sites.len() - 1)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::seam;
    use crate::{FitRefusal, Refusal};

    const RATE: u32 = 24_000;

    /// 50 ms of noise floor 90 dB under a unit direct impulse, reflections 7, 13 and 19 ms after it,
    /// and white noise from 4 ms falling 60 dB in half a second: the plugin's planted response, which
    /// its own loading test fits.
    fn planted() -> Vec<f32> {
        let fs = f64::from(RATE);
        let frames = (1.2 * fs) as usize;
        let direct = (0.05 * fs) as usize;
        let tail = direct + (0.004 * fs) as usize;
        let mut state = 12u64.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 52) as f64 - 1.0
        };
        let mut out: Vec<f32> = (0..frames)
            .map(|n| {
                let floor = 3.0e-5 * noise();
                let late = if n >= tail {
                    0.1 * 10f64.powf(-3.0 * (n - tail) as f64 / fs / 0.5) * noise()
                } else {
                    0.0
                };
                (floor + late) as f32
            })
            .collect();
        out[direct] += 1.0;
        for (delay_s, gain) in [(0.007, 0.5), (0.013, 0.35), (0.019, 0.25)] {
            out[direct + (delay_s * fs) as usize] += gain;
        }
        out
    }

    /// **Every reservation a fit asks for can be refused, and the fit then refuses by name.** A
    /// reservation that failed the ordinary way would abort the plugin's host; one that is refused
    /// here must come back as [`FitRefusal::OutOfMemory`] from whichever stage asked for it, not as a
    /// panic, a partial fit or a refusal of the response. Each call site is refused at its first
    /// reservation and at its last — the response's analysis and the search first, the verification's
    /// render and analysis last — and the sites are held to the stages the crate's documentation
    /// names, so a buffer moved off the seam is noticed.
    #[test]
    fn every_reservation_a_fit_asks_for_can_be_refused_by_name() {
        let response = planted();
        let channels = [response.as_slice()];
        let rate = RATE as f32;
        let (fitted, sites) = seam::watching(None, || crate::fit(&channels, rate));
        assert!(
            fitted.is_ok(),
            "the planted response did not fit: {fitted:?}"
        );

        let mut files: Vec<&str> = sites.iter().map(|site| site.file()).collect();
        files.sort_unstable();
        files.dedup();
        let files: Vec<&str> = files
            .iter()
            .map(|file| file.rsplit(['/', '\\']).next().unwrap_or(file))
            .collect();
        assert_eq!(
            files,
            [
                "band.rs",
                "decay.rs",
                "early.rs",
                "fit.rs",
                "lib.rs",
                "render.rs",
                "search.rs"
            ],
            "the reservations a fit asks for no longer come from the stages the seam names"
        );

        let key = |site: &&'static std::panic::Location<'static>| {
            (site.file(), site.line(), site.column())
        };
        let mut refused: Vec<usize> = Vec::new();
        for (index, site) in sites.iter().enumerate() {
            let first = sites.iter().position(|other| key(other) == key(site)) == Some(index);
            let last = sites.iter().rposition(|other| key(other) == key(site)) == Some(index);
            if first || last {
                refused.push(index);
            }
        }
        for index in refused {
            let (outcome, _) = seam::watching(Some(index), || crate::fit(&channels, rate));
            assert!(
                matches!(outcome, Err(FitRefusal::OutOfMemory { .. })),
                "refusing reservation {index} of {}, at {}, gave {outcome:?}",
                sites.len(),
                sites[index]
            );
        }

        let (analysed, _) = seam::watching(Some(0), || crate::analyse(&channels, rate));
        assert_eq!(
            analysed.err(),
            Some(Refusal::OutOfMemory {
                bytes: response.len() * core::mem::size_of::<f64>()
            }),
            "the analyser's first buffer is the combined energy"
        );
    }
}

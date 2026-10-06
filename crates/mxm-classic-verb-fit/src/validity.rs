//! The validity checks of `plans/plan-mxm-classic-verb.md` §4.5 and the confidence they combine
//! into.
//!
//! Each check maps a measured value onto a score from zero to one by a straight ramp between two
//! stated values. **The confidence is the lowest score**, and the check that set it is named: a
//! response is only as trustworthy as its weakest reading, and an average would let two good bands
//! hide a bad one.

use crate::analysis::{Band, Check, CheckKind, Validity};
use crate::math::finite_f32;

fn ramp(value: f64, zero_at: f64, full_at: f64) -> f64 {
    ((value - zero_at) / (full_at - zero_at)).clamp(0.0, 1.0)
}

/// `deviations` holds each band's straightness statistic from the noise-compensated decay, aligned
/// with `bands`.
pub(crate) fn assess(
    prominence_db: Option<f64>,
    bands: &[Band],
    deviations: &[Option<f32>],
) -> Validity {
    let mut checks = vec![Check {
        kind: CheckKind::OnsetProminence,
        value_db: prominence_db.and_then(finite_f32),
        score: prominence_db
            .map(|p| ramp(p, crate::ONSET_BELOW_PEAK_DB, crate::PROMINENCE_FULL_DB) as f32),
    }];
    for (band, &deviation) in bands
        .iter()
        .zip(deviations)
        .filter(|(band, _)| band.required)
    {
        let Some(decay) = band.decay() else { continue };
        let band_hz = band.centre_hz;
        checks.push(Check {
            kind: CheckKind::NoiseMargin { band_hz },
            value_db: Some(decay.margin_db),
            score: Some(ramp(
                f64::from(decay.margin_db),
                crate::MARGIN_ZERO_DB,
                crate::MARGIN_FULL_DB,
            ) as f32),
        });
        checks.push(Check {
            kind: CheckKind::Straightness { band_hz },
            value_db: deviation,
            score: deviation.map(|d| {
                ramp(
                    f64::from(d),
                    crate::STRAIGHTNESS_ZERO_DB,
                    crate::STRAIGHTNESS_FULL_DB,
                ) as f32
            }),
        });
    }

    let mut confidence = 1.0f32;
    let mut weakest = None;
    for check in &checks {
        if let Some(score) = check.score {
            if weakest.is_none() || score < confidence {
                confidence = score;
                weakest = Some(check.kind);
            }
        }
    }
    Validity {
        checks,
        confidence,
        weakest,
        floor: crate::CONFIDENCE_FLOOR as f32,
    }
}

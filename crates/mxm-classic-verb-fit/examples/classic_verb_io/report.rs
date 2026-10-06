//! The fit report as JSON, written by `classic_verb_fit` and `classic_verb_generate` alike: the fitted
//! space, the four controls, the target descriptors, the verification's descriptors and the error for
//! every one. **A report names its source only by the logical name it is given** — never a path.

use mxm_classic_verb_dsp::{EarlyTap, SPACE_VERSION, Space};
use mxm_classic_verb_fit::{
    Analysis, BandReading, Fit, TailTexture, TextureErrors, Width, WidthReport,
};

use super::Json;

/// A tail texture with the segment it was read on, or `null` where there is none.
fn texture(t: &Option<TailTexture>) -> Json {
    t.as_ref().map_or(Json::Null, |t| {
        Json::object(vec![
            ("start_s", Json::num(t.segment.start_s)),
            ("end_s", Json::num(t.segment.end_s)),
            ("peakiness", Json::opt(t.peakiness)),
            ("kurtosis", Json::opt(t.kurtosis)),
            ("echo_density", Json::opt(t.echo_density)),
            ("periodicity", Json::opt(t.periodicity)),
            ("periodicity_lag_s", Json::opt(t.periodicity_lag_s)),
        ])
    })
}

fn texture_errors(e: &TextureErrors) -> Json {
    Json::object(vec![
        ("peakiness", Json::opt(e.peakiness)),
        ("kurtosis", Json::opt(e.kurtosis)),
        ("echo_density", Json::opt(e.echo_density)),
        ("periodicity", Json::opt(e.periodicity)),
    ])
}

fn tap(t: &EarlyTap) -> Json {
    Json::object(vec![
        ("time_units", Json::num(t.time)),
        ("gain_l", Json::num(t.gain_l)),
        ("gain_r", Json::num(t.gain_r)),
    ])
}

pub fn space(s: &Space) -> Json {
    Json::object(vec![
        ("version", Json::Number(f64::from(SPACE_VERSION))),
        ("early", Json::Array(s.early.iter().map(tap).collect())),
        ("early_level", Json::num(s.early_level)),
        ("decay_ratio_low", Json::num(s.decay_ratio_low)),
        ("decay_ratio_high", Json::num(s.decay_ratio_high)),
        ("decay_ratio_top", Json::num(s.decay_ratio_top)),
        ("tone_low_db", Json::num(s.tone_low_db)),
        ("tone_high_db", Json::num(s.tone_high_db)),
        ("high_cut_hz", Json::num(s.high_cut_hz)),
        ("width", Json::num(s.width)),
    ])
}

pub fn analysis(a: &Analysis) -> Json {
    let bands = a
        .bands
        .iter()
        .map(|b| match &b.reading {
            BandReading::Measured(d) => Json::object(vec![
                ("centre_hz", Json::num(b.centre_hz)),
                ("required", Json::Bool(b.required)),
                ("t20_s", Json::opt(d.t20_s)),
                ("t30_s", Json::opt(d.t30_s)),
                ("edt_s", Json::opt(d.edt_s)),
                ("noise_floor_db", Json::num(d.noise_floor_db)),
                ("margin_db", Json::num(d.margin_db)),
                ("level_re_1k_db", Json::opt(d.level_re_1k_db)),
                (
                    "max_deviation_db",
                    Json::opt(d.straightness.map(|s| s.max_deviation_db)),
                ),
                (
                    "single_slope",
                    d.straightness
                        .map_or(Json::Null, |s| Json::Bool(s.single_slope)),
                ),
            ]),
            other => Json::object(vec![
                ("centre_hz", Json::num(b.centre_hz)),
                ("reading", Json::text(format!("{other:?}"))),
            ]),
        })
        .collect();
    let width = match &a.width {
        Width::Measured(c) => Json::object(vec![
            ("iacc", Json::num(c.iacc)),
            ("peak_iacf", Json::num(c.peak_iacf)),
            ("window_end_s", Json::num(c.window_end_s)),
        ]),
        Width::NotMeasured(why) => Json::text(format!("not measured: {why:?}")),
    };
    Json::object(vec![
        ("confidence", Json::num(a.validity.confidence)),
        (
            "weakest_check",
            a.validity
                .weakest
                .map_or(Json::Null, |w| Json::text(format!("{w:?}"))),
        ),
        ("direct_s", Json::num(a.onset.direct_s)),
        ("prominence_db", Json::opt(a.onset.prominence_db)),
        ("drr_db", Json::opt(a.onset.drr_db)),
        ("pre_delay_s", Json::opt(a.pre_delay_s)),
        ("mixing_time_s", Json::opt(a.echo_density.mixing_time_s)),
        ("bands", Json::Array(bands)),
        ("width", width),
        ("texture_a", texture(&a.texture_a)),
        ("texture_b", texture(&a.texture_b)),
        (
            "reflections",
            Json::Array(
                a.early
                    .reflections
                    .iter()
                    .map(|r| {
                        Json::object(vec![
                            ("delay_s", Json::num(r.delay_s)),
                            ("level_db", Json::num(r.level_db)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "echo_density",
            Json::Array(
                a.echo_density
                    .points
                    .iter()
                    .map(|p| Json::opt(p.density))
                    .collect(),
            ),
        ),
    ])
}

pub fn report(name: &str, sample_rate: u32, f: &Fit) -> Json {
    let r = &f.report;
    let e = &r.errors;
    let width = match r.width {
        WidthReport::Matched {
            target_peak_iacf,
            rendered_peak_iacf,
        } => Json::object(vec![
            ("target_peak_iacf", Json::num(target_peak_iacf)),
            ("rendered_peak_iacf", Json::num(rendered_peak_iacf)),
        ]),
        WidthReport::NotMeasured(why) => Json::text(format!("not measured: {why:?}")),
    };
    Json::object(vec![
        ("source", Json::text(name)),
        ("sample_rate", Json::Number(f64::from(sample_rate))),
        ("space", space(&f.space)),
        (
            "controls",
            Json::object(vec![
                ("decay_s", Json::num(f.controls.decay_s)),
                ("size_s", Json::num(f.controls.size_s)),
                ("diffusion", Json::num(f.controls.diffusion)),
                ("pre_delay_s", Json::num(f.controls.pre_delay_s)),
            ]),
        ),
        (
            "contract",
            Json::text(
                "relative controls neutral, modulation depth zero, decay shape natural; mix, ducking and modulation rate untouched",
            ),
        ),
        (
            "clamps",
            Json::Array(
                r.clamps
                    .iter()
                    .map(|c| {
                        Json::object(vec![
                            ("value", Json::text(format!("{:?}", c.value))),
                            ("fitted", Json::num(c.fitted)),
                            ("applied", Json::num(c.applied)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "errors",
            Json::object(vec![
                (
                    "bands",
                    Json::Array(
                        e.bands
                            .iter()
                            .map(|b| {
                                Json::object(vec![
                                    ("centre_hz", Json::num(b.centre_hz)),
                                    ("t30_percent", Json::opt(b.t30_percent)),
                                    ("t20_percent", Json::opt(b.t20_percent)),
                                    ("tone_db", Json::opt(b.tone_db)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("pre_delay_s", Json::opt(e.pre_delay_s)),
                ("mixing_time_s", Json::opt(e.mixing_time_s)),
                ("profile_distance", Json::opt(e.profile_distance)),
                ("envelope_db", Json::opt(e.envelope_db)),
                ("iacc", Json::opt(e.iacc)),
                ("early_to_late_db", Json::opt(e.early_to_late_db)),
                ("reflection_time_rms_s", Json::opt(e.reflection_time_rms_s)),
                (
                    "reflection_level_rms_db",
                    Json::opt(e.reflection_level_rms_db),
                ),
                (
                    "reflections_matched",
                    Json::Number(e.reflections_matched as f64),
                ),
                (
                    "reflections_compared",
                    Json::Number(e.reflections_compared as f64),
                ),
                ("texture_a", texture_errors(&e.texture_a)),
                ("texture_b", texture_errors(&e.texture_b)),
                ("target_confidence", Json::num(e.target_confidence)),
                (
                    "verification_confidence",
                    Json::num(e.verification_confidence),
                ),
            ]),
        ),
        (
            "fit",
            Json::object(vec![
                (
                    "decay_bands",
                    Json::Array(
                        r.decay
                            .bands
                            .iter()
                            .map(|b| {
                                Json::object(vec![
                                    ("centre_hz", Json::num(b.centre_hz)),
                                    ("target_s", Json::num(b.target_s)),
                                    ("weight", Json::num(b.weight)),
                                    ("predicted_s", Json::num(b.predicted_s)),
                                    ("modelled_s", Json::num(b.modelled_s)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("tone_residual_db", Json::opt(r.tone.residual_db)),
                ("tone_open_residual_db", Json::opt(r.tone.open_residual_db)),
                (
                    "first_arrival",
                    Json::text(format!("{:?}", r.early.first_arrival)),
                ),
                ("taps", Json::Number(r.early.taps as f64)),
                (
                    "reflections_beyond_reach",
                    Json::Number(r.early.beyond_reach as f64),
                ),
                (
                    "target_early_to_late_db",
                    Json::opt(r.early.target_early_to_late_db),
                ),
                ("width", width),
                (
                    "search_evaluations",
                    Json::Number(f64::from(r.search.evaluations)),
                ),
                (
                    "search_profile_distance",
                    Json::opt(r.search.profile_distance),
                ),
                (
                    "search_envelope_distance_db",
                    Json::opt(r.search.envelope_distance_db),
                ),
                ("wet_gain", Json::num(r.wet_gain)),
            ]),
        ),
        (
            "verification_refusal",
            r.verification_refusal
                .as_ref()
                .map_or(Json::Null, |refusal| Json::text(refusal.to_string())),
        ),
        ("target", analysis(&r.target)),
        ("verification", analysis(&r.verification)),
    ])
}

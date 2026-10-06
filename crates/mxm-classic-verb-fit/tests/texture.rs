//! The tail texture (`AGENTS.md`, *The tail texture*): planted tails whose texture is known because it
//! was put there — decaying Gaussian noise, a sparse comb of modes, sparse clicks and a repeating burst.
//!
//! **Where the tolerances come from.** `measure_the_texture_populations` (ignored; run it in release)
//! reads every plant over a population of realisations — seeds, sample rates, mono and stereo, decay
//! times — and prints each reading's range per segment. Each assertion quotes the population's figure,
//! not the seeds asserted, so a changed seed is not a changed claim. The planted generator is
//! `tests/support`'s own, independent of the analyser. Nothing here reads a file.

mod support;

use mxm_classic_verb_fit::{
    Analysis, Segment, TailTexture, TextureSegments, analyse, analyse_with_segments,
};
use support::{TexturePlan, TextureTail, assert_finite};

fn analysed(plan: &TexturePlan) -> Analysis {
    let channels = plan.build();
    let slices: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    let analysis = analyse(&slices, plan.rate as f32)
        .unwrap_or_else(|refusal| panic!("{plan:?} is analysed: {refusal}"));
    assert_finite(&analysis);
    analysis
}

fn segments(analysis: &Analysis) -> Vec<(&'static str, &TailTexture)> {
    [("A", &analysis.texture_a), ("B", &analysis.texture_b)]
        .into_iter()
        .filter_map(|(name, texture)| texture.as_ref().map(|t| (name, t)))
        .collect()
}

fn reading(value: Option<f32>, what: &str, plan: &TexturePlan, segment: &str) -> f32 {
    value.unwrap_or_else(|| panic!("{plan:?} segment {segment} has a {what}"))
}

const NOISE_T60_S: [f64; 3] = [0.6, 1.5, 3.0];
const RATES: [f64; 2] = [44_100.0, 48_000.0];

#[test]
fn decaying_noise_reads_as_a_diffuse_field() {
    // Population: 96 realisations at T60 0.6, 1.5 and 3 s. Segment A in all 96; B in the 64 at 1.5 and
    // 3 s, the 0.6 s tail reaching its floor within 150 ms of A's end. Over both segments: peakiness
    // 1.798–2.014, kurtosis 2.912–3.073, echo density 0.977–1.015, periodicity 0.064–0.203.
    for (rate, channels, t60_s, seed) in [
        (48_000.0, 2, 0.6, 1),
        (44_100.0, 1, 1.5, 2),
        (48_000.0, 1, 3.0, 3),
    ] {
        let plan = TexturePlan::new(rate, channels, t60_s, TextureTail::Noise, seed);
        let analysis = analysed(&plan);
        assert!(analysis.texture_a.is_some(), "{plan:?}: A is read");
        assert_eq!(
            analysis.texture_b.is_some(),
            t60_s > 1.0,
            "{plan:?}: B is read where the tail outlasts it"
        );
        for (segment, t) in segments(&analysis) {
            let peakiness = reading(t.peakiness, "peakiness", &plan, segment);
            assert!(
                (1.65..2.2).contains(&peakiness),
                "{plan:?} {segment}: peakiness {peakiness}"
            );
            let kurtosis = reading(t.kurtosis, "kurtosis", &plan, segment);
            assert!(
                (2.8..3.2).contains(&kurtosis),
                "{plan:?} {segment}: kurtosis {kurtosis}"
            );
            let density = reading(t.echo_density, "echo density", &plan, segment);
            assert!(
                (0.95..1.05).contains(&density),
                "{plan:?} {segment}: echo density {density}"
            );
            let periodicity = reading(t.periodicity, "periodicity", &plan, segment);
            assert!(
                periodicity < 0.3,
                "{plan:?} {segment}: periodicity {periodicity}"
            );
        }
    }
}

#[test]
fn segments_are_placed_by_the_mid_band_decay_and_cut_to_powers_of_two() {
    // A 3 s decay puts A at its bounds' ends — 100 ms after the origin, 600 ms long — and leaves B its
    // whole 800 ms, so the frames are known exactly: at 48 kHz, A from 3 + 100 ms for 28,800 frames
    // cut to 16,384, and B from 3 + 700 ms for 38,400 cut to 32,768.
    let plan = TexturePlan::new(48_000.0, 1, 3.0, TextureTail::Noise, 5);
    let analysis = analysed(&plan);
    let frames = |seconds: f32| (f64::from(seconds) * 48_000.0).round() as usize;
    let a = analysis.texture_a.expect("A").segment;
    assert_eq!(
        (frames(a.start_s), frames(a.end_s)),
        (4_944, 4_944 + 16_384)
    );
    let b = analysis.texture_b.expect("B").segment;
    assert_eq!(
        (frames(b.start_s), frames(b.end_s)),
        (33_744, 33_744 + 32_768)
    );

    // Stated segments are read as stated — shortened by the same rule, which leaves a power of two as it
    // is — and an absent one reads nothing.
    let channels = plan.build();
    let slices: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    let seconds = |frames: u32| (f64::from(frames) / 48_000.0) as f32;
    let stated = Segment {
        start_s: seconds(12_000),
        end_s: seconds(12_000 + 4_096),
    };
    let longer = Segment {
        end_s: seconds(12_000 + 4_800),
        ..stated
    };
    let other = analyse_with_segments(
        &slices,
        48_000.0,
        &TextureSegments {
            a: Some(stated),
            b: None,
        },
    )
    .expect("analysed");
    assert_eq!(other.texture_a.expect("A as stated").segment, stated);
    assert_eq!(other.texture_b, None);
    let shortened = analyse_with_segments(
        &slices,
        48_000.0,
        &TextureSegments {
            a: None,
            b: Some(longer),
        },
    )
    .expect("analysed");
    assert_eq!(shortened.texture_a, None);
    assert_eq!(shortened.texture_b, other.texture_a);
    // Everything but the texture is the same analysis.
    assert_eq!(
        Analysis {
            texture_a: analysis.texture_a,
            texture_b: analysis.texture_b,
            ..other.clone()
        },
        analysis
    );
    assert_eq!(
        analyse_with_segments(&slices, 48_000.0, &analysis.texture_segments()).expect("analysed"),
        analysis,
        "an analysis read again on its own segments is itself"
    );
}

#[test]
fn a_sparse_comb_of_modes_reads_far_peakier_than_noise() {
    // Population: 96 realisations at T60 0.8, 1.2 and 2 s. Segment A (341–372 ms) read 4.817–5.572 in
    // every one, against noise's worst 2.014. **B's reading follows its length**, because a short Hann
    // window smears modes 30 Hz apart into one another: 85–93 ms read 1.411–1.547, under noise, and
    // 171–186 ms 2.430–2.799; 683–743 ms read 9.396–10.925. So B is held to the comb only when long.
    for (rate, channels, t60_s, seed) in [(48_000.0, 2, 2.0, 1), (44_100.0, 1, 1.2, 2)] {
        let plan = TexturePlan::new(
            rate,
            channels,
            t60_s,
            TextureTail::Modes { spacing_hz: 30.0 },
            seed,
        );
        let analysis = analysed(&plan);
        for (segment, t) in segments(&analysis) {
            let peakiness = reading(t.peakiness, "peakiness", &plan, segment);
            if segment == "A" || t.segment.end_s - t.segment.start_s > 0.6 {
                assert!(peakiness > 4.0, "{plan:?} {segment}: peakiness {peakiness}");
            }
        }
        if t60_s == 2.0 {
            let b = analysis.texture_b.expect("a 2 s comb has a long B");
            assert!(b.segment.end_s - b.segment.start_s > 0.6, "{b:?}");
        }
    }
}

#[test]
fn sparse_clicks_read_heavy_tailed_and_sparse() {
    // Population: 85 of 96 realisations at T60 0.8, 1.2 and 2 s analysed — the analyser refused 11 for
    // a band's decay or its margin, which a click train leaves ragged; these two are analysed. Over both
    // segments: kurtosis 517–1,111 against noise's worst 3.073, echo density 0.060–0.567 against
    // noise's lowest 0.977.
    for (rate, channels, seed) in [(48_000.0, 2, 1), (44_100.0, 1, 2)] {
        let plan = TexturePlan::new(
            rate,
            channels,
            1.2,
            TextureTail::Clicks { per_second: 50.0 },
            seed,
        );
        let analysis = analysed(&plan);
        for (segment, t) in segments(&analysis) {
            let kurtosis = reading(t.kurtosis, "kurtosis", &plan, segment);
            assert!(kurtosis > 100.0, "{plan:?} {segment}: kurtosis {kurtosis}");
            let density = reading(t.echo_density, "echo density", &plan, segment);
            assert!(density < 0.7, "{plan:?} {segment}: echo density {density}");
        }
    }
}

#[test]
fn a_repeating_burst_reads_its_period() {
    // Population: 96 realisations at T60 0.8, 1.2 and 2 s, B read in 89. Periodicity 0.656–0.947 over
    // both segments (0.887–0.899 in A), at 20.000 ms in every one; noise's worst is 0.203.
    for (rate, channels, seed) in [(48_000.0, 2, 1), (44_100.0, 1, 2)] {
        let plan = TexturePlan::new(
            rate,
            channels,
            1.2,
            TextureTail::Repeats {
                period_s: 0.020,
                burst_s: 0.002,
            },
            seed,
        );
        let analysis = analysed(&plan);
        for (segment, t) in segments(&analysis) {
            let periodicity = reading(t.periodicity, "periodicity", &plan, segment);
            assert!(
                periodicity > 0.5,
                "{plan:?} {segment}: periodicity {periodicity}"
            );
            let lag = reading(t.periodicity_lag_s, "periodicity lag", &plan, segment);
            assert!(
                (lag - 0.020).abs() <= 0.0001,
                "{plan:?} {segment}: lag {lag} s"
            );
        }
    }
}

/// The range of one reading over a population.
#[derive(Clone, Copy, Debug)]
struct Range {
    low: f64,
    high: f64,
    sum: f64,
    count: usize,
}

impl Range {
    const EMPTY: Range = Range {
        low: f64::INFINITY,
        high: f64::NEG_INFINITY,
        sum: 0.0,
        count: 0,
    };

    fn add(&mut self, value: Option<f32>) {
        if let Some(v) = value {
            let v = f64::from(v);
            self.low = self.low.min(v);
            self.high = self.high.max(v);
            self.sum += v;
            self.count += 1;
        }
    }

    fn text(&self) -> String {
        if self.count == 0 {
            return "—".to_owned();
        }
        format!(
            "{:.3}–{:.3} (mean {:.3}, {})",
            self.low,
            self.high,
            self.sum / self.count as f64,
            self.count
        )
    }
}

/// **How the tolerances were argued.** Every plant over its population, each reading's range per
/// segment printed. Run it in release and read it against `NOTES.md`, *The tail texture —
/// measured*:
///
/// ```bash
/// cargo test -p mxm-classic-verb-fit --release --test texture -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, printed for AGENTS.md"]
fn measure_the_texture_populations() {
    const OTHER_T60_S: [f64; 3] = [0.8, 1.2, 2.0];
    let plants: [(&str, TextureTail, &[f64]); 4] = [
        ("noise", TextureTail::Noise, &NOISE_T60_S),
        (
            "modes 30 Hz",
            TextureTail::Modes { spacing_hz: 30.0 },
            &OTHER_T60_S,
        ),
        (
            "clicks 50/s",
            TextureTail::Clicks { per_second: 50.0 },
            &OTHER_T60_S,
        ),
        (
            "repeats 20 ms",
            TextureTail::Repeats {
                period_s: 0.020,
                burst_s: 0.002,
            },
            &OTHER_T60_S,
        ),
    ];
    for (name, tail, t60s) in plants {
        println!("{name}: 44.1 and 48 kHz, mono and stereo, seeds 1–8");
        let mut all = [[Range::EMPTY; 6]; 2];
        for &t60_s in t60s {
            let mut ranges = [[Range::EMPTY; 6]; 2];
            let (mut realisations, mut refused) = (0, Vec::new());
            for rate in RATES {
                for channels in [1, 2] {
                    for seed in 1..=8 {
                        let plan = TexturePlan::new(rate, channels, t60_s, tail, seed);
                        let built = plan.build();
                        let slices: Vec<&[f32]> = built.iter().map(Vec::as_slice).collect();
                        let analysis = match analyse(&slices, rate as f32) {
                            Ok(analysis) => analysis,
                            Err(refusal) => {
                                refused.push(format!(
                                    "{rate} Hz {channels} ch seed {seed}: {refusal}"
                                ));
                                continue;
                            }
                        };
                        realisations += 1;
                        for r in [&mut ranges, &mut all] {
                            for (r, texture) in
                                r.iter_mut().zip([analysis.texture_a, analysis.texture_b])
                            {
                                if let Some(t) = texture {
                                    r[0].add(t.peakiness);
                                    r[1].add(t.kurtosis);
                                    r[2].add(t.echo_density);
                                    r[3].add(t.periodicity);
                                    r[4].add(t.periodicity_lag_s.map(|s| 1000.0 * s));
                                    r[5].add(Some(1000.0 * (t.segment.end_s - t.segment.start_s)));
                                }
                            }
                        }
                    }
                }
            }
            println!("  T60 {t60_s} s: {realisations} analysed");
            print_ranges(&ranges);
            for line in refused {
                println!("    refused: {line}");
            }
        }
        println!("  all:");
        print_ranges(&all);
    }
}

fn print_ranges(ranges: &[[Range; 6]; 2]) {
    for (segment, r) in ["A", "B"].iter().zip(ranges) {
        println!(
            "    {segment}: peakiness {} | kurtosis {} | echo density {} | periodicity {} | lag ms {} | length ms {}",
            r[0].text(),
            r[1].text(),
            r[2].text(),
            r[3].text(),
            r[4].text(),
            r[5].text()
        );
    }
}

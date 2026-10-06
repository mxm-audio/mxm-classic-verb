//! Never NaN, and deterministic (`plans/plan-mxm-classic-verb.md` §4.3, §4.6). Nothing here reads a
//! file.

mod support;

use mxm_classic_verb_fit::{DecayMethod, MIN_DURATION_S, Refusal, analyse, analyse_with_method};
use support::{Layout, Plan, XorShift, assert_finite};

#[test]
fn pathological_inputs_never_produce_nan() {
    let rate = 22_050.0f64;
    let frames = (MIN_DURATION_S * rate).ceil() as usize;
    let mut rng = XorShift::new(3);
    let mut cases: Vec<(&str, Vec<Vec<f32>>)> = Vec::new();

    let mut impulse = vec![0.0f32; frames];
    impulse[500] = 1.0;
    cases.push(("a single impulse", vec![impulse]));

    let mut step = vec![0.0f32; frames];
    step[500..].iter_mut().for_each(|v| *v = 0.5);
    cases.push(("a DC step", vec![step]));

    let mut burst: Vec<f32> = (0..frames).map(|_| (1e-5 * rng.bipolar()) as f32).collect();
    for k in 0..50 {
        burst[500 + k] += (0.5 * rng.bipolar()) as f32;
    }
    cases.push(("a 50-sample burst", vec![burst]));

    let nyquist: Vec<f32> = (0..frames)
        .map(|n| if n % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    cases.push(("a full-scale Nyquist tone", vec![nyquist]));

    // Extremely short band content at the lowest rate: a 20 ms decay in every band, stereo.
    let mut short = Plan::room(rate, Layout::Decorrelated);
    short.duration_s = MIN_DURATION_S;
    short.t60_s = [0.02; 7];
    short.reflections.clear();
    short.tail_start_s = 0.001;
    short.direct_s = 0.01;
    cases.push(("a 20 ms decay", short.build().channels));

    // A decay that ends in exact digital silence rather than noise.
    let mut silent_end: Vec<f32> = vec![0.0; frames];
    silent_end[200] = 1.0;
    for (n, s) in silent_end.iter_mut().enumerate().skip(201).take(4_000) {
        *s += (0.05 * 10f64.powf(-3.0 * (n - 201) as f64 / rate / 0.1) * rng.bipolar()) as f32;
    }
    cases.push(("a decay into digital silence", vec![silent_end]));

    let mut analysed = 0;
    let mut outcomes = Vec::new();
    for (name, channels) in &cases {
        let slices: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
        match analyse(&slices, rate as f32) {
            Ok(analysis) => {
                assert_finite(&analysis);
                analysed += 1;
                outcomes.push(format!(
                    "{name}: analysed, confidence {}",
                    analysis.validity.confidence
                ));
            }
            Err(refusal) => {
                let text = format!("{refusal:?}");
                assert!(
                    !text.contains("NaN") && !text.contains("inf"),
                    "{name}: {text}"
                );
                assert!(
                    !matches!(refusal, Refusal::NonFiniteSample { .. }),
                    "{name}"
                );
                outcomes.push(format!("{name}: {text}"));
            }
        }
    }
    // Measured: the 20 ms decay (confidence 0.77) and the decay into digital silence (0.85) are
    // analysed; the impulse, the step, the burst and the Nyquist tone are refused for want of a decay
    // in a required band. Before trailing silence was trimmed the decay into silence was refused too
    // (no decay at 500 Hz), because its late fit landed in the band filter's ringing. So the scan is
    // exercised on inputs the analyser accepted.
    assert!(
        analysed >= 2,
        "only {analysed} analysed:\n{}",
        outcomes.join("\n")
    );
}

#[test]
fn analyses_of_the_same_samples_are_bit_identical() {
    let planted = Plan::room(48_000.0, Layout::Decorrelated).build();
    let copy: Vec<Vec<f32>> = planted.channels.clone();
    let first = analyse(&planted.slices(), 48_000.0).expect("analysed");
    let copy_slices: Vec<&[f32]> = copy.iter().map(Vec::as_slice).collect();
    let second = analyse(&copy_slices, 48_000.0).expect("analysed");
    // `Debug` prints every f32 as the shortest text that reads back to the same bits, so equal text
    // is equal bits, NaN aside, and there is no NaN.
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    let explicit = analyse_with_method(&copy_slices, 48_000.0, DecayMethod::NoiseCompensated)
        .expect("analysed");
    assert_eq!(format!("{first:?}"), format!("{explicit:?}"));
}

#[test]
#[ignore = "a timing measurement at the duration ceiling; run it in release by hand"]
fn the_duration_ceiling_is_affordable() {
    // 30 s of stereo at 192 kHz: a click, a 3 s decay and a floor. Measured on the development
    // machine in release: 1.02 s, with the test process peaking at a 137 MB working set, 46 MB of it
    // the input. No threshold is asserted on either: both depend on the machine.
    let rate = 192_000.0f64;
    let frames = (mxm_classic_verb_fit::MAX_DURATION_S * rate) as usize;
    let mut rng = XorShift::new(9);
    let channels: Vec<Vec<f32>> = (0..2)
        .map(|_| {
            let mut c = vec![0.0f32; frames];
            c[1_000] = 1.0;
            for (n, v) in c.iter_mut().enumerate().skip(1_001) {
                let t = (n - 1_001) as f64 / rate;
                *v += (0.03 * 10f64.powf(-3.0 * t / 3.0) * rng.bipolar() + 1e-5 * rng.bipolar())
                    as f32;
            }
            c
        })
        .collect();
    let slices: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    let start = std::time::Instant::now();
    let analysis = analyse(&slices, rate as f32).expect("analysed");
    eprintln!(
        "30 s of stereo at 192 kHz analysed in {:.2} s",
        start.elapsed().as_secs_f64()
    );
    assert_finite(&analysis);
}

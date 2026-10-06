//! The loaded space as durable content — what a user preset's `state` and the host's `loaded` field
//! carry (plan §2.2).
//!
//! **Versioned JSON, kilobytes, and either a space or an explicit absence.** Never the response's
//! audio and never a path: a space is 42 numbers, and the fit report beside it is a short list of
//! descriptor errors. The payload version is this plugin's; the space's own form number is
//! `mxm_classic_verb_dsp::SPACE_VERSION`, recorded beside the space so a form change is refused
//! rather than misread.
//!
//! ```json
//! { "version": 1, "space": null }
//! { "version": 1, "space": { "form": 1, "early": [[0.18, 0.82, 0.55], …], "early_level": 0.7, … },
//!   "report": { "confidence": 0.82, "errors": [{ "what": "decay", "hz": 1000.0, "value": 3.1 }], "clamps": [] } }
//! ```

use mxm_classic_verb_dsp::{EARLY_TAPS, EarlyTap, SPACE_VERSION, Space};
use serde::{Deserialize, Serialize};

/// The payload's own version. **A version this build does not know is refused whole**, before any
/// gesture (plan §2.2), because a field an older build cannot read may be the one that changes the
/// sound.
pub const PAYLOAD_VERSION: u32 = 1;

/// Below this a fit's confidence is reported as **low** beside the result rather than hidden
/// (plan §4.5). **Chosen**, as the middle of the confidence scale: the fit crate refuses under
/// `CONFIDENCE_FLOOR` (0.25) and scores a check fully at 1, and nothing measured yet puts the line
/// elsewhere. P3.5's real responses may move it.
pub const LOW_CONFIDENCE: f32 = 0.5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    pub version: u32,
    /// `None` is the explicit absence: serialised as `"space": null`, never omitted.
    pub space: Option<StoredSpace>,
    /// The fit report's summary, kept so the Space card can say how good the held space is after a
    /// reload. Absent with an absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
}

impl Payload {
    /// Nothing loaded.
    pub fn absence() -> Self {
        Self {
            version: PAYLOAD_VERSION,
            space: None,
            report: None,
        }
    }

    /// A held space and the report that came with it. The space is bounded on the way in.
    pub fn holding(space: &Space, report: Option<Report>) -> Self {
        Self {
            version: PAYLOAD_VERSION,
            space: Some(StoredSpace::from_space(&space.sanitised())),
            report,
        }
    }

    /// The payload checked and put in canonical form, with its space decoded: a known version, a
    /// space of this build's form with every value bounded, and no report beside an absence.
    pub fn normalised(mut self) -> Result<(Self, Option<Space>), String> {
        if self.version != PAYLOAD_VERSION {
            return Err(unknown_version(u64::from(self.version)));
        }
        let space = match &self.space {
            Some(stored) => Some(stored.to_space()?),
            None => None,
        };
        match space {
            Some(space) => {
                self.space = Some(StoredSpace::from_space(&space));
                if let Some(report) = self.report.take() {
                    self.report = Some(report.bounded());
                }
            }
            None => self.report = None,
        }
        Ok((self, space))
    }

    /// A deterministic identity for dirty comparison: FNV-1a over the canonical JSON. **Absence has
    /// a fingerprint of its own**, so clearing a space marks a loaded preset modified exactly as
    /// loading one does.
    pub fn fingerprint(&self) -> u64 {
        let text = serde_json::to_string(self).expect("a payload always serialises");
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a payload always serialises")
    }
}

fn unknown_version(found: u64) -> String {
    format!(
        "the loaded space was saved as version {found}, and this build reads version {PAYLOAD_VERSION}"
    )
}

/// Parses a payload from a preset's `state`: **refused whole** when it is not an object, carries no
/// version or one this build does not read, names neither a space nor an absence, or is malformed.
pub fn parse(value: &serde_json::Value) -> Result<(Payload, Option<Space>), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "the loaded space is not a JSON object".to_owned())?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "the loaded space carries no version".to_owned())?;
    if version != u64::from(PAYLOAD_VERSION) {
        return Err(unknown_version(version));
    }
    if !object.contains_key("space") {
        return Err("the loaded space names neither a space nor an absence".to_owned());
    }
    let payload: Payload = serde_json::from_value(value.clone())
        .map_err(|error| format!("the loaded space is malformed: {error}"))?;
    payload.normalised()
}

/// [`parse`], from the string nice-plug keeps a persistent field as.
pub fn parse_serialized(text: &str) -> Result<(Payload, Option<Space>), String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("the loaded space is not JSON: {error}"))?;
    parse(&value)
}

/// A space in the payload: the DSP crate's `Space`, field by field, with its form number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredSpace {
    /// `SPACE_VERSION` when it was written.
    pub form: u32,
    /// Each early tap as `[time in units of Size, left gain, right gain]`.
    pub early: Vec<[f32; 3]>,
    pub early_level: f32,
    pub decay_ratio_low: f32,
    pub decay_ratio_high: f32,
    pub decay_ratio_top: f32,
    pub tone_low_db: f32,
    pub tone_high_db: f32,
    pub width: f32,
    /// The space's high cut corner in hertz; `HIGH_CUT_OPEN_HZ` and above is open.
    pub high_cut_hz: f32,
}

impl StoredSpace {
    pub fn from_space(space: &Space) -> Self {
        Self {
            form: SPACE_VERSION,
            early: space
                .early
                .iter()
                .map(|tap| [tap.time, tap.gain_l, tap.gain_r])
                .collect(),
            early_level: space.early_level,
            decay_ratio_low: space.decay_ratio_low,
            decay_ratio_high: space.decay_ratio_high,
            decay_ratio_top: space.decay_ratio_top,
            tone_low_db: space.tone_low_db,
            tone_high_db: space.tone_high_db,
            width: space.width,
            high_cut_hz: space.high_cut_hz,
        }
    }

    /// The space, **bounded by the DSP crate's own `sanitised`**: a payload is user-editable JSON,
    /// and a space from somebody's file is user-generated input (`plugins/AGENTS.md`, *Editable
    /// models*). A form this build does not know, or the wrong number of taps, is refused.
    pub fn to_space(&self) -> Result<Space, String> {
        if self.form != SPACE_VERSION {
            return Err(format!(
                "the loaded space is of form {}, and this build plays form {SPACE_VERSION}",
                self.form
            ));
        }
        if self.early.len() != EARLY_TAPS {
            return Err(format!(
                "the loaded space has {} early taps, and a space has {EARLY_TAPS}",
                self.early.len()
            ));
        }
        let mut early = [EarlyTap {
            time: 0.0,
            gain_l: 0.0,
            gain_r: 0.0,
        }; EARLY_TAPS];
        for (tap, [time, gain_l, gain_r]) in early.iter_mut().zip(&self.early) {
            *tap = EarlyTap {
                time: *time,
                gain_l: *gain_l,
                gain_r: *gain_r,
            };
        }
        Ok(Space {
            early,
            early_level: self.early_level,
            decay_ratio_low: self.decay_ratio_low,
            decay_ratio_high: self.decay_ratio_high,
            decay_ratio_top: self.decay_ratio_top,
            tone_low_db: self.tone_low_db,
            tone_high_db: self.tone_high_db,
            width: self.width,
            high_cut_hz: self.high_cut_hz,
        }
        .sanitised())
    }
}

// ---------------------------------------------------------------------------------------------------
// The fit report's summary.
// ---------------------------------------------------------------------------------------------------

/// What a fit measured about its own result, in this plugin's words (the brief's *fit report*).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The response's confidence, 0–1 (plan §4.5).
    pub confidence: f32,
    /// Fitted render against response, one entry per descriptor both sides measured.
    #[serde(default)]
    pub errors: Vec<DescriptorError>,
    /// Fitted values that lay outside their control's range and where they were put (plan §2).
    #[serde(default)]
    pub clamps: Vec<ClampNote>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Descriptor {
    /// A band's decay time, render over response, percent.
    Decay,
    /// A band's level against 1 kHz, render minus response, dB.
    Tone,
    /// Seconds.
    PreDelay,
    /// Seconds.
    MixingTime,
    /// Late-field IACC, render minus response.
    Iacc,
    /// Early-to-late energy, render over response, dB.
    EarlyToLate,
    /// RMS time error over the matched reflections, seconds.
    ReflectionTime,
    /// RMS level error over the matched reflections, dB.
    ReflectionLevel,
    /// A descriptor a later build reports; shown, never ranked.
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DescriptorError {
    pub what: Descriptor,
    /// The band, for a per-band descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hz: Option<f32>,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Clamped {
    Decay,
    Size,
    Diffusion,
    PreDelay,
    DecayRatioLow,
    DecayRatioHigh,
    DecayRatioTop,
    ToneLow,
    ToneHigh,
    Width,
    EarlyLevel,
    HighCut,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClampNote {
    pub what: Clamped,
    pub fitted: f32,
    pub applied: f32,
}

impl Descriptor {
    /// **How big an error is, for choosing the largest across units.** An error is ranked by its
    /// size over this scale. **Chosen**, each on the scale of the just-noticeable differences
    /// commonly quoted from ISO 3382-1 where there is one — 5 % of a decay time, 1 dB of a level or
    /// an energy ratio, 10 ms of a time — and otherwise from the fit crate's own matching window:
    /// 1 ms of reflection timing (`REFLECTION_MATCH_S`), and 3 dB of reflection level. None of them
    /// is a pass mark; they only decide which error the one summary line names.
    pub const fn scale(self) -> Option<f32> {
        match self {
            Descriptor::Decay => Some(5.0),
            Descriptor::Tone | Descriptor::EarlyToLate => Some(1.0),
            Descriptor::PreDelay | Descriptor::MixingTime => Some(0.010),
            Descriptor::Iacc => Some(0.075),
            Descriptor::ReflectionTime => Some(0.001),
            Descriptor::ReflectionLevel => Some(3.0),
            Descriptor::Other => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Descriptor::Decay => "decay",
            Descriptor::Tone => "tone",
            Descriptor::PreDelay => "pre-delay",
            Descriptor::MixingTime => "mixing time",
            Descriptor::Iacc => "width (IACC)",
            Descriptor::EarlyToLate => "early/late balance",
            Descriptor::ReflectionTime => "reflection timing",
            Descriptor::ReflectionLevel => "reflection levels",
            Descriptor::Other => "other",
        }
    }
}

fn band_name(hz: f32) -> String {
    if hz >= 1_000.0 {
        let khz = hz / 1_000.0;
        if khz.fract() == 0.0 {
            format!("{khz:.0} kHz")
        } else {
            format!("{khz:.1} kHz")
        }
    } else {
        format!("{hz:.0} Hz")
    }
}

impl DescriptorError {
    /// This error over its descriptor's scale, or `None` when it is not ranked.
    pub fn weight(&self) -> Option<f32> {
        self.what.scale().map(|scale| self.value.abs() / scale)
    }

    /// `1 kHz decay +3.1 %`, `pre-delay +0.4 ms`.
    pub fn describe(&self) -> String {
        let name = match self.hz {
            Some(hz) => format!("{} {}", band_name(hz), self.what.name()),
            None => self.what.name().to_owned(),
        };
        let value = match self.what {
            Descriptor::Decay => format!("{:+.1} %", self.value),
            Descriptor::Tone | Descriptor::EarlyToLate | Descriptor::ReflectionLevel => {
                format!("{:+.1} dB", self.value)
            }
            Descriptor::PreDelay | Descriptor::MixingTime => {
                format!("{:+.1} ms", self.value * 1_000.0)
            }
            Descriptor::ReflectionTime => format!("{:.2} ms", self.value * 1_000.0),
            Descriptor::Iacc | Descriptor::Other => format!("{:+.2}", self.value),
        };
        format!("{name} {value}")
    }
}

impl Clamped {
    fn name(self) -> &'static str {
        match self {
            Clamped::Decay => "Decay",
            Clamped::Size => "Size",
            Clamped::Diffusion => "Diffusion",
            Clamped::PreDelay => "Pre-delay",
            Clamped::DecayRatioLow => "the low decay ratio",
            Clamped::DecayRatioHigh => "the high decay ratio",
            Clamped::DecayRatioTop => "the top decay ratio",
            Clamped::ToneLow => "the low tone",
            Clamped::ToneHigh => "the high tone",
            Clamped::Width => "the width",
            Clamped::EarlyLevel => "the early level",
            Clamped::HighCut => "the high cut",
            Clamped::Other => "a value",
        }
    }
}

impl Report {
    /// Every number bounded and finite, so a hand-edited payload cannot paint a NaN.
    fn bounded(mut self) -> Self {
        self.confidence = if self.confidence.is_finite() {
            self.confidence.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.errors.retain(|error| {
            error.value.is_finite() && error.hz.is_none_or(|hz| hz.is_finite() && hz > 0.0)
        });
        self.clamps
            .retain(|clamp| clamp.fitted.is_finite() && clamp.applied.is_finite());
        self
    }

    pub fn is_low_confidence(&self) -> bool {
        self.confidence < LOW_CONFIDENCE
    }

    /// The error that is largest against its own scale, first in list order on a tie.
    pub fn largest_error(&self) -> Option<&DescriptorError> {
        self.errors
            .iter()
            .filter_map(|error| error.weight().map(|weight| (weight, error)))
            .fold(
                None,
                |best: Option<(f32, &DescriptorError)>, (weight, error)| match best {
                    Some((top, _)) if top >= weight => best,
                    _ => Some((weight, error)),
                },
            )
            .map(|(_, error)| error)
    }

    /// The brief's one line: confidence, and the largest descriptor error.
    pub fn summary(&self) -> String {
        let confidence = if self.is_low_confidence() {
            format!("Low confidence {:.2}", self.confidence)
        } else {
            format!("Confidence {:.2}", self.confidence)
        };
        match self.largest_error() {
            Some(error) => format!("{confidence} · largest error {}", error.describe()),
            None => format!("{confidence} · no descriptor error measured"),
        }
    }

    /// The full report, for the hover.
    pub fn details(&self) -> String {
        let mut lines = vec![format!(
            "Fitted from an impulse response, with confidence {:.2}{}.",
            self.confidence,
            if self.is_low_confidence() {
                ", which is low: the response may not be a clean impulse response"
            } else {
                ""
            }
        )];
        if self.errors.is_empty() {
            lines.push("No descriptor was measured on both the response and the fit.".to_owned());
        } else {
            lines.push("The fitted reverb against the response:".to_owned());
            lines.extend(
                self.errors
                    .iter()
                    .map(|error| format!("  {}", error.describe())),
            );
        }
        for clamp in &self.clamps {
            lines.push(format!(
                "{} was fitted at {} and held at {}.",
                clamp.what.name(),
                trim(clamp.fitted),
                trim(clamp.applied)
            ));
        }
        lines.join("\n")
    }
}

fn trim(value: f32) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            confidence: 0.82,
            errors: vec![
                DescriptorError {
                    what: Descriptor::Decay,
                    hz: Some(1_000.0),
                    value: 6.0,
                },
                DescriptorError {
                    what: Descriptor::Tone,
                    hz: Some(125.0),
                    value: -1.5,
                },
                DescriptorError {
                    what: Descriptor::PreDelay,
                    hz: None,
                    value: 0.0004,
                },
            ],
            clamps: vec![ClampNote {
                what: Clamped::Size,
                fitted: 0.41,
                applied: 0.3,
            }],
        }
    }

    #[test]
    fn absence_is_explicit_and_has_a_fingerprint_of_its_own() {
        let absence = Payload::absence();
        let text = serde_json::to_string(&absence).unwrap();
        assert_eq!(text, r#"{"version":1,"space":null}"#);
        let held = Payload::holding(&Space::HALL, None);
        assert_ne!(absence.fingerprint(), held.fingerprint());
        assert_ne!(
            Payload::holding(&Space::ROOM, None).fingerprint(),
            held.fingerprint()
        );
        assert_eq!(absence.fingerprint(), Payload::absence().fingerprint());
    }

    #[test]
    fn a_space_round_trips_through_json_bit_for_bit() {
        let mut odd = Space::CHAMBER;
        odd.early[3].gain_l = 0.123_456_79;
        odd.tone_high_db = -2.718_281_7;
        let payload = Payload::holding(&odd, Some(report()));
        let (back, space) =
            parse(&serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap())
                .expect("its own payload parses");
        assert_eq!(space, Some(odd));
        assert_eq!(back, payload);
        assert_eq!(back.fingerprint(), payload.fingerprint());
    }

    #[test]
    fn an_unknown_version_is_refused_whole() {
        let mut value = Payload::holding(&Space::HALL, None).to_value();
        value["version"] = serde_json::json!(2);
        let error = parse(&value).unwrap_err();
        assert!(error.contains("version 2"), "{error}");
        assert!(parse(&serde_json::json!({ "space": null })).is_err());
        assert!(
            parse(&serde_json::json!({ "version": 1 })).is_err(),
            "no space and no absence"
        );
        assert!(parse(&serde_json::json!([1, 2])).is_err());
        let mut form = Payload::holding(&Space::HALL, None).to_value();
        form["space"]["form"] = serde_json::json!(SPACE_VERSION + 1);
        assert!(parse(&form).unwrap_err().contains("form"));
        let mut taps = Payload::holding(&Space::HALL, None).to_value();
        taps["space"]["early"] = serde_json::json!([[0.1, 0.2, 0.3]]);
        assert!(parse(&taps).unwrap_err().contains("early taps"));
    }

    #[test]
    fn a_hostile_space_is_bounded_on_the_way_in() {
        let mut value = Payload::holding(&Space::HALL, None).to_value();
        value["space"]["decay_ratio_low"] = serde_json::json!(1.0e30);
        value["space"]["width"] = serde_json::json!(-5.0);
        value["space"]["early"][0] = serde_json::json!([-9.0, 1.0e20, 0.5]);
        let (payload, space) = parse(&value).expect("bounded, not refused");
        let space = space.unwrap();
        assert_eq!(space, space.sanitised());
        assert_eq!(
            space.decay_ratio_low,
            mxm_classic_verb_dsp::space::MAX_DECAY_RATIO
        );
        assert_eq!(space.width, 0.0);
        assert_eq!(payload.space.unwrap().to_space().unwrap(), space);
    }

    #[test]
    fn an_absence_carries_no_report() {
        let value =
            serde_json::json!({ "version": 1, "space": null, "report": { "confidence": 0.9 } });
        let (payload, space) = parse(&value).unwrap();
        assert_eq!(space, None);
        assert_eq!(payload, Payload::absence());
    }

    #[test]
    fn the_summary_names_confidence_and_the_largest_error_against_its_scale() {
        let report = report();
        // 6 % over 5 % is 1.2; 1.5 dB over 1 dB is 1.5; 0.4 ms over 10 ms is 0.04. Tone wins.
        assert_eq!(
            report.summary(),
            "Confidence 0.82 · largest error 125 Hz tone -1.5 dB"
        );
        let low = Report {
            confidence: 0.31,
            errors: Vec::new(),
            clamps: Vec::new(),
        };
        assert_eq!(
            low.summary(),
            "Low confidence 0.31 · no descriptor error measured"
        );
        let details = report.details();
        assert!(details.contains("1 kHz decay +6.0 %"), "{details}");
        assert!(details.contains("pre-delay +0.4 ms"), "{details}");
        assert!(
            details.contains("Size was fitted at 0.41 and held at 0.3."),
            "{details}"
        );
    }

    #[test]
    fn a_later_builds_descriptor_is_shown_and_never_ranked() {
        let value = serde_json::json!({ "confidence": 0.9, "errors": [
            { "what": "something_new", "value": 99.0 },
            { "what": "decay", "hz": 500.0, "value": 1.0 }
        ] });
        let report: Report = serde_json::from_value(value).unwrap();
        assert_eq!(report.errors[0].what, Descriptor::Other);
        assert_eq!(report.largest_error().unwrap().what, Descriptor::Decay);
    }

    #[test]
    fn the_payload_is_kilobytes() {
        let mut full = report();
        for hz in [125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0] {
            for what in [Descriptor::Decay, Descriptor::Tone] {
                full.errors.push(DescriptorError {
                    what,
                    hz: Some(hz),
                    value: -123.456,
                });
            }
        }
        let text = serde_json::to_string(&Payload::holding(&Space::ROOM, Some(full))).unwrap();
        assert!(text.len() < 4_096, "{} bytes", text.len());
    }
}

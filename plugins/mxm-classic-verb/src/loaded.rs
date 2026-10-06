//! The loaded space: at most one held space, how the audio thread reads it, and the fit generations
//! that decide whether a running fit may still land (plan §2.2, §5.2).
//!
//! # Two halves, and only one of them is the audio thread's
//!
//! **Off the audio thread**, everything lives behind one mutex: the durable payload, a space staged
//! but not yet audible, the editor's status line, and a finished fit waiting for the editor. Every
//! writer — the editor's landing, a preset recall, a host state restore, the background task — takes
//! it, and none of them is `process()`.
//!
//! **The audio thread reads a sequence lock over the committed space's bits**, and nothing else.
//! A space is 42 numbers, copied by value, so there is nothing to allocate, retire or reclaim: the
//! sampler's reader-counted slots exist because its snapshots are heap allocations, and a
//! 172-byte copy needs none of that. The one writer holds the mutex, makes the sequence odd, stores
//! every word and makes it even again; a reader that sees an odd sequence, or a different one after
//! copying, keeps the space it already has and looks again at the next block. It never waits,
//! locks, allocates or frees. [`LoadedField::commit`] is the one store that makes a staged space
//! audible, and because `process` reads it once at the start of a block, **the barrier is the
//! process boundary**.
//!
//! # Generations
//!
//! Every drop takes a new generation ([`LoadedField::begin`]). A newer drop, a preset recall, Init,
//! a host state restore and the editor closing each [`LoadedField::supersede`] the running fit. A
//! result is kept only if its generation is still the newest when it completes, taken only if it is
//! still the newest when the editor looks, and committed only if it is still the newest after the
//! gestures. Reset does not supersede: it clears DSP state, not the patch.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use mxm_classic_verb_dsp::{EARLY_TAPS, EarlyTap, Space};
use nice_plug::params::persist::PersistentField;

use crate::loading::Outcome;
use crate::payload::{self, Payload, Report};

/// A presence flag, three words per early tap and eight scalars.
const WORDS: usize = 1 + 3 * EARLY_TAPS + 8;

/// What the Space card's load line says, apart from the held space's own summary.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    /// A fit is running. The name is the dropped file's, for the editor only; it never enters state.
    Fitting {
        name: String,
    },
    /// The newest drop was refused, and why.
    Refused {
        reason: String,
    },
}

struct State {
    /// The committed durable content.
    payload: Payload,
    /// Its space, decoded once.
    space: Option<Space>,
    /// Prepared and not yet audible.
    staged: Option<(Payload, Option<Space>)>,
    status: Status,
    /// A finished fit and the generation it ran under, waiting for the editor.
    ready: Option<(u64, Outcome)>,
}

pub struct LoadedField {
    state: Mutex<State>,
    committed: Committed,
    fingerprint: AtomicU64,
    /// The newest fit generation. Written only with `state` held; read without it, which is what
    /// makes it a cheap cancellation point for the background task.
    generation: AtomicU64,
}

impl Default for LoadedField {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for LoadedField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedField")
            .field("generation", &self.generation())
            .field("sequence", &self.committed_sequence())
            .finish_non_exhaustive()
    }
}

impl LoadedField {
    /// Nothing loaded.
    pub fn new() -> Self {
        let payload = Payload::absence();
        Self {
            fingerprint: AtomicU64::new(payload.fingerprint()),
            state: Mutex::new(State {
                payload,
                space: None,
                staged: None,
                status: Status::Idle,
                ready: None,
            }),
            committed: Committed::new(),
            generation: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    // ── The audio thread's half ────────────────────────────────────────────────────────────────

    /// Moves on every commit. Comparing it is how a block knows there is anything to read.
    pub fn committed_sequence(&self) -> u64 {
        self.committed.sequence.load(Ordering::Acquire)
    }

    /// The committed space and the sequence it was read at, or `None` when a write was in progress.
    /// Atomics only.
    pub fn read_committed(&self) -> Option<(u64, Option<Space>)> {
        self.committed.read()
    }

    // ── Everyone else's ────────────────────────────────────────────────────────────────────────

    /// The committed space, for the editor.
    pub fn space(&self) -> Option<Space> {
        self.lock().space
    }

    pub fn holds_space(&self) -> bool {
        self.lock().space.is_some()
    }

    pub fn report(&self) -> Option<Report> {
        self.lock().payload.report.clone()
    }

    pub fn status(&self) -> Status {
        self.lock().status.clone()
    }

    pub fn payload(&self) -> Payload {
        self.lock().payload.clone()
    }

    /// The committed payload's fingerprint, without the lock: the preset row asks every frame.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint.load(Ordering::Acquire)
    }

    pub fn is_staged(&self) -> bool {
        self.lock().staged.is_some()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generation() == generation
    }

    // ── Generations ────────────────────────────────────────────────────────────────────────────

    /// A drop: a new generation, which supersedes whatever was running, and the busy state.
    pub fn begin(&self, name: &str) -> u64 {
        let mut state = self.lock();
        let generation = self.bump();
        state.ready = None;
        state.status = Status::Fitting {
            name: name.to_owned(),
        };
        generation
    }

    /// A newer drop, a recall, Init, a restore or the editor closing: the running fit may no longer
    /// land, a finished one waiting for the editor is discarded, and the load line says nothing
    /// about either.
    pub fn supersede(&self) {
        let mut state = self.lock();
        self.supersede_locked(&mut state);
    }

    fn supersede_locked(&self, state: &mut State) {
        self.bump();
        state.ready = None;
        state.status = Status::Idle;
    }

    /// Only with `state` held.
    fn bump(&self) -> u64 {
        let generation = self.generation().wrapping_add(1);
        self.generation.store(generation, Ordering::Release);
        generation
    }

    /// A job's end. Kept for the editor only if nothing superseded it; otherwise discarded here,
    /// unread. Returns whether it was kept.
    pub fn complete(&self, generation: u64, outcome: Outcome) -> bool {
        let mut state = self.lock();
        if !self.is_current(generation) {
            return false;
        }
        state.ready = Some((generation, outcome));
        true
    }

    /// The finished fit, if the newest drop has one.
    pub fn take_ready(&self) -> Option<(u64, Outcome)> {
        let mut state = self.lock();
        let current = self.generation();
        match state.ready.take() {
            Some((generation, outcome)) if generation == current => Some((generation, outcome)),
            _ => None,
        }
    }

    /// Names a refusal on the load line, if its drop is still the newest.
    pub fn refuse(&self, generation: u64, reason: String) -> bool {
        let mut state = self.lock();
        if !self.is_current(generation) {
            return false;
        }
        state.status = Status::Refused { reason };
        true
    }

    // ── The commit barrier ─────────────────────────────────────────────────────────────────────

    /// Stages a fitted space for a drop that is still the newest. Nothing is audible yet.
    pub fn stage_for(&self, generation: u64, payload: Payload) -> bool {
        let Ok(normalised) = payload.normalised() else {
            return false;
        };
        let mut state = self.lock();
        if !self.is_current(generation) {
            return false;
        }
        state.staged = Some(normalised);
        true
    }

    /// Makes the staged space audible **only if its drop is still the newest**; a superseded one is
    /// dropped unheard. Returns whether anything was committed.
    pub fn commit_for(&self, generation: u64) -> bool {
        let mut state = self.lock();
        if !self.is_current(generation) {
            state.staged = None;
            return false;
        }
        let committed = self.commit_locked(&mut state);
        if committed {
            state.status = Status::Idle;
        }
        committed
    }

    /// Makes whatever is staged audible: the preset seam's commit. Idempotent.
    pub fn commit(&self) -> bool {
        let mut state = self.lock();
        self.commit_locked(&mut state)
    }

    fn commit_locked(&self, state: &mut State) -> bool {
        let Some((payload, space)) = state.staged.take() else {
            return false;
        };
        self.publish(state, payload, space);
        true
    }

    fn publish(&self, state: &mut State, payload: Payload, space: Option<Space>) {
        self.committed.write(space);
        self.fingerprint
            .store(payload.fingerprint(), Ordering::Release);
        state.payload = payload;
        state.space = space;
    }

    // ── The preset seam ────────────────────────────────────────────────────────────────────────

    /// Preflight only: a payload is parsed whole, and nothing changes.
    pub fn validate_preset_state(state: Option<&serde_json::Value>) -> Result<(), String> {
        state.map(payload::parse).transpose().map(|_| ())
    }

    /// A recall, before its gestures: supersedes the running fit — **a factory recipe too**, since a
    /// result landing after it would overwrite what the player just chose — and stages the recalled
    /// space or absence. A refused payload changes nothing, and supersedes nothing.
    pub fn apply_preset_state(&self, state: Option<&serde_json::Value>) -> Result<(), String> {
        let parsed = state.map(payload::parse).transpose()?;
        let mut guard = self.lock();
        self.supersede_locked(&mut guard);
        if let Some(parsed) = parsed {
            guard.staged = Some(parsed);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn open_a_write_for_test(&self) {
        let _state = self.lock();
        let sequence = self.committed_sequence();
        assert_eq!(sequence & 1, 0);
        self.committed
            .sequence
            .store(sequence + 1, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn close_the_write_for_test(&self, space: Option<Space>) {
        let _state = self.lock();
        let sequence = self.committed_sequence();
        assert_eq!(sequence & 1, 1);
        for (slot, word) in self.committed.words.iter().zip(encode(space)) {
            slot.store(word, Ordering::Release);
        }
        self.committed
            .sequence
            .store(sequence + 1, Ordering::Release);
    }
}

/// **Host state.** nice-plug writes every parameter before it hands over the fields, so a restore
/// already arrives in the barrier's order and publishes in one step; it supersedes the running fit
/// first. `Plugin::filter_state` has refused a payload this build cannot read before this is
/// reached, so the refusal here is defensive and keeps the current space.
impl<'a> PersistentField<'a, Payload> for LoadedField {
    fn set(&self, payload: Payload) {
        let normalised = payload.normalised();
        let mut state = self.lock();
        self.supersede_locked(&mut state);
        state.staged = None;
        match normalised {
            Ok((payload, space)) => self.publish(&mut state, payload, space),
            Err(reason) => state.status = Status::Refused { reason },
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Payload) -> R,
    {
        f(&self.lock().payload)
    }
}

/// A sequence lock over a space's bits. See the module documentation.
struct Committed {
    /// Even when the words are whole, odd while one writer is storing them.
    sequence: AtomicU64,
    words: [AtomicU32; WORDS],
}

impl Committed {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            words: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// With the state mutex held, so there is exactly one writer.
    fn write(&self, space: Option<Space>) {
        let sequence = self.sequence.load(Ordering::Acquire);
        self.sequence
            .store(sequence.wrapping_add(1), Ordering::Release);
        for (slot, word) in self.words.iter().zip(encode(space)) {
            slot.store(word, Ordering::Release);
        }
        self.sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
    }

    fn read(&self) -> Option<(u64, Option<Space>)> {
        let before = self.sequence.load(Ordering::Acquire);
        if before & 1 == 1 {
            return None;
        }
        let mut words = [0u32; WORDS];
        for (word, slot) in words.iter_mut().zip(&self.words) {
            *word = slot.load(Ordering::Acquire);
        }
        if self.sequence.load(Ordering::Acquire) != before {
            return None;
        }
        Some((before, decode(&words)))
    }
}

fn encode(space: Option<Space>) -> [u32; WORDS] {
    let mut words = [0u32; WORDS];
    if let Some(space) = space {
        words[0] = 1;
        let values = space
            .early
            .iter()
            .flat_map(|tap| [tap.time, tap.gain_l, tap.gain_r])
            .chain([
                space.early_level,
                space.decay_ratio_low,
                space.decay_ratio_high,
                space.decay_ratio_top,
                space.tone_low_db,
                space.tone_high_db,
                space.width,
                space.high_cut_hz,
            ]);
        for (word, value) in words[1..].iter_mut().zip(values) {
            *word = value.to_bits();
        }
    }
    words
}

fn decode(words: &[u32; WORDS]) -> Option<Space> {
    if words[0] == 0 {
        return None;
    }
    let value = |index: usize| f32::from_bits(words[index]);
    let base = 1 + 3 * EARLY_TAPS;
    Some(Space {
        early: std::array::from_fn(|tap| EarlyTap {
            time: value(1 + 3 * tap),
            gain_l: value(2 + 3 * tap),
            gain_r: value(3 + 3 * tap),
        }),
        early_level: value(base),
        decay_ratio_low: value(base + 1),
        decay_ratio_high: value(base + 2),
        decay_ratio_top: value(base + 3),
        tone_low_db: value(base + 4),
        tone_high_db: value(base + 5),
        width: value(base + 6),
        high_cut_hz: value(base + 7),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn a_space_survives_the_bits_it_is_published_as() {
        for space in [None, Some(Space::ROOM), Some(Space::PLATE)] {
            assert_eq!(decode(&encode(space)), space);
        }
    }

    /// **The audio thread never reads a half-written space.** One writer commits two spaces in turn
    /// as fast as it can while a reader copies; every copy it keeps is one of the two, whole.
    #[test]
    fn concurrent_readers_only_see_whole_spaces() {
        let field = Arc::new(LoadedField::new());
        let running = Arc::new(AtomicBool::new(true));
        let reader = {
            let field = Arc::clone(&field);
            let running = Arc::clone(&running);
            std::thread::spawn(move || {
                let mut seen = 0usize;
                while running.load(Ordering::Relaxed) {
                    if let Some((sequence, space)) = field.read_committed() {
                        assert_eq!(sequence & 1, 0);
                        assert!(
                            space.is_none()
                                || space == Some(Space::ROOM)
                                || space == Some(Space::HALL),
                            "a torn space was read: {space:?}"
                        );
                        seen += 1;
                    }
                }
                seen
            })
        };
        for round in 0..20_000 {
            let space = if round % 2 == 0 {
                Space::ROOM
            } else {
                Space::HALL
            };
            let generation = field.begin("round");
            assert!(field.stage_for(generation, Payload::holding(&space, None)));
            assert!(field.commit_for(generation));
        }
        running.store(false, Ordering::Relaxed);
        assert!(reader.join().expect("the reader never panicked") > 0);
    }

    #[test]
    fn a_write_in_progress_is_never_read() {
        let field = LoadedField::new();
        let before = field.read_committed().expect("a fresh field is readable");
        field.open_a_write_for_test();
        assert_eq!(field.read_committed(), None);
        field.close_the_write_for_test(Some(Space::PLATE));
        let (sequence, space) = field.read_committed().expect("readable again");
        assert!(sequence > before.0);
        assert_eq!(space, Some(Space::PLATE));
    }

    #[test]
    fn a_staged_space_is_not_committed_until_it_is_asked_for() {
        let field = LoadedField::new();
        let sequence = field.committed_sequence();
        let generation = field.begin("x");
        assert!(field.stage_for(generation, Payload::holding(&Space::ROOM, None)));
        assert!(field.is_staged());
        assert_eq!(field.committed_sequence(), sequence, "staging was audible");
        assert!(!field.holds_space());
        assert!(field.commit_for(generation));
        assert_eq!(field.space(), Some(Space::ROOM));
        assert_ne!(field.committed_sequence(), sequence);
        assert!(!field.commit(), "committing twice published twice");
    }

    #[test]
    fn a_generation_decides_whether_a_result_may_land() {
        let field = LoadedField::new();
        let first = field.begin("first");
        assert_eq!(
            field.status(),
            Status::Fitting {
                name: "first".into()
            }
        );
        let second = field.begin("second");
        assert!(!field.complete(first, Err(crate::loading::Refusal::Stopped)));
        assert!(field.take_ready().is_none());
        assert!(field.complete(second, Err(crate::loading::Refusal::Stopped)));
        field.supersede();
        assert!(
            field.take_ready().is_none(),
            "a superseded result was handed over"
        );
        assert_eq!(
            field.status(),
            Status::Idle,
            "a superseded fit left a message"
        );
        assert!(!field.refuse(second, "late".into()));
        assert!(!field.stage_for(second, Payload::holding(&Space::HALL, None)));
    }
}

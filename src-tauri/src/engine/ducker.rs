//! The ducking state machine for one rule.
//!
//! Pure logic: it reads source levels and drives a [`StripControl`], which the
//! real app implements over the Voicemeeter Remote API and the tests implement
//! with an in-memory fake. Metering is pre-fader, so a muted source still
//! triggers - deliberate, so a muted mic can still duck the music.

use std::collections::{HashMap, HashSet};

use super::config::Rule;
use super::layout::{clamp_gain, FADE_SLACK_EXTRA_MS, FADE_SLACK_MULT};

/// The slice of Voicemeeter the state machine needs. Faked in tests.
pub trait StripControl {
    fn get_gain(&self, strip: u32) -> f32;
    fn set_gain(&self, strip: u32, gain: f32);
    fn fade_gain(&self, strip: u32, gain: f32, ms: f64);
    fn get_color(&self, strip: u32) -> (f32, f32);
    fn set_color(&self, strip: u32, x: f32, y: f32);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Ducked,
    Releasing,
}

#[derive(Debug, Clone, Copy)]
struct TargetState {
    base_gain: f32,
    base_color: Option<(f32, f32)>,
    ducked_gain: f32,
}

pub struct Ducker {
    rule: Rule,
    color_strips: HashSet<u32>,
    state: State,
    targets: HashMap<u32, TargetState>,
    last_loud_ms: f64,
    settled_ms: f64,
    release_done_ms: f64,
}

impl Ducker {
    pub fn new(rule: Rule, color_strips: HashSet<u32>) -> Self {
        Ducker {
            rule,
            color_strips,
            state: State::Idle,
            targets: HashMap::new(),
            last_loud_ms: f64::NEG_INFINITY,
            settled_ms: 0.0,
            release_done_ms: 0.0,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    /// `levels`: source strip -> peak dB, or `None` to skip that source this tick.
    pub fn tick(&mut self, vm: &dyn StripControl, now_ms: f64, levels: &HashMap<u32, Option<f32>>) {
        self.tick_with(vm, now_ms, levels, false);
    }

    /// As [`tick`], but `force` engages ducking regardless of source level - used
    /// by the settings window's hold-to-preview button.
    pub fn tick_with(
        &mut self,
        vm: &dyn StripControl,
        now_ms: f64,
        levels: &HashMap<u32, Option<f32>>,
        force: bool,
    ) {
        let any_loud = force
            || self.rule.sources.iter().any(|s| {
                matches!(levels.get(&s.strip), Some(Some(db)) if *db >= s.threshold_db)
            });
        if any_loud {
            self.last_loud_ms = now_ms;
        }
        match self.state {
            State::Idle => {
                if any_loud {
                    self.capture_baseline(vm);
                    self.duck(vm, now_ms);
                }
            }
            State::Ducked => {
                if now_ms >= self.settled_ms {
                    self.track_manual_moves(vm);
                }
                if now_ms - self.last_loud_ms >= self.rule.hold_ms {
                    self.release(vm, now_ms);
                }
            }
            State::Releasing => {
                if any_loud {
                    // baseline is still valid; don't re-capture mid-fade
                    self.duck(vm, now_ms);
                } else if now_ms >= self.release_done_ms {
                    self.finish_release(vm);
                }
            }
        }
    }

    /// Put every target back where it was, immediately. Used on shutdown.
    pub fn restore(&mut self, vm: &dyn StripControl) {
        if self.state == State::Idle {
            return;
        }
        for t in &self.rule.targets {
            if let Some(st) = self.targets.get(t) {
                vm.set_gain(*t, st.base_gain);
                if let Some((x, y)) = st.base_color {
                    vm.set_color(*t, x, y);
                }
            }
        }
        self.targets.clear();
        self.state = State::Idle;
    }

    /// Drop state without touching Voicemeeter (it went away).
    pub fn forget(&mut self) {
        self.targets.clear();
        self.state = State::Idle;
    }

    // -- internals

    fn capture_baseline(&mut self, vm: &dyn StripControl) {
        for t in &self.rule.targets {
            let g = vm.get_gain(*t);
            let color = if self.rule.color.is_some() && self.color_strips.contains(t) {
                Some(vm.get_color(*t))
            } else {
                None
            };
            self.targets.insert(
                *t,
                TargetState {
                    base_gain: g,
                    base_color: color,
                    ducked_gain: clamp_gain(g + self.rule.duck_db),
                },
            );
        }
    }

    fn duck(&mut self, vm: &dyn StripControl, now_ms: f64) {
        for t in &self.rule.targets {
            let st = self.targets[t];
            move_gain(vm, *t, st.ducked_gain, self.rule.attack_ms);
            if st.base_color.is_some() {
                if let Some((x, y)) = self.rule.color_tuple() {
                    vm.set_color(*t, x, y);
                }
            }
        }
        self.state = State::Ducked;
        self.settled_ms = now_ms + self.rule.attack_ms * FADE_SLACK_MULT + FADE_SLACK_EXTRA_MS;
    }

    fn release(&mut self, vm: &dyn StripControl, now_ms: f64) {
        for t in &self.rule.targets {
            let st = self.targets[t];
            move_gain(vm, *t, st.base_gain, self.rule.release_ms);
        }
        self.state = State::Releasing;
        self.release_done_ms = now_ms + self.rule.release_ms * FADE_SLACK_MULT + FADE_SLACK_EXTRA_MS;
    }

    fn finish_release(&mut self, vm: &dyn StripControl) {
        // Colour comes back only once the fader has finished rising, so the sound
        // isn't left bright-but-quiet for half a second.
        for t in &self.rule.targets {
            if let Some(st) = self.targets.get(t) {
                if let Some((x, y)) = st.base_color {
                    vm.set_color(*t, x, y);
                }
            }
        }
        self.targets.clear();
        self.state = State::Idle;
    }

    fn track_manual_moves(&mut self, vm: &dyn StripControl) {
        for t in self.rule.targets.clone() {
            if let Some(st) = self.targets.get_mut(&t) {
                let g = vm.get_gain(t);
                let delta = g - st.ducked_gain;
                if delta.abs() > 0.25 {
                    st.base_gain = clamp_gain(st.base_gain + delta);
                    st.ducked_gain = g;
                }
            }
        }
    }
}

fn move_gain(vm: &dyn StripControl, strip: u32, gain: f32, ms: f64) {
    if ms > 0.0 {
        vm.fade_gain(strip, gain, ms);
    } else {
        vm.set_gain(strip, gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::config::{ColorCfg, Rule, Source};
    use std::cell::RefCell;

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Set(u32, f32),
        Fade(u32, f32, f64),
        Color(u32, f32, f32),
    }

    struct FakeVm {
        gains: RefCell<HashMap<u32, f32>>,
        colors: RefCell<HashMap<u32, (f32, f32)>>,
        calls: RefCell<Vec<Call>>,
    }

    impl FakeVm {
        fn new() -> Self {
            let mut gains = HashMap::new();
            gains.insert(1, -6.59);
            gains.insert(2, -14.32);
            let mut colors = HashMap::new();
            colors.insert(1, (0.0, 0.0));
            colors.insert(2, (-0.18, 0.0));
            FakeVm {
                gains: RefCell::new(gains),
                colors: RefCell::new(colors),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn fades(&self) -> Vec<Call> {
            self.calls.borrow().iter().filter(|c| matches!(c, Call::Fade(..))).cloned().collect()
        }
        fn has_fade(&self, strip: u32, gain: f32, ms: f64) -> bool {
            self.calls.borrow().iter().any(|c| match c {
                Call::Fade(s, g, m) => *s == strip && (*g - gain).abs() < 0.01 && (*m - ms).abs() < 0.01,
                _ => false,
            })
        }
        fn has_set(&self, strip: u32, gain: f32) -> bool {
            self.calls.borrow().iter().any(|c| match c {
                Call::Set(s, g) => *s == strip && (*g - gain).abs() < 0.01,
                _ => false,
            })
        }
        fn color_of(&self, strip: u32) -> (f32, f32) {
            self.colors.borrow()[&strip]
        }
        fn set_gain_direct(&self, strip: u32, g: f32) {
            self.gains.borrow_mut().insert(strip, g);
        }
        fn clear_calls(&self) {
            self.calls.borrow_mut().clear();
        }
        fn any_color_call(&self) -> bool {
            self.calls.borrow().iter().any(|c| matches!(c, Call::Color(..)))
        }
    }

    impl StripControl for FakeVm {
        fn get_gain(&self, strip: u32) -> f32 {
            self.gains.borrow()[&strip]
        }
        fn set_gain(&self, strip: u32, gain: f32) {
            self.gains.borrow_mut().insert(strip, gain);
            self.calls.borrow_mut().push(Call::Set(strip, gain));
        }
        fn fade_gain(&self, strip: u32, gain: f32, ms: f64) {
            self.gains.borrow_mut().insert(strip, gain);
            self.calls.borrow_mut().push(Call::Fade(strip, gain, ms));
        }
        fn get_color(&self, strip: u32) -> (f32, f32) {
            self.colors.borrow()[&strip]
        }
        fn set_color(&self, strip: u32, x: f32, y: f32) {
            self.colors.borrow_mut().insert(strip, (x, y));
            self.calls.borrow_mut().push(Call::Color(strip, x, y));
        }
    }

    fn make_rule(duck_db: f32, attack_ms: f64, color: Option<ColorCfg>) -> Rule {
        Rule {
            sources: vec![
                Source { strip: 0, threshold_db: -40.0 },
                Source { strip: 3, threshold_db: -40.0 },
            ],
            targets: vec![1, 2],
            duck_db,
            attack_ms,
            release_ms: 600.0,
            hold_ms: 1000.0,
            color,
        }
    }

    fn default_rule() -> Rule {
        make_rule(-15.0, 150.0, Some(ColorCfg { x: -0.5, y: 0.0 }))
    }

    fn ducker() -> Ducker {
        Ducker::new(default_rule(), HashSet::from([1, 2]))
    }

    fn loud() -> HashMap<u32, Option<f32>> {
        HashMap::from([(0, Some(-20.0)), (3, Some(-80.0))])
    }
    fn quiet() -> HashMap<u32, Option<f32>> {
        HashMap::from([(0, Some(-80.0)), (3, Some(-80.0))])
    }

    #[test]
    fn engages_on_loud_source() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &loud());
        assert_eq!(d.state(), State::Ducked);
        assert!(vm.has_fade(1, -21.59, 150.0));
        assert!(vm.has_fade(2, -29.32, 150.0));
        assert_eq!(vm.color_of(1), (-0.5, 0.0));
        assert_eq!(vm.color_of(2), (-0.5, 0.0));
    }

    #[test]
    fn stays_idle_when_quiet() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &quiet());
        assert_eq!(d.state(), State::Idle);
        assert!(vm.calls.borrow().is_empty());
    }

    #[test]
    fn holds_then_releases_then_restores_color() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &loud());
        d.tick(&vm, 500.0, &quiet());
        assert_eq!(d.state(), State::Ducked, "inside hold window");
        d.tick(&vm, 999.0, &quiet());
        assert_eq!(d.state(), State::Ducked);
        d.tick(&vm, 1000.0, &quiet());
        assert_eq!(d.state(), State::Releasing);
        assert!(vm.has_fade(1, -6.59, 600.0));
        assert!(vm.has_fade(2, -14.32, 600.0));
        assert_eq!(vm.color_of(1), (-0.5, 0.0), "colour stays until fade-up finishes");
        d.tick(&vm, 1000.0 + 600.0 * 1.5 + 100.0 + 1.0, &quiet());
        assert_eq!(d.state(), State::Idle);
        assert_eq!(vm.color_of(1), (0.0, 0.0));
        assert_eq!(vm.color_of(2), (-0.18, 0.0));
    }

    #[test]
    fn hold_timer_resets_while_loud() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &loud());
        d.tick(&vm, 900.0, &loud());
        d.tick(&vm, 1800.0, &quiet());
        assert_eq!(d.state(), State::Ducked);
        d.tick(&vm, 1900.0, &quiet());
        assert_eq!(d.state(), State::Releasing);
    }

    #[test]
    fn any_source_triggers() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &HashMap::from([(0, Some(-80.0)), (3, Some(-30.0))]));
        assert_eq!(d.state(), State::Ducked);
    }

    #[test]
    fn per_source_threshold() {
        let vm = FakeVm::new();
        let mut rule = default_rule();
        rule.sources = vec![
            Source { strip: 0, threshold_db: -40.0 },
            Source { strip: 3, threshold_db: -20.0 },
        ];
        let mut d = Ducker::new(rule, HashSet::from([1, 2]));
        d.tick(&vm, 0.0, &HashMap::from([(0, Some(-80.0)), (3, Some(-30.0))]));
        assert_eq!(d.state(), State::Idle);
        d.tick(&vm, 1.0, &HashMap::from([(0, Some(-80.0)), (3, Some(-19.0))]));
        assert_eq!(d.state(), State::Ducked);
    }

    #[test]
    fn none_source_is_skipped() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &HashMap::from([(0, None), (3, Some(-80.0))]));
        assert_eq!(d.state(), State::Idle);
        d.tick(&vm, 1.0, &HashMap::from([(0, None), (3, Some(-20.0))]));
        assert_eq!(d.state(), State::Ducked);
    }

    #[test]
    fn retrigger_during_release_keeps_baseline() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &loud());
        d.tick(&vm, 1000.0, &quiet()); // -> Releasing
        vm.set_gain_direct(1, -12.0); // pretend the real fade-up is mid-way
        vm.clear_calls();
        d.tick(&vm, 1100.0, &loud());
        assert_eq!(d.state(), State::Ducked);
        assert_eq!(
            vm.fades(),
            vec![Call::Fade(1, -21.59, 150.0), Call::Fade(2, -29.32, 150.0)],
            "re-duck uses the original baseline, not the mid-fade gain"
        );
        d.tick(&vm, 2100.0, &quiet());
        assert!(vm.has_fade(1, -6.59, 600.0));
    }

    #[test]
    fn manual_fader_move_shifts_baseline() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &loud());
        vm.set_gain_direct(1, -16.59); // user nudged the target up 5 dB while ducked
        d.tick(&vm, 100.0, &loud()); // still inside attack settle window: ignored
        d.tick(&vm, 400.0, &loud()); // settled: adopt the delta
        d.tick(&vm, 1400.0, &quiet());
        assert!(vm.has_fade(1, -1.59, 600.0));
    }

    #[test]
    fn gain_clamps_at_floor() {
        let vm = FakeVm::new();
        vm.set_gain_direct(1, -55.0);
        let mut d = ducker();
        d.tick(&vm, 0.0, &loud());
        assert!(vm.has_fade(1, -60.0, 150.0));
    }

    #[test]
    fn no_color_when_rule_has_none() {
        let vm = FakeVm::new();
        let mut d = Ducker::new(make_rule(-15.0, 150.0, None), HashSet::from([1, 2]));
        d.tick(&vm, 0.0, &loud());
        assert!(!vm.any_color_call());
    }

    #[test]
    fn color_skipped_for_strips_without_pad() {
        let vm = FakeVm::new();
        let mut d = Ducker::new(default_rule(), HashSet::from([1])); // strip 2 has no pad
        d.tick(&vm, 0.0, &loud());
        assert_eq!(vm.color_of(2), (-0.18, 0.0));
        assert_eq!(vm.color_of(1), (-0.5, 0.0));
    }

    #[test]
    fn restore_is_immediate_and_complete() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick(&vm, 0.0, &loud());
        d.restore(&vm);
        assert_eq!(d.state(), State::Idle);
        assert_eq!(vm.get_gain(1), -6.59);
        assert_eq!(vm.get_gain(2), -14.32);
        assert_eq!(vm.color_of(1), (0.0, 0.0));
        assert!(vm.has_set(1, -6.59));
    }

    #[test]
    fn restore_when_idle_is_noop() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.restore(&vm);
        assert!(vm.calls.borrow().is_empty());
    }

    #[test]
    fn zero_attack_sets_gain_directly() {
        let vm = FakeVm::new();
        let mut d = Ducker::new(make_rule(-15.0, 0.0, Some(ColorCfg { x: -0.5, y: 0.0 })), HashSet::from([1, 2]));
        d.tick(&vm, 0.0, &loud());
        assert!(vm.has_set(1, -21.59));
    }

    #[test]
    fn force_ducks_when_sources_are_quiet() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick_with(&vm, 0.0, &quiet(), true);
        assert_eq!(d.state(), State::Ducked);
        assert!(vm.has_fade(1, -21.59, 150.0));
    }

    #[test]
    fn releasing_after_force_stops_restores_baseline() {
        let vm = FakeVm::new();
        let mut d = ducker();
        d.tick_with(&vm, 0.0, &quiet(), true); // held
        d.tick_with(&vm, 1000.0, &quiet(), false); // released -> hold elapsed -> release
        assert_eq!(d.state(), State::Releasing);
        assert!(vm.has_fade(1, -6.59, 600.0));
    }
}

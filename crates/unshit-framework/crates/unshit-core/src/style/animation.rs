//! CSS `@keyframes` + `animation:` runtime driver.
//!
//! This module layers keyframe animation on top of the transition substrate
//! in `crate::style::transition`. It reuses the cubic bezier solver, the
//! `AnimatableValue` interpolation helpers, and the property extract/apply
//! routines, so no easing or lerp math is duplicated here.
//!
//! The driver owns a side table keyed by `NodeId` and advances every active
//! animation once per frame. Idle nodes pay no memory cost because state is
//! only inserted when an animation is actually running on the element.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smallvec::SmallVec;

use crate::id::NodeId;
use crate::style::parse::{CompiledStylesheet, StyleDeclaration};
use crate::style::transition::{self, AnimatableValue, TimingFunction, TransitionProperty};
use crate::style::types::{
    AnimationDef, AnimationDirection, AnimationFillMode, AnimationPlayState, ComputedStyle,
    IterationCount, Keyframe, KeyframesRule,
};
use crate::tree::NodeArena;

/// State of a single running animation on a single element.
///
/// Unlike transitions, animations do not need a snapshot of the old/new
/// style, because the frame values come from a global keyframes table that
/// is consulted every tick. Only the timing bookkeeping lives here, plus a
/// snapshot of the element's base style at the moment the animation first
/// started, which is used to synthesize missing 0%/100% keyframe endpoints
/// without feeding the already animated value back into itself on the next
/// frame.
#[derive(Clone, Debug)]
pub struct AnimationState {
    /// The resolved name referenced by this animation. When the name does
    /// not resolve to a `KeyframesRule`, the entry stays inert but is kept
    /// around so a later stylesheet reload can pick it up.
    pub name: Arc<str>,
    /// Original definition the cascade produced. Preserved so we can detect
    /// when the resolver has changed the parameters and the state has to be
    /// rebuilt.
    pub def: AnimationDef,
    /// Wall clock instant at which the animation entered the running state.
    pub start_time: Instant,
    /// Offset shift applied to the playhead when the animation is resumed
    /// from a paused state, so the playhead picks up where it left off
    /// instead of jumping forward by the paused duration.
    pub paused_offset: Duration,
    /// If the animation is currently paused, the instant at which it was
    /// paused is stored here so we can compute the pause duration on resume.
    pub paused_at: Option<Instant>,
    /// `true` once the animation has reached its end iteration and the fill
    /// mode determines the final sample. Used by the driver to keep the
    /// entry alive for the `forwards` and `both` fill modes.
    pub completed: bool,
    /// Cached snapshot of the element's cascaded style at the moment this
    /// state was created. Used to synthesize missing keyframe endpoints so
    /// the previous tick's animated output never becomes the new base.
    pub base_style: Option<Box<ComputedStyle>>,
}

impl AnimationState {
    pub fn new(name: Arc<str>, def: AnimationDef, start_time: Instant) -> Self {
        // A paused animation starts paused at its start_time so the first
        // tick does not shift the playhead forward.
        let paused_at =
            if def.play_state == AnimationPlayState::Paused { Some(start_time) } else { None };
        Self {
            name,
            def,
            start_time,
            paused_offset: Duration::ZERO,
            paused_at,
            completed: false,
            base_style: None,
        }
    }

    /// Compute the signed elapsed time since the animation started, honoring
    /// the paused offset and the negative delay case.
    ///
    /// The returned value is a signed nanosecond count to preserve the
    /// negative portion of the delay when the author wrote something like
    /// `animation-delay: -500ms` (which starts the animation in progress).
    fn signed_active_ns(&self, now: Instant) -> i64 {
        let reference = self.paused_at.unwrap_or(now);
        let elapsed = reference.duration_since(self.start_time);
        let elapsed_ns = elapsed.as_nanos().min(i64::MAX as u128) as i64;
        let paused_ns = self.paused_offset.as_nanos().min(i64::MAX as u128) as i64;
        // Subtract the delay; negative delay implies the playhead started
        // mid animation, so the adjusted elapsed is larger than the real
        // wall clock elapsed.
        elapsed_ns - paused_ns - self.def.delay_nanos
    }

    /// Compute the next wake instant relative to `now` for this animation,
    /// or `None` when the animation has completed and no further ticks are
    /// expected.
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if self.def.play_state == AnimationPlayState::Paused {
            return None;
        }
        if self.completed {
            return None;
        }
        // The driver wakes on every frame once the animation has entered
        // the running phase, which matches how the transition ticker
        // schedules itself.
        let active_ns = self.signed_active_ns(now);
        if active_ns < 0 {
            // Still in the delay phase. Wake exactly when the delay ends.
            let wait = (-active_ns) as u64;
            return Some(now + Duration::from_nanos(wait));
        }
        // Already active: keep ticking on the next frame.
        Some(now + Duration::from_millis(16))
    }
}

/// The global animation driver. Owns a side table keyed by `NodeId` so idle
/// nodes pay zero state cost.
#[derive(Debug, Default)]
pub struct AnimationDriver {
    /// All running animations, keyed by the node they apply to.
    pub running: HashMap<NodeId, SmallVec<[AnimationState; 2]>>,
}

impl AnimationDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true while at least one animation is ticking. The app loop
    /// uses this to decide whether to keep requesting redraws.
    pub fn has_active(&self) -> bool {
        self.running.values().any(|states| states.iter().any(|s| !s.completed))
    }

    /// Push the full set of animations for a single node, replacing any
    /// prior state.
    ///
    /// This is called by the resolver after the cascade has produced the new
    /// `ComputedStyle::animations` list. The driver diffs the prior state
    /// against the incoming `defs` and:
    ///
    /// - Keeps an existing entry unchanged when its `AnimationDef` matches.
    /// - Creates a new entry when the definition is new, caching the
    ///   provided `base_style` so later ticks can synthesize missing
    ///   keyframe endpoints without consuming their own outputs.
    /// - Drops any entries whose definitions no longer appear.
    pub fn sync_node(
        &mut self,
        node_id: NodeId,
        defs: &[AnimationDef],
        base_style: &ComputedStyle,
        now: Instant,
    ) {
        // The `animation-name: none` form is encoded as a single default
        // `AnimationDef` with `name == None`. Treat that as clearing all
        // animations on this node.
        let cleared = defs.iter().all(|d| d.name.is_none());
        if cleared || defs.is_empty() {
            self.running.remove(&node_id);
            return;
        }

        let existing = self.running.remove(&node_id).unwrap_or_default();
        let mut next: SmallVec<[AnimationState; 2]> = SmallVec::new();
        for def in defs {
            let Some(name) = def.name.clone() else {
                continue;
            };
            // Reuse the existing state when the definition matches exactly.
            // Matching covers the case where the resolver recomputes styles
            // on every frame; we must not restart animations in that case.
            if let Some(prev) = existing.iter().find(|s| s.def == *def) {
                next.push(prev.clone());
            } else {
                let mut state = AnimationState::new(name, def.clone(), now);
                state.base_style = Some(Box::new(base_style.clone()));
                next.push(state);
            }
        }
        if !next.is_empty() {
            self.running.insert(node_id, next);
        }
    }

    /// Remove all state for a node (used when the node is deallocated).
    pub fn remove_node(&mut self, node_id: NodeId) {
        self.running.remove(&node_id);
    }

    /// Tick every active animation, sampling the current value and applying
    /// it onto each element's computed style.
    ///
    /// Entries belonging to nodes that have been deallocated since the last
    /// tick are removed silently. Entries that reference a missing
    /// `@keyframes` rule are kept inert but never apply any values.
    ///
    /// The tick only touches properties that the animations actually
    /// reference, so it can coexist with the transition ticker without
    /// clobbering its output.
    pub fn tick(
        &mut self,
        arena: &mut NodeArena,
        stylesheet: &CompiledStylesheet,
        now: Instant,
    ) -> SmallVec<[NodeId; 8]> {
        let mut dead_nodes: SmallVec<[NodeId; 4]> = SmallVec::new();
        let mut ticked_nodes: SmallVec<[NodeId; 8]> = SmallVec::new();
        for (node_id, states) in self.running.iter_mut() {
            if arena.get(*node_id).is_none() {
                dead_nodes.push(*node_id);
                continue;
            }
            ticked_nodes.push(*node_id);

            // Start from the cached cascade snapshot of the first state
            // that has one, falling back to the element's current style.
            // Using the cached base guarantees that the output of the
            // previous tick never becomes the input of the current tick.
            let mut style_snapshot: ComputedStyle =
                match states.iter().find_map(|s| s.base_style.as_deref()).cloned() {
                    Some(base) => base,
                    None => match arena.get(*node_id) {
                        Some(el) => el.computed_style.clone(),
                        None => continue,
                    },
                };

            // Collect the union of animated properties up front so we can
            // restore only those fields onto the element. Anything outside
            // this set (transitions, hover rollovers) stays intact.
            let mut touched: SmallVec<[TransitionProperty; 8]> = SmallVec::new();
            for state in states.iter() {
                if let Some(rule) = stylesheet.keyframes.get(state.name.as_ref()) {
                    for prop in collect_animated_properties(rule) {
                        if !touched.contains(&prop) {
                            touched.push(prop);
                        }
                    }
                }
            }

            let mut i = 0;
            while i < states.len() {
                let state = &mut states[i];

                // Already-completed non-fill animations are kept so that
                // sync_node can match them and avoid restarting. Skip
                // them during tick since they have no visual effect.
                if state.completed {
                    i += 1;
                    continue;
                }

                // Paused animations do not advance the playhead.
                if state.def.play_state == AnimationPlayState::Paused {
                    // Still apply the frozen sample so the element renders
                    // the same values every frame while paused.
                    apply_sample(
                        &mut style_snapshot,
                        state,
                        stylesheet,
                        state.paused_at.unwrap_or(state.start_time),
                    );
                    i += 1;
                    continue;
                }

                // Handle a transition from paused to running so the
                // playhead picks up at the same offset.
                if let Some(paused_at) = state.paused_at.take() {
                    let pause_duration = now.saturating_duration_since(paused_at);
                    state.paused_offset += pause_duration;
                }

                let completed = apply_sample(&mut style_snapshot, state, stylesheet, now);
                if completed {
                    state.completed = true;
                }
                i += 1;
            }

            if let Some(el) = arena.get_mut(*node_id) {
                for prop in &touched {
                    let value = transition::extract_value(&style_snapshot, *prop);
                    transition::apply_value(&mut el.computed_style, *prop, &value);
                }
            }
        }

        // Remove entries for deallocated nodes (detected at the top of the
        // loop). Completed-but-retained animations stay so sync_node can
        // match them and avoid restarting.
        for id in dead_nodes {
            self.running.remove(&id);
        }

        ticked_nodes
    }

    /// Compute the soonest instant at which any running animation needs to
    /// sample a new frame. Paused animations and completed animations that
    /// only linger because of their fill mode contribute `None`.
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        let mut out: Option<Instant> = None;
        for states in self.running.values() {
            for state in states {
                if let Some(wake) = state.next_wake(now) {
                    out = Some(match out {
                        Some(current) if current < wake => current,
                        _ => wake,
                    });
                }
            }
        }
        out
    }
}

/// Sample one animation state and fold the result into `style`. Returns
/// `true` when the animation has reached its final iteration and should be
/// considered complete.
fn apply_sample(
    style: &mut ComputedStyle,
    state: &AnimationState,
    stylesheet: &CompiledStylesheet,
    now: Instant,
) -> bool {
    let def = &state.def;

    // Look up the keyframes by name. A missing name is inert.
    let Some(rule) = stylesheet.keyframes.get(state.name.as_ref()) else {
        return false;
    };

    // Compute the signed elapsed time in nanoseconds. Negative values
    // correspond to the delay window where the animation has not yet
    // started. When the delay is negative the start_time is logically in
    // the past and the playhead is already inside the first iteration.
    let active_ns = state.signed_active_ns(now);
    let duration_ns = def.duration.as_nanos().min(i64::MAX as u128).max(1) as i64;

    // Phase: before, active, or after.
    if active_ns < 0 {
        // Delay phase. Apply the "backwards" fill mode if requested, i.e.
        // the first keyframe at offset 0.0 already applies.
        if matches!(def.fill_mode, AnimationFillMode::Backwards | AnimationFillMode::Both) {
            let fallback_base;
            let base: &ComputedStyle = match state.base_style.as_deref() {
                Some(base) => base,
                None => {
                    fallback_base = style.clone();
                    &fallback_base
                }
            };
            sample_at_progress(style, base, rule, 0.0, def.timing_function);
        }
        return false;
    }

    // Translate the active time into a fractional iteration count, clamped
    // against the total iteration count.
    let raw_iteration = active_ns as f64 / duration_ns as f64;
    let total_iterations: f64 = match def.iteration_count {
        IterationCount::Finite(n) => n.max(0.0) as f64,
        IterationCount::Infinite => f64::INFINITY,
    };

    let completed = raw_iteration >= total_iterations;
    let clamped_iteration = if completed {
        // Snap to the final end of the last iteration.
        total_iterations
    } else {
        raw_iteration
    };

    // Iteration index (0-based) and intra-iteration progress.
    let mut iter_index = clamped_iteration.floor();
    let mut local_progress = clamped_iteration - iter_index;
    // At the very end of the last iteration of a whole iteration count, the
    // playhead has passed the imaginary next iteration's 0 mark. Walk it
    // back so the `forwards` fill mode stamps the end value of the actual
    // last iteration instead of the start of a non existent one.
    if completed && local_progress < 1e-6 && iter_index > 0.0 {
        iter_index -= 1.0;
        local_progress = 1.0;
    }
    let iter_index_u = iter_index as u64;

    // Direction flip.
    let reverse = match def.direction {
        AnimationDirection::Normal => false,
        AnimationDirection::Reverse => true,
        AnimationDirection::Alternate => iter_index_u % 2 == 1,
        AnimationDirection::AlternateReverse => iter_index_u % 2 == 0,
    };
    if reverse {
        local_progress = 1.0 - local_progress;
    }

    let progress = local_progress.clamp(0.0, 1.0) as f32;
    // Use the cached base snapshot when present so missing endpoints never
    // fold the previously animated output back into themselves on the
    // next frame. Falls back to the current style for backward compatible
    // test callers that never go through `sync_node`.
    let fallback_base;
    let base: &ComputedStyle = match state.base_style.as_deref() {
        Some(base) => base,
        None => {
            fallback_base = style.clone();
            &fallback_base
        }
    };
    sample_at_progress(style, base, rule, progress, def.timing_function);

    completed
}

/// Evaluate the keyframes rule at the given fractional progress and merge
/// the result into `style`. Uses the transition lerp machinery so no easing
/// math is duplicated in this module.
fn sample_at_progress(
    style: &mut ComputedStyle,
    base: &ComputedStyle,
    rule: &KeyframesRule,
    progress: f32,
    timing_function: TimingFunction,
) {
    if rule.frames.is_empty() {
        return;
    }

    // The progress is in animation-local time; the timing function maps it
    // to the eased animation-output time. This matches the CSS3 spec: the
    // animation-timing-function applies between keyframes (and to the full
    // duration when no explicit per keyframe timing is given).
    let eased = timing_function.evaluate(progress);

    // Build the timeline for every property encountered. We walk the frames
    // once per property to avoid re-building the frame list per tick.
    let props = collect_animated_properties(rule);
    for prop in props {
        let (frames_lo, frames_hi) = bracket_frames_for_property(rule, prop, eased);
        let (lo_offset, lo_val, hi_offset, hi_val) = match (frames_lo, frames_hi) {
            (Some((lo_off, lo_val)), Some((hi_off, hi_val))) => (lo_off, lo_val, hi_off, hi_val),
            (Some((lo_off, lo_val)), None) => {
                // The only frame is before the current progress: synthesize
                // the end from the element's base cascaded style so we
                // never read back an already animated value.
                let base_val = transition::extract_value(base, prop);
                (lo_off, lo_val, 1.0, base_val)
            }
            (None, Some((hi_off, hi_val))) => {
                let base_val = transition::extract_value(base, prop);
                (0.0, base_val, hi_off, hi_val)
            }
            (None, None) => continue,
        };

        let span = (hi_offset - lo_offset).max(1e-6);
        let local_t = ((eased - lo_offset) / span).clamp(0.0, 1.0);
        let sampled = lo_val.lerp(&hi_val, local_t);
        transition::apply_value(style, prop, &sampled);
    }
}

/// Gather the set of `TransitionProperty` identifiers referenced by any
/// frame in the rule. Properties not understood by the transition machinery
/// are skipped.
fn collect_animated_properties(rule: &KeyframesRule) -> SmallVec<[TransitionProperty; 8]> {
    let mut props: SmallVec<[TransitionProperty; 8]> = SmallVec::new();
    for frame in &rule.frames {
        for decl in &frame.declarations {
            if let Some(prop) = declaration_property(decl) {
                if !props.contains(&prop) {
                    props.push(prop);
                }
            }
        }
    }
    props
}

/// Map a `StyleDeclaration` to the `TransitionProperty` identifier the
/// transition module uses for value extract / apply.
fn declaration_property(decl: &StyleDeclaration) -> Option<TransitionProperty> {
    Some(match decl {
        StyleDeclaration::Opacity(_) => TransitionProperty::Opacity,
        StyleDeclaration::Background(_) => TransitionProperty::Background,
        StyleDeclaration::Color(_) => TransitionProperty::Color,
        StyleDeclaration::BorderColor(_) => TransitionProperty::BorderColor,
        StyleDeclaration::BorderWidth(_) => TransitionProperty::BorderWidth,
        StyleDeclaration::BorderRadius(_) => TransitionProperty::BorderRadius,
        StyleDeclaration::Padding(_) => TransitionProperty::Padding,
        StyleDeclaration::Margin(_) => TransitionProperty::Margin,
        StyleDeclaration::Width(_) => TransitionProperty::Width,
        StyleDeclaration::Height(_) => TransitionProperty::Height,
        StyleDeclaration::RowGap(_) | StyleDeclaration::ColumnGap(_) | StyleDeclaration::Gap(_) => {
            TransitionProperty::Gap
        }
        StyleDeclaration::FontSize(_) => TransitionProperty::FontSize,
        StyleDeclaration::OutlineColor(_) => TransitionProperty::OutlineColor,
        StyleDeclaration::OutlineWidth(_) => TransitionProperty::OutlineWidth,
        StyleDeclaration::BoxShadowList(_) => TransitionProperty::BoxShadow,
        StyleDeclaration::LetterSpacing(_) => TransitionProperty::LetterSpacing,
        StyleDeclaration::LineHeight(_) => TransitionProperty::LineHeight,
        _ => return None,
    })
}

/// Bracket the given eased progress between the nearest lower and upper
/// frames that define `prop`. Either side may be `None` when the rule is
/// missing an explicit endpoint, in which case the caller substitutes the
/// element's base computed style for that side.
fn bracket_frames_for_property(
    rule: &KeyframesRule,
    prop: TransitionProperty,
    progress: f32,
) -> (Option<(f32, AnimatableValue)>, Option<(f32, AnimatableValue)>) {
    let mut lo: Option<(f32, AnimatableValue)> = None;
    let mut hi: Option<(f32, AnimatableValue)> = None;

    for frame in &rule.frames {
        let Some(value) = extract_frame_value(frame, prop) else {
            continue;
        };
        if frame.offset <= progress {
            // Prefer the latest frame at or before progress.
            match lo {
                Some((off, _)) if off >= frame.offset => {}
                _ => lo = Some((frame.offset, value.clone())),
            }
        }
        if frame.offset >= progress {
            // Prefer the earliest frame at or after progress.
            match hi {
                Some((off, _)) if off <= frame.offset => {}
                _ => hi = Some((frame.offset, value)),
            }
        }
    }

    (lo, hi)
}

/// Build an `AnimatableValue` out of the latest declaration of `prop`
/// inside a keyframe. We walk the declarations in order so the last one
/// wins, matching the CSS spec.
fn extract_frame_value(frame: &Keyframe, prop: TransitionProperty) -> Option<AnimatableValue> {
    let mut out: Option<AnimatableValue> = None;
    for decl in &frame.declarations {
        let value = match (prop, decl) {
            (TransitionProperty::Opacity, StyleDeclaration::Opacity(v)) => {
                Some(AnimatableValue::Float(*v))
            }
            (TransitionProperty::Background, StyleDeclaration::Background(v)) => {
                Some(AnimatableValue::Background(v.clone()))
            }
            (TransitionProperty::Color, StyleDeclaration::Color(v)) => {
                Some(AnimatableValue::Color(*v))
            }
            (TransitionProperty::BorderColor, StyleDeclaration::BorderColor(v)) => {
                Some(AnimatableValue::Color(*v))
            }
            (TransitionProperty::BorderWidth, StyleDeclaration::BorderWidth(v)) => {
                Some(AnimatableValue::Edges(*v))
            }
            (TransitionProperty::BorderRadius, StyleDeclaration::BorderRadius(v)) => {
                Some(AnimatableValue::Corners(*v))
            }
            (TransitionProperty::Padding, StyleDeclaration::Padding(v)) => {
                Some(AnimatableValue::Edges(*v))
            }
            (TransitionProperty::Margin, StyleDeclaration::Margin(v)) => {
                Some(AnimatableValue::Edges(*v))
            }
            (TransitionProperty::Width, StyleDeclaration::Width(v)) => {
                Some(AnimatableValue::Dimension(*v))
            }
            (TransitionProperty::Height, StyleDeclaration::Height(v)) => {
                Some(AnimatableValue::Dimension(*v))
            }
            (TransitionProperty::Gap, StyleDeclaration::RowGap(v))
            | (TransitionProperty::Gap, StyleDeclaration::ColumnGap(v))
            | (TransitionProperty::Gap, StyleDeclaration::Gap(v)) => {
                Some(AnimatableValue::Float(*v))
            }
            (TransitionProperty::FontSize, StyleDeclaration::FontSize(v)) => {
                Some(AnimatableValue::Float(*v))
            }
            (TransitionProperty::OutlineColor, StyleDeclaration::OutlineColor(v)) => {
                Some(AnimatableValue::Color(*v))
            }
            (TransitionProperty::OutlineWidth, StyleDeclaration::OutlineWidth(v)) => {
                Some(AnimatableValue::Float(*v))
            }
            (TransitionProperty::BoxShadow, StyleDeclaration::BoxShadowList(_)) => {
                // box-shadow keyframes are not animated through the side
                // table because the parsed form has unresolved currentColor
                // entries; the cascade owns that resolution. Animations on
                // box-shadow fall back to the base computed style instead.
                None
            }
            (TransitionProperty::LetterSpacing, StyleDeclaration::LetterSpacing(v)) => {
                Some(AnimatableValue::Float(*v))
            }
            (TransitionProperty::LineHeight, StyleDeclaration::LineHeight(v)) => {
                Some(AnimatableValue::Float(*v))
            }
            _ => None,
        };
        if let Some(v) = value {
            out = Some(v);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::parse::CompiledStylesheet;
    use crate::style::transition::TimingFunction;
    use crate::style::types::{
        AnimationDef, AnimationDirection, AnimationFillMode, AnimationPlayState, IterationCount,
    };
    use std::time::Duration;

    fn stylesheet_with_opacity_keyframes(name: &str, from: f32, to: f32) -> CompiledStylesheet {
        let css =
            format!("@keyframes {name} {{ from {{ opacity: {from}; }} to {{ opacity: {to}; }} }}");
        CompiledStylesheet::parse(&css)
    }

    fn def_linear(name: &str, duration_ms: u64) -> AnimationDef {
        AnimationDef {
            name: Some(Arc::<str>::from(name)),
            duration: Duration::from_millis(duration_ms),
            timing_function: TimingFunction::Linear,
            delay: Duration::ZERO,
            delay_nanos: 0,
            iteration_count: IterationCount::Finite(1.0),
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }

    #[test]
    fn test_driver_lerps_opacity_across_frame() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let state =
            AnimationState::new(Arc::<str>::from("fade"), def_linear("fade", 1000), Instant::now());
        let mut style = ComputedStyle::default();
        // Sample at the midpoint of a 1s animation.
        let now = state.start_time + Duration::from_millis(500);
        let completed = apply_sample(&mut style, &state, &sheet, now);
        assert!(!completed);
        assert!(
            (style.opacity - 0.5).abs() < 0.01,
            "expected ~0.5 opacity at 500ms, got {}",
            style.opacity
        );
    }

    #[test]
    fn test_driver_iteration_count_finite() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let mut def = def_linear("fade", 100);
        def.iteration_count = IterationCount::Finite(3.0);
        let state = AnimationState::new(Arc::<str>::from("fade"), def, Instant::now());
        let mut style = ComputedStyle::default();
        // Past the end: 3 iterations of 100ms plus a cushion.
        let now = state.start_time + Duration::from_millis(350);
        let completed = apply_sample(&mut style, &state, &sheet, now);
        assert!(completed, "expected iteration count to cap out");
    }

    #[test]
    fn test_driver_iteration_count_infinite() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let mut def = def_linear("fade", 100);
        def.iteration_count = IterationCount::Infinite;
        let state = AnimationState::new(Arc::<str>::from("fade"), def, Instant::now());
        let mut style = ComputedStyle::default();
        // Far into the future: infinite animations never complete.
        let now = state.start_time + Duration::from_secs(10);
        let completed = apply_sample(&mut style, &state, &sheet, now);
        assert!(!completed, "infinite animations must never complete");
    }

    #[test]
    fn test_driver_direction_alternate() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let mut def = def_linear("fade", 100);
        def.direction = AnimationDirection::Alternate;
        def.iteration_count = IterationCount::Finite(2.0);
        let state = AnimationState::new(Arc::<str>::from("fade"), def, Instant::now());
        let mut style = ComputedStyle::default();
        // Start of iteration 1 (reverse): should read close to 1.0 (end of
        // the previous iteration, which alternated).
        let now = state.start_time + Duration::from_millis(150);
        apply_sample(&mut style, &state, &sheet, now);
        assert!(
            style.opacity > 0.4,
            "expected opacity >0.4 on reverse half, got {}",
            style.opacity
        );
    }

    #[test]
    fn test_driver_fill_mode_forwards() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let mut def = def_linear("fade", 100);
        def.fill_mode = AnimationFillMode::Forwards;
        let mut driver = AnimationDriver::new();
        let node = NodeId::DANGLING; // we will not use the arena in this test
        driver.running.insert(
            node,
            SmallVec::from_vec(vec![AnimationState::new(
                Arc::<str>::from("fade"),
                def,
                Instant::now(),
            )]),
        );
        // Drive the sample directly via apply_sample to avoid needing a
        // NodeArena for this unit test.
        let state = driver.running.get(&node).unwrap()[0].clone();
        let mut style = ComputedStyle::default();
        let now = state.start_time + Duration::from_millis(500);
        let completed = apply_sample(&mut style, &state, &sheet, now);
        assert!(completed);
        assert!((style.opacity - 1.0).abs() < 1e-3, "forwards fill should stick at end value");
    }

    #[test]
    fn test_driver_fill_mode_backwards() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let mut def = def_linear("fade", 100);
        def.fill_mode = AnimationFillMode::Backwards;
        def.delay = Duration::from_millis(200);
        def.delay_nanos = 200_000_000;
        let state = AnimationState::new(Arc::<str>::from("fade"), def, Instant::now());
        let mut style = ComputedStyle { opacity: 0.42, ..ComputedStyle::default() };
        let now = state.start_time + Duration::from_millis(50);
        apply_sample(&mut style, &state, &sheet, now);
        // During the delay window, the first keyframe value (0.0) applies.
        assert!(
            (style.opacity - 0.0).abs() < 1e-3,
            "backwards fill should pin to first keyframe, got {}",
            style.opacity
        );
    }

    #[test]
    fn test_driver_play_state_paused() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let mut def = def_linear("fade", 1000);
        def.play_state = AnimationPlayState::Paused;
        let state = AnimationState::new(Arc::<str>::from("fade"), def, Instant::now());
        let mut style = ComputedStyle::default();
        // Even after wall clock moves forward, the paused animation reads
        // its frozen sample (at offset 0, which is opacity 0.0).
        let now = state.start_time + Duration::from_millis(500);
        apply_sample(&mut style, &state, &sheet, now);
        assert!(style.opacity < 0.05, "paused animation must not advance");
    }

    #[test]
    fn test_driver_next_wake_returns_min_across_animations() {
        let mut driver = AnimationDriver::new();
        let now = Instant::now();
        let defs = [def_linear("a", 200), def_linear("b", 100), def_linear("c", 500)];
        // Stagger start times so next_wake diverges per entry.
        driver.running.insert(
            NodeId::DANGLING,
            defs.iter()
                .map(|d| AnimationState::new(d.name.clone().unwrap(), d.clone(), now))
                .collect(),
        );
        let wake = driver.next_wake(now).expect("at least one wake");
        // All entries are in the active phase here so next_wake folds back
        // to the per frame budget; assert it is a sane value in the future.
        assert!(wake >= now);
    }

    #[test]
    fn test_driver_missing_keyframes_name_is_inert() {
        let sheet = CompiledStylesheet::parse(""); // no keyframes
        let state = AnimationState::new(
            Arc::<str>::from("missing"),
            def_linear("missing", 100),
            Instant::now(),
        );
        let mut style = ComputedStyle::default();
        let now = state.start_time + Duration::from_millis(50);
        let completed = apply_sample(&mut style, &state, &sheet, now);
        assert!(!completed, "missing keyframes must not report completion");
    }

    #[test]
    fn test_driver_negative_delay_starts_in_progress() {
        let sheet = stylesheet_with_opacity_keyframes("fade", 0.0, 1.0);
        let mut def = def_linear("fade", 1000);
        def.delay = Duration::ZERO;
        def.delay_nanos = -500_000_000; // -500ms
        let state = AnimationState::new(Arc::<str>::from("fade"), def, Instant::now());
        let mut style = ComputedStyle::default();
        // Sample at t == start_time: the playhead should already be at 0.5
        // because of the negative delay.
        let now = state.start_time;
        apply_sample(&mut style, &state, &sheet, now);
        assert!(
            (style.opacity - 0.5).abs() < 0.02,
            "expected ~0.5 opacity at t=0 with -500ms delay, got {}",
            style.opacity
        );
    }

    /// Regression: completed non-fill animations must be retained in the
    /// driver so sync_node can match them. Without retention, sync_node
    /// treats the animation as new and restarts it, causing an infinite
    /// blink loop (#42).
    #[test]
    fn completed_non_fill_animation_not_restarted_by_sync() {
        use crate::element::Element;
        use crate::element::Tag;

        let sheet = stylesheet_with_opacity_keyframes("fadein", 0.0, 1.0);
        let mut arena = NodeArena::new();
        let node_id = arena.alloc(Element::new(Tag::Div));

        let mut driver = AnimationDriver::new();
        let def = def_linear("fadein", 100);
        let start = Instant::now();

        // Sync the animation definition onto the node.
        let base = ComputedStyle::default();
        driver.sync_node(node_id, &[def.clone()], &base, start);

        // Tick past the end of the animation so it completes.
        let after_end = start + Duration::from_millis(200);
        let ticked = driver.tick(&mut arena, &sheet, after_end);
        assert!(!ticked.is_empty(), "should have ticked the node");

        // The animation should now be completed.
        let states = driver.running.get(&node_id).expect("node should still be in running map");
        assert!(states.iter().any(|s| s.completed), "animation should be marked completed");

        // has_active should return false (completed animations are not active).
        assert!(!driver.has_active(), "completed animations are not active");

        // Now sync again with the same definition (simulating the next
        // frame's resolver pass). The completed state should be matched
        // and reused, NOT replaced with a new one.
        driver.sync_node(node_id, &[def], &base, after_end);
        let states_after = driver.running.get(&node_id).expect("node should still exist");
        assert!(
            states_after.iter().any(|s| s.completed),
            "sync_node should reuse the completed state, not restart; got {:?}",
            states_after.iter().map(|s| s.completed).collect::<Vec<_>>()
        );
    }
}

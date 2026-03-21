//! Custom tracing Layer that captures Bevy's per-system span durations
//! into a global map for display in the in-game profiler.
//!
//! Bevy creates one span per system at startup via `info_span!("system", name = ...)`,
//! then enters/exits it each frame. We capture the name on creation and time
//! each enter→exit pair.
//!
//! Runtime-toggled: checks RENDER_PROFILING AtomicBool so there's near-zero
//! overhead when the profiler is disabled.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::messages::RENDER_PROFILING;

/// Global EMA-smoothed timings written by the tracing Layer, read by frame_timer_start.
pub static TRACING_TIMINGS: LazyLock<Mutex<HashMap<String, f32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Per-system peak (max) ms over a rolling window, for spike detection.
/// Values are (peak_ms, frames_since_reset). Peak resets after PEAK_WINDOW frames.
pub static TRACING_PEAKS: LazyLock<Mutex<HashMap<String, (f32, u32)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Number of frames before peak resets. ~2 seconds at 60fps.
const PEAK_WINDOW: u32 = 120;

/// Clear all peak and EMA timing data.
///
/// Call on game session start (OnEnter Playing/Running) so stale lifecycle
/// peaks from OnExit systems (e.g. game_cleanup_system) do not pollute the
/// in-game profiler view.
pub fn clear_peaks() {
    if let Ok(mut peaks) = TRACING_PEAKS.lock() {
        peaks.clear();
    }
    if let Ok(mut timings) = TRACING_TIMINGS.lock() {
        timings.clear();
    }
}

/// Stored in span extensions at creation time to hold the system name.
struct SpanName(String);

/// Stored in span extensions on enter to track the start time.
struct SpanStart(Instant);

/// Visitor that extracts the `name` field from span attributes.
struct NameExtractor(Option<String>);

impl Visit for NameExtractor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "name" {
            self.0 = Some(value.to_string());
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "name" {
            self.0 = Some(format!("{:?}", value));
        }
    }
}

/// Tracing layer that captures Bevy system span durations.
/// Only active when RENDER_PROFILING is true (set by debug_profiler toggle).
pub struct SystemTimingLayer;

impl<S: Subscriber + for<'a> LookupSpan<'a>> tracing_subscriber::Layer<S> for SystemTimingLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        // Store the system name at span creation time (happens once per system at startup)
        if attrs.metadata().name() != "system" {
            return;
        }
        let mut visitor = NameExtractor(None);
        attrs.values().record(&mut visitor);
        if let Some(name) = visitor.0 {
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(SpanName(name));
            }
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if !RENDER_PROFILING.load(Ordering::Relaxed) {
            return;
        }
        if let Some(span) = ctx.span(id) {
            // Only time spans that have a SpanName (i.e. "system" spans)
            if span.extensions().get::<SpanName>().is_some() {
                span.extensions_mut().replace(SpanStart(Instant::now()));
            }
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if !RENDER_PROFILING.load(Ordering::Relaxed) {
            return;
        }
        let Some(span) = ctx.span(id) else { return };
        let ext = span.extensions();
        let (Some(name), Some(start)) = (ext.get::<SpanName>(), ext.get::<SpanStart>()) else {
            return;
        };
        let ms = start.0.elapsed().as_secs_f64() as f32 * 1000.0;
        if let Ok(mut map) = TRACING_TIMINGS.lock() {
            let entry = map.entry(name.0.clone()).or_insert(0.0);
            *entry = *entry * 0.9 + ms * 0.1;
        }
        if let Ok(mut peaks) = TRACING_PEAKS.lock() {
            let entry = peaks.entry(name.0.clone()).or_insert((0.0, 0));
            entry.0 = entry.0.max(ms);
            entry.1 += 1;
            if entry.1 >= PEAK_WINDOW {
                entry.0 = ms; // reset window, seed with current
                entry.1 = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_peaks_removes_stale_lifecycle_peaks() {
        // Simulate a stale peak from a lifecycle system (e.g. game_cleanup_system)
        // that runs once on OnExit and never accumulates enough executions to self-reset.
        {
            let mut peaks = TRACING_PEAKS.lock().unwrap();
            peaks.insert("game_cleanup_system".to_string(), (13.17, 1));
            peaks.insert("some_per_frame_system".to_string(), (0.5, 50));
        }
        {
            let mut timings = TRACING_TIMINGS.lock().unwrap();
            timings.insert("game_cleanup_system".to_string(), 13.17);
        }

        clear_peaks();

        let peaks = TRACING_PEAKS.lock().unwrap();
        assert!(
            peaks.is_empty(),
            "clear_peaks must remove all peak entries so stale lifecycle peaks do not persist"
        );
        drop(peaks);

        let timings = TRACING_TIMINGS.lock().unwrap();
        assert!(
            timings.is_empty(),
            "clear_peaks must remove all EMA timing entries"
        );
    }

    #[test]
    fn peak_window_never_resets_for_infrequent_systems() {
        // Verify the root cause: a system that runs once never reaches PEAK_WINDOW=120
        // and its peak never self-resets. After clear_peaks(), new peaks start fresh.
        {
            let mut peaks = TRACING_PEAKS.lock().unwrap();
            // Lifecycle system ran once: execution_count=1, far below PEAK_WINDOW
            peaks.insert("lifecycle_system".to_string(), (10.0, 1));
        }

        // Before clear: peak is still present (would never self-expire at count=1)
        {
            let peaks = TRACING_PEAKS.lock().unwrap();
            let (peak_ms, count) = peaks["lifecycle_system"];
            assert_eq!(count, 1);
            assert!(
                count < PEAK_WINDOW,
                "count={count} never reaches PEAK_WINDOW={PEAK_WINDOW}"
            );
            assert!(peak_ms > 0.0);
        }

        // After clear: peak is gone
        clear_peaks();
        let peaks = TRACING_PEAKS.lock().unwrap();
        assert!(!peaks.contains_key("lifecycle_system"));
    }
}

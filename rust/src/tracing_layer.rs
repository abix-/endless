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

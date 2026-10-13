//! Incremental Chrome JSON array. Only the one-byte closing bracket is replaced.
//! Perfetto accepts a truncated final event; all preceding complete events survive.
//! See JsonTraceTokenizer::OnPushDataToSorter (kOnlyTraceEvents) in Perfetto.
use super::*;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    ops::Bound::Excluded,
};

const BATCH_SIZE: usize = 2048;
const MAX_SEGMENTS: usize = MAX_EVENTS * 2 + MAX_ACTIVE_SCOPES;

#[derive(Default)]
struct Track {
    tid: usize,
    // The already emitted intervals are laminar. Index both edges so a late
    // enclosing event need not split at every child or rewrite any old event.
    starts: BTreeMap<Instant, Instant>,
    ends: BTreeMap<Instant, Instant>,
}

struct Pending {
    bytes: Vec<u8>,
    cursor: usize,
    final_batch: bool,
}

/// Owned by Capture for retry, accessed only by writer/background save tasks.
#[derive(Default)]
pub(super) struct Stream {
    file: Option<File>,
    offset: u64,
    cursor: usize,
    tracks: HashMap<ThreadId, Track>,
    segments: usize,
    omitted: u64,
    pending: Option<Pending>,
    last_checkpoint: Option<Instant>,
    finished: bool,
}

impl Stream {
    pub(super) fn save(&mut self, capture: &Capture, stopping: bool) -> anyhow::Result<u64> {
        if self.finished {
            return Ok(capture
                .lost
                .load(Ordering::Relaxed)
                .saturating_add(self.omitted));
        }
        if self.file.is_none() {
            let directory = capture
                .path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Missing trace directory"))?;
            std::fs::create_dir_all(directory)?;
            // Never truncate a previously published recording or another file.
            self.file = Some(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&capture.path)?,
            );
        }
        self.publish()?;
        while !self.finished {
            let (slices, active, exhausted, at, checkpoint) = {
                // Same lock order as Scope::finish. Clone only new events.
                let active = capture.active.lock().unwrap_or_else(|e| e.into_inner());
                let slices = capture.slices.lock().unwrap_or_else(|e| e.into_inner());
                // Timestamp after taking the snapshot locks: a newly completed
                // event must never be clipped or consumed before its own end.
                let at = capture
                    .end
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .unwrap_or_else(Instant::now);
                let checkpoint = stopping
                    || self.last_checkpoint.is_none_or(|last| {
                        at.saturating_duration_since(last) >= Duration::from_secs(1)
                    });
                let end = (self.cursor + BATCH_SIZE).min(slices.len());
                let exhausted = end == slices.len();
                let active = if checkpoint && exhausted {
                    active
                        .iter()
                        .map(|(id, slice)| (*id, slice.clone()))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                (
                    slices[self.cursor..end].to_vec(),
                    active,
                    exhausted,
                    at,
                    checkpoint,
                )
            };
            if slices.is_empty() && !checkpoint {
                break;
            }
            let cursor = self.cursor + slices.len();
            let final_batch = stopping && exhausted;
            let mut events = Vec::new();
            if self.offset == 0 {
                events.push(
                    json!({"ph":"M","name":"process_name","pid":1,"args":{"name":"VCLogg2"}}),
                );
                self.track(capture.ui_thread, capture, &mut events);
            }
            let mut slices: Vec<_> = slices
                .into_iter()
                .enumerate()
                .map(|(ix, slice)| ((self.cursor + ix) as u64, slice))
                .collect();
            if final_batch {
                slices.extend(active.iter().map(|(id, slice)| {
                    let mut slice = slice.clone();
                    slice.end = at;
                    (MAX_EVENTS as u64 + *id, Arc::new(slice))
                }));
            }
            // Sorting is limited to this batch. Previously emitted records are untouched.
            slices.sort_by_key(|(id, slice)| (slice.start, std::cmp::Reverse(slice.end), *id));
            for (id, slice) in slices {
                self.slice(capture, &slice, id, at, &mut events);
            }
            if checkpoint && exhausted {
                if !stopping {
                    // Append observations instead of repeatedly writing unfinished X
                    // slices, which would duplicate their eventual completed duration.
                    for (id, slice) in active {
                        if slice.start >= at {
                            continue;
                        }
                        if self.segments >= MAX_SEGMENTS {
                            self.omitted += 1;
                            continue;
                        }
                        self.segments += 1;
                        let tid = self.track(slice.thread, capture, &mut events);
                        events.push(json!({"ph":"i","s":"t","name":slice.name,"cat":"Application active scopes",
                            "pid":1,"tid":tid,"ts":micros(capture, at),
                            "args":{"active_scope_id":id,"source_thread":format!("{:?}",slice.thread),
                                "thread_role":if slice.thread == capture.ui_thread {"ui"} else {"background"},
                                "incomplete":true,"original_ts_us":micros(capture, slice.start.max(capture.start)),
                                "elapsed_us":at.saturating_duration_since(slice.start.max(capture.start)).as_secs_f64()*1_000_000.}}));
                    }
                }
                events.push(json!({"ph":"i","s":"t","name":if stopping {"Recording summary"} else {"Recording checkpoint"},
                    "pid":1,"tid":1,"ts":micros(capture, at),"args":{
                        "format_version":2,"ui_thread":format!("{:?}",capture.ui_thread),"ui_track_prefix":"GPUI UI / Main",
                        "recording":!stopping,"lost_events":capture.lost.load(Ordering::Relaxed).saturating_add(self.omitted),
                        "omitted_export_segments":self.omitted,"export_segment_limit":MAX_SEGMENTS,"limit":MAX_EVENTS,
                        "includes_active_scopes":stopping,"active_scope_checkpoints":true,"active_scope_limit":MAX_ACTIVE_SCOPES,
                        "duration_ms":at.saturating_duration_since(capture.start).as_secs_f64()*1000.,
                        "build":if cfg!(debug_assertions) {"Debug"} else {"Release"},"version":env!("CARGO_PKG_VERSION"),
                        "tracks":"One track per Rust thread. Incremental split slices; active scopes are instant observations until stop. Wall time, not CPU samples or GPU timings"}}));
                self.last_checkpoint = Some(at);
            }
            let mut bytes = if self.offset == 0 {
                b"[\n".to_vec()
            } else {
                b",\n".to_vec()
            };
            // At least the initial metadata, new slices, or checkpoint is normally
            // present. A batch whose events were omitted still advances the cursor.
            if events.is_empty() {
                self.cursor = cursor;
            } else {
                for (ix, event) in events.iter().enumerate() {
                    if ix > 0 {
                        bytes.extend_from_slice(b",\n");
                    }
                    serde_json::to_writer(&mut bytes, event)?;
                }
                bytes.extend_from_slice(b"\n]");
                self.pending = Some(Pending {
                    bytes,
                    cursor,
                    final_batch,
                });
                self.publish()?;
            }
            if exhausted {
                break;
            }
        }
        if self.finished {
            self.file = None;
            // Dispose of retained history on this worker, not when the result
            // capture is released by the foreground completion callback.
            self.tracks.clear();
            let mut active = capture.active.lock().unwrap_or_else(|e| e.into_inner());
            capture
                .slices
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            active.clear();
        }
        Ok(capture
            .lost
            .load(Ordering::Relaxed)
            .saturating_add(self.omitted))
    }

    fn publish(&mut self) -> anyhow::Result<()> {
        let Some(pending) = &self.pending else {
            return Ok(());
        };
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Missing trace file"))?;
        // On an IO error retain the exact batch and committed offset. Retrying
        // overwrites only its partial tail, never replays committed events.
        file.seek(SeekFrom::Start(self.offset))?;
        file.write_all(&pending.bytes)?;
        file.set_len(self.offset + pending.bytes.len() as u64)?;
        file.sync_data()?;
        self.offset += pending.bytes.len() as u64 - 1;
        self.cursor = pending.cursor;
        self.finished = pending.final_batch;
        self.pending = None;
        Ok(())
    }

    fn track(
        &mut self,
        thread: ThreadId,
        capture: &Capture,
        events: &mut Vec<serde_json::Value>,
    ) -> usize {
        let next = if thread == capture.ui_thread {
            1
        } else {
            self.tracks.len() + 1
        };
        self.tracks.entry(thread).or_insert_with(|| {
            let role = if thread == capture.ui_thread { "GPUI UI / Main" } else { "Background" };
            events.push(json!({"ph":"M","name":"thread_name","pid":1,"tid":next,"args":{"name":format!("{role} · {thread:?}")}}));
            Track { tid: next, ..Track::default() }
        }).tid
    }

    fn slice(
        &mut self,
        capture: &Capture,
        slice: &Slice,
        id: u64,
        at: Instant,
        events: &mut Vec<serde_json::Value>,
    ) {
        let start = slice.start.max(capture.start);
        let end = slice.end.min(at);
        if start >= at || end < start {
            return;
        }
        let tid = self.track(slice.thread, capture, events);
        let role = if slice.thread == capture.ui_thread {
            "ui"
        } else {
            "background"
        };
        if self.segments >= MAX_SEGMENTS {
            self.omitted += 1;
            return;
        }
        if start == end || slice.category == "GPUI frame latency" {
            self.segments += 1;
            events.push(json!({"ph":"i","s":"t","name":slice.name,"cat":slice.category,"pid":1,"tid":tid,
                "ts":micros(capture,start),"args":{"thread_role":role,"source_thread":format!("{:?}",slice.thread),
                "elapsed_us":end.duration_since(start).as_secs_f64()*1_000_000.}}));
            return;
        }
        let track = self
            .tracks
            .get_mut(&slice.thread)
            .expect("registered trace thread");
        let mut a = start;
        while a < end {
            if self.segments >= MAX_SEGMENTS {
                // The remaining logical tail is omitted without expanding it.
                self.omitted += 1;
                break;
            }
            // Re-evaluate after every cut: a continuation can share its new
            // start with an already emitted child and must fit inside it too.
            let crossing_start = track
                .starts
                .range((Excluded(a), Excluded(end)))
                .find_map(|(s, e)| (*e > end).then_some(*s))
                .unwrap_or(end);
            let crossing_end = track
                .ends
                .range((Excluded(a), Excluded(end)))
                .find_map(|(e, s)| (*s <= a).then_some(*e))
                .unwrap_or(end);
            let b = crossing_start.min(crossing_end);
            self.segments += 1;
            events.push(json!({"ph":"X","name":slice.name,"cat":slice.category,"pid":1,"tid":tid,
                "ts":micros(capture,a),"dur":b.duration_since(a).as_secs_f64()*1_000_000.,
                "args":{"thread_role":role,"source_thread":format!("{:?}",slice.thread),
                "incomplete":slice.incomplete || slice.end > at,"active_scope_id":slice.scope_id,"source_event":id,"continued":a>start,
                "split":a>start || b<end,"original_ts_us":micros(capture,start),"original_duration_us":end.duration_since(start).as_secs_f64()*1_000_000.}}));
            track
                .starts
                .entry(a)
                .and_modify(|e| *e = (*e).max(b))
                .or_insert(b);
            track
                .ends
                .entry(b)
                .and_modify(|s| *s = (*s).min(a))
                .or_insert(a);
            a = b;
        }
    }
}

fn micros(capture: &Capture, at: Instant) -> f64 {
    at.saturating_duration_since(capture.start).as_secs_f64() * 1_000_000.
}

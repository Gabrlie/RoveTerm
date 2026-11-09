//! One sync round: detect local changes, pull and merge other devices' segments, push this
//! device's changes as a new segment. See docs/decisions/0005-sync.md.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hlc::{Clock, Hlc, now_millis};
use crate::{SyncBackend, SyncError, SyncKey};

/// Directory holding one subdirectory of segments per device.
const DEVICES: &str = "devices";
const SEGMENT_SUFFIX: &str = ".seg";
/// Longest record kind or id accepted from a segment.
const MAX_ID: usize = 128;
/// Once this device has more segments than this, a round rewrites its history as one segment.
pub const COMPACT_AFTER: usize = 32;

/// A record as the store holds it: `kind` names the table (`host`, `snippet`…), `id` the row.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub kind: String,
    pub id: String,
    pub body: Value,
}

/// One change to one record. `body: None` deletes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub kind: String,
    pub id: String,
    pub hlc: Hlc,
    pub body: Option<Value>,
}

/// The local data being synced. `records` lists everything syncable; `apply` writes the
/// changes that won the merge (upsert, or delete when `body` is `None`), ordered by timestamp.
/// The store may reorder them to satisfy its own references (parents before children).
pub trait RecordStore: Send + Sync {
    fn records(&self) -> Result<Vec<Record>, SyncError>;
    fn apply(&self, changes: &[Change]) -> Result<(), SyncError>;
}

/// What this device last knew about a record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    /// Keyed fingerprint of the body; empty for a deleted record.
    pub fingerprint: String,
    /// Timestamp of the change that produced this state.
    pub hlc: Hlc,
    pub deleted: bool,
}

/// This device's sync bookkeeping. Persisted by the caller between rounds (it holds no
/// secrets: fingerprints are keyed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncState {
    /// This device's id in the space.
    pub device: String,
    pub clock: Clock,
    /// Next segment number to write.
    pub next_seq: u64,
    /// Highest segment read from each other device.
    pub seen: BTreeMap<String, u64>,
    /// State of every record seen, by `kind:id`.
    pub known: BTreeMap<String, Known>,
}

impl SyncState {
    /// Fresh state for a device joining a space; `device` must be a valid path component.
    pub fn new(device: &str) -> Self {
        Self {
            device: device.to_owned(),
            clock: Clock::new(device),
            next_seq: 1,
            seen: BTreeMap::new(),
            known: BTreeMap::new(),
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Segments read from other devices.
    pub segments: usize,
    /// Remote changes written to the local store.
    pub applied: usize,
    /// Local changes sent in a new segment.
    pub pushed: usize,
    /// This device's segments were rewritten as one.
    pub compacted: bool,
}

#[derive(Serialize, Deserialize)]
struct Segment {
    device: String,
    seq: u64,
    changes: Vec<Change>,
}

fn record_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

fn segment_name(seq: u64) -> String {
    format!("{seq:016}{SEGMENT_SUFFIX}")
}

fn segment_aad(device: &str, seq: u64) -> Vec<u8> {
    format!("rove/sync/segment/v1:{device}:{seq}").into_bytes()
}

fn fingerprint(key: &SyncKey, body: &Value) -> Result<String, SyncError> {
    let bytes = serde_json::to_vec(body).map_err(|error| SyncError::Format(error.to_string()))?;
    Ok(key.fingerprint(&bytes))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_ID && !value.chars().any(char::is_control)
}

/// Segment numbers present in `dir`, ascending.
async fn segments_in(backend: &dyn SyncBackend, dir: &str) -> Result<Vec<u64>, SyncError> {
    let mut seqs: Vec<u64> = backend
        .list(dir)
        .await?
        .iter()
        .filter_map(|name| name.strip_suffix(SEGMENT_SUFFIX)?.parse().ok())
        .collect();
    seqs.sort_unstable();
    Ok(seqs)
}

/// Runs one sync round against `backend`. `state` is updated only when the round succeeds;
/// remote changes already written to `store` are written again, harmlessly, by the next round.
pub async fn sync(
    backend: &dyn SyncBackend,
    key: &SyncKey,
    state: &mut SyncState,
    store: &dyn RecordStore,
) -> Result<Report, SyncError> {
    let mut next = state.clone();
    let mut report = Report::default();
    // A device's first round: it has never read nor written anything in this space.
    let joining = state.known.is_empty() && state.seen.is_empty() && state.next_seq <= 1;
    let own_dir = format!("{DEVICES}/{}", next.device);
    crate::backend::check_path(&own_dir)?;

    // Local changes since the last round, timestamped now so they compete with remote ones.
    let mut pending: BTreeMap<String, Change> = BTreeMap::new();
    let mut current: HashMap<String, Record> = HashMap::new();
    for record in store.records()? {
        current.insert(record_key(&record.kind, &record.id), record);
    }
    for (record_id, record) in &current {
        let print = fingerprint(key, &record.body)?;
        let unchanged = next
            .known
            .get(record_id)
            .is_some_and(|known| !known.deleted && known.fingerprint == print);
        if !unchanged {
            let hlc = next.clock.tick(now_millis());
            pending.insert(
                record_id.clone(),
                Change {
                    kind: record.kind.clone(),
                    id: record.id.clone(),
                    hlc,
                    body: Some(record.body.clone()),
                },
            );
        }
    }
    for (record_id, known) in &next.known {
        if !known.deleted
            && !current.contains_key(record_id)
            && let Some((kind, id)) = record_id.split_once(':')
        {
            let hlc = next.clock.tick(now_millis());
            pending.insert(
                record_id.clone(),
                Change {
                    kind: kind.to_owned(),
                    id: id.to_owned(),
                    hlc,
                    body: None,
                },
            );
        }
    }
    for change in pending.values() {
        let known = match &change.body {
            Some(body) => Known {
                fingerprint: fingerprint(key, body)?,
                hlc: change.hlc.clone(),
                deleted: false,
            },
            None => Known {
                fingerprint: String::new(),
                hlc: change.hlc.clone(),
                deleted: true,
            },
        };
        next.known
            .insert(record_key(&change.kind, &change.id), known);
    }

    // Pull every other device's new segments.
    let mut remote: Vec<Change> = Vec::new();
    for device in backend.list(DEVICES).await? {
        if device == next.device || crate::backend::check_path(&device).is_err() {
            continue;
        }
        let dir = format!("{DEVICES}/{device}");
        let seen = next.seen.get(&device).copied().unwrap_or(0);
        for seq in segments_in(backend, &dir)
            .await?
            .into_iter()
            .filter(|seq| *seq > seen)
        {
            let Some(sealed) = backend.get(&format!("{dir}/{}", segment_name(seq))).await? else {
                continue;
            };
            let plain = key.open(&sealed, &segment_aad(&device, seq))?;
            let segment: Segment = serde_json::from_slice(&plain)
                .map_err(|error| SyncError::Format(error.to_string()))?;
            if segment.device != device || segment.seq != seq {
                return Err(SyncError::Rejected);
            }
            remote.extend(segment.changes.into_iter().filter(|change| {
                valid_id(&change.kind) && valid_id(&change.id) && !change.kind.contains(':')
            }));
            next.seen.insert(device.clone(), seq);
            report.segments += 1;
        }
    }

    // Merge: a remote change wins when its timestamp is later than what this device knows. In a
    // device's first round the space wins for records both sides have: the local copies were
    // only just timestamped, not just edited (a fresh device's default preferences, say).
    remote.sort_by(|a, b| a.hlc.cmp(&b.hlc));
    let mut winners: BTreeMap<String, Change> = BTreeMap::new();
    for change in remote {
        next.clock.observe(&change.hlc);
        let record_id = record_key(&change.kind, &change.id);
        let space_wins = joining && pending.contains_key(&record_id);
        if !space_wins
            && next
                .known
                .get(&record_id)
                .is_some_and(|known| known.hlc >= change.hlc)
        {
            continue;
        }
        pending.remove(&record_id);
        let known = match &change.body {
            Some(body) => Known {
                fingerprint: fingerprint(key, body)?,
                hlc: change.hlc.clone(),
                deleted: false,
            },
            None => Known {
                fingerprint: String::new(),
                hlc: change.hlc.clone(),
                deleted: true,
            },
        };
        next.known.insert(record_id.clone(), known);
        winners.insert(record_id, change);
    }
    if !winners.is_empty() {
        let mut ordered: Vec<Change> = winners.into_values().collect();
        ordered.sort_by(|a, b| a.hlc.cmp(&b.hlc));
        store.apply(&ordered)?;
        report.applied = ordered.len();
    }

    // Push what is left as one new segment, numbered after anything already written.
    let mut written = segments_in(backend, &own_dir).await?;
    if !pending.is_empty() {
        let seq = next.next_seq.max(written.last().map_or(1, |last| last + 1));
        let changes: Vec<Change> = pending.into_values().collect();
        report.pushed = changes.len();
        write_segment(backend, key, &next.device, seq, changes).await?;
        next.next_seq = seq + 1;
        written.push(seq);
    }

    if written.len() > COMPACT_AFTER {
        compact(backend, key, &mut next, store, &written).await?;
        report.compacted = true;
    }

    *state = next;
    Ok(report)
}

async fn write_segment(
    backend: &dyn SyncBackend,
    key: &SyncKey,
    device: &str,
    seq: u64,
    changes: Vec<Change>,
) -> Result<(), SyncError> {
    let segment = Segment {
        device: device.to_owned(),
        seq,
        changes,
    };
    let plain = zeroize::Zeroizing::new(
        serde_json::to_vec(&segment).map_err(|error| SyncError::Format(error.to_string()))?,
    );
    let sealed = key.seal(&plain, &segment_aad(device, seq))?;
    backend
        .put(
            &format!("{DEVICES}/{device}/{}", segment_name(seq)),
            &sealed,
        )
        .await
}

/// Rewrites this device's history as one segment: the latest change it made to every record
/// it was the last to change (deletions included), then removes the older segments. Changes
/// other devices superseded are dropped; they would lose every merge anyway. A device reading
/// meanwhile misses nothing: the new segment has a higher number than everything it replaces.
async fn compact(
    backend: &dyn SyncBackend,
    key: &SyncKey,
    state: &mut SyncState,
    store: &dyn RecordStore,
    written: &[u64],
) -> Result<(), SyncError> {
    let mut bodies: HashMap<String, Value> = store
        .records()?
        .into_iter()
        .map(|record| (record_key(&record.kind, &record.id), record.body))
        .collect();
    let mut changes = Vec::new();
    for (record_id, known) in &state.known {
        if known.hlc.device != state.device {
            continue;
        }
        let Some((kind, id)) = record_id.split_once(':') else {
            continue;
        };
        let body = if known.deleted {
            None
        } else {
            // Gone from the store without a round seeing it: the next round records the deletion.
            match bodies.remove(record_id) {
                Some(body) => Some(body),
                None => continue,
            }
        };
        changes.push(Change {
            kind: kind.to_owned(),
            id: id.to_owned(),
            hlc: known.hlc.clone(),
            body,
        });
    }
    let seq = state
        .next_seq
        .max(written.last().map_or(1, |last| last + 1));
    write_segment(backend, key, &state.device, seq, changes).await?;
    state.next_seq = seq + 1;
    for old in written.iter().filter(|old| **old < seq) {
        backend
            .delete(&format!(
                "{DEVICES}/{}/{}",
                state.device,
                segment_name(*old)
            ))
            .await?;
    }
    Ok(())
}

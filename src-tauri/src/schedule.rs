//! When the fleet lead's scheduled tasks are due. The app has no service running in the
//! background, so tasks run while it is open: every minute it runs what is due, and at start it
//! decides about runs missed while it was closed (the latest one is made when the task allows
//! catching up, otherwise it is noted as missed). Each decision is an `ai/schedule_run` journal
//! record (`schedule`, `due`, `status`, `conversation`), so a time is never handled twice.

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use rove_vault::{Repeat, Schedule};

/// Journal kind of a handled scheduled time.
pub const KIND_RUN: &str = "ai/schedule_run";

/// The latest scheduled time of `repeat` at or before `now`, in `now`'s time zone.
pub fn latest_due(repeat: Repeat, now: &Zoned) -> Option<Timestamp> {
    let at = |zoned: Result<Zoned, jiff::Error>| zoned.ok().map(|zoned| zoned.timestamp());
    let today = |hour: u8, minute: u8| {
        now.date()
            .at(hour as i8, minute as i8, 0, 0)
            .to_zoned(now.time_zone().clone())
    };
    let candidate = match repeat {
        Repeat::Once { at: millis } => {
            let at = Timestamp::from_millisecond(millis).ok()?;
            return (at <= now.timestamp()).then_some(at);
        }
        Repeat::Hourly { minute } => {
            let this_hour = now
                .date()
                .at(now.hour(), minute as i8, 0, 0)
                .to_zoned(now.time_zone().clone());
            let this_hour = at(this_hour)?;
            if this_hour <= now.timestamp() {
                this_hour
            } else {
                this_hour.checked_sub(1.hour()).ok()?
            }
        }
        Repeat::Daily { hour, minute } => {
            let zoned = today(hour, minute).ok()?;
            if zoned.timestamp() <= now.timestamp() {
                zoned.timestamp()
            } else {
                zoned.checked_sub(1.day()).ok()?.timestamp()
            }
        }
        Repeat::Weekly {
            weekday,
            hour,
            minute,
        } => {
            let back = (i64::from(now.weekday().to_monday_zero_offset()) - i64::from(weekday))
                .rem_euclid(7);
            let zoned = today(hour, minute).ok()?.checked_sub(back.days()).ok()?;
            if zoned.timestamp() <= now.timestamp() {
                zoned.timestamp()
            } else {
                zoned.checked_sub(7.days()).ok()?.timestamp()
            }
        }
    };
    Some(candidate)
}

/// What to do about a schedule now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Wait,
    /// Run for the scheduled time `due` (Unix milliseconds).
    Run {
        due: i64,
    },
    /// `due` passed while the app was closed and the task does not catch up: note it.
    Missed {
        due: i64,
    },
}

/// What to do about `schedule` at `now`, given the latest time already handled (`handled`) and
/// when the app started (`started`, Unix milliseconds).
pub fn decide(schedule: &Schedule, handled: Option<i64>, started: i64, now: &Zoned) -> Decision {
    if !schedule.enabled {
        return Decision::Wait;
    }
    let Some(due) = latest_due(schedule.repeat, now).map(|due| due.as_millisecond()) else {
        return Decision::Wait;
    };
    // Times before the task existed, and times already handled, are not due.
    if due < schedule.created_at || handled.is_some_and(|handled| handled >= due) {
        return Decision::Wait;
    }
    // A minute's grace: a time just before start counts as reached while running.
    if due + 60_000 >= started || schedule.catch_up {
        Decision::Run { due }
    } else {
        Decision::Missed { due }
    }
}

/// A scheduled run ended (`{schedule, name, conversation, ok}`).
pub const REPORT_EVENT: &str = "ai-report";
/// How often due tasks are looked for.
const TICK: std::time::Duration = std::time::Duration::from_secs(60);
/// The first look, once the vault and connections have had time to settle.
const FIRST_TICK: std::time::Duration = std::time::Duration::from_secs(20);

/// Tasks running now, by id; a task never runs twice at once.
static RUNNING: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
    std::sync::Mutex::new(None);

fn claim(id: &str) -> bool {
    RUNNING
        .lock()
        .map(|mut running| {
            running
                .get_or_insert_with(Default::default)
                .insert(id.to_owned())
        })
        .unwrap_or(false)
}

fn release(id: &str) {
    if let Ok(mut running) = RUNNING.lock()
        && let Some(running) = running.as_mut()
    {
        running.remove(id);
    }
}

/// Whether task `id` is running now.
pub fn is_running(id: &str) -> bool {
    RUNNING
        .lock()
        .map(|running| running.as_ref().is_some_and(|running| running.contains(id)))
        .unwrap_or(false)
}

/// The latest handled record of each task: schedule id → (due, status, conversation, at).
pub fn handled(
    journal: &dyn rove_journal::JournalStore,
) -> std::collections::HashMap<String, (i64, String, Option<String>, i64)> {
    let records = journal
        .read(&rove_journal::JournalQuery {
            kind: Some(KIND_RUN.to_owned()),
            limit: Some(5000),
            newest_first: true,
            ..rove_journal::JournalQuery::default()
        })
        .unwrap_or_default();
    let mut latest = std::collections::HashMap::new();
    for record in records {
        let payload = &record.payload;
        let (Some(id), Some(due)) = (
            payload.get("schedule").and_then(serde_json::Value::as_str),
            payload.get("due").and_then(serde_json::Value::as_i64),
        ) else {
            continue;
        };
        // Newest first: the first record of a task is its latest.
        latest.entry(id.to_owned()).or_insert_with(|| {
            (
                due,
                payload["status"].as_str().unwrap_or_default().to_owned(),
                payload["conversation"].as_str().map(str::to_owned),
                record.ts,
            )
        });
    }
    latest
}

fn record(
    journal: &dyn rove_journal::JournalStore,
    schedule: &str,
    due: i64,
    status: &str,
    conversation: Option<&str>,
) {
    let record = rove_journal::NewRecord {
        kind: KIND_RUN.to_owned(),
        session_id: None,
        payload: serde_json::json!({ "schedule": schedule, "due": due, "status": status, "conversation": conversation }),
    };
    if let Err(error) = journal.append(record) {
        tracing::warn!(%error, "recording a scheduled run failed");
    }
}

fn reserve_run(
    journal: &dyn rove_journal::JournalStore,
    schedule: &str,
    due: i64,
) -> Result<i64, rove_journal::JournalError> {
    journal.append(rove_journal::NewRecord {
        kind: "ai/schedule_claim".into(),
        session_id: None,
        payload: serde_json::json!({"schedule":schedule, "due":due}),
    })
}

/// Runs `schedule` for its time `due` in the background, unless it is running already.
pub fn spawn_run(app: &tauri::AppHandle, schedule: Schedule, due: i64) -> bool {
    use tauri::{Emitter, Manager};
    let state = app.state::<crate::state::AppState>();
    if !state
        .vault
        .installation_id()
        .is_ok_and(|id| schedule.owner == Some(id))
        || !claim(&schedule.id)
    {
        return false;
    }
    let reserved = reserve_run(state.journal.as_ref(), &schedule.id, due);
    if let Err(error) = reserved {
        release(&schedule.id);
        tracing::warn!(%error, "could not reserve the scheduled run");
        return false;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::state::AppState>();
        // Handled from the start: a run cut short by quitting is not made again.
        record(state.journal.as_ref(), &schedule.id, due, "started", None);
        tracing::info!(schedule = %schedule.id, "a scheduled task started");
        let when = Timestamp::from_millisecond(due)
            .map(|at| {
                at.to_zoned(TimeZone::system())
                    .strftime("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let (conversation, result) =
            crate::commands::ai::run_scheduled(&app, &schedule, &when).await;
        let ok = result.is_ok();
        if let Err(error) = &result {
            tracing::warn!(schedule = %schedule.id, %error, "a scheduled task failed");
        }
        record(
            state.journal.as_ref(),
            &schedule.id,
            due,
            if ok { "done" } else { "failed" },
            conversation.as_deref(),
        );
        release(&schedule.id);
        if let Err(error) = app.emit(
            REPORT_EVENT,
            serde_json::json!({ "schedule": schedule.id, "name": schedule.name, "conversation": conversation, "ok": ok, "error": result.err() }),
        ) {
            tracing::warn!(%error, "announcing a report failed");
        }
    });
    true
}

/// Looks for due tasks every minute while the app runs.
pub fn start(app: &tauri::AppHandle) {
    use tauri::Manager;
    let app = app.clone();
    let started = Timestamp::now().as_millisecond();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK).await;
        loop {
            let state = app.state::<crate::state::AppState>();
            match state.vault.preferences() {
                Ok(prefs) if !prefs.ai.schedules.is_empty() => {
                    let handled = handled(state.journal.as_ref());
                    let now = now();
                    let device = state.vault.installation_id().ok();
                    for schedule in prefs.ai.schedules {
                        if device.is_none() || device != schedule.owner {
                            continue;
                        }
                        if is_running(&schedule.id) {
                            continue;
                        }
                        let last = handled.get(&schedule.id).map(|entry| entry.0);
                        match decide(&schedule, last, started, &now) {
                            Decision::Wait => {}
                            Decision::Run { due } => {
                                spawn_run(&app, schedule, due);
                            }
                            Decision::Missed { due } => {
                                tracing::info!(schedule = %schedule.id, "a scheduled time passed while the app was closed");
                                record(state.journal.as_ref(), &schedule.id, due, "missed", None);
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "scheduled tasks: the preferences could not be read")
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// `now` in the device's time zone.
pub fn now() -> Zoned {
    Zoned::now().with_time_zone(TimeZone::system())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Zoned {
        text.parse().unwrap()
    }

    fn schedule(repeat: Repeat, catch_up: bool) -> Schedule {
        Schedule {
            owner: None,
            id: "s".into(),
            name: "s".into(),
            instruction: "check".into(),
            repeat,
            allow_changes: false,
            catch_up,
            enabled: true,
            created_at: at("2026-11-01T00:00:00+08:00[Asia/Shanghai]")
                .timestamp()
                .as_millisecond(),
        }
    }

    #[test]
    fn the_latest_time_is_found_for_each_repeat() {
        let now = at("2026-11-11T08:30:00+08:00[Asia/Shanghai]"); // a Wednesday
        let ms = |text: &str| Some(at(text).timestamp());
        assert_eq!(
            latest_due(Repeat::Hourly { minute: 45 }, &now),
            ms("2026-11-11T07:45:00+08:00[Asia/Shanghai]")
        );
        assert_eq!(
            latest_due(Repeat::Daily { hour: 9, minute: 0 }, &now),
            ms("2026-11-10T09:00:00+08:00[Asia/Shanghai]")
        );
        assert_eq!(
            latest_due(Repeat::Daily { hour: 8, minute: 0 }, &now),
            ms("2026-11-11T08:00:00+08:00[Asia/Shanghai]")
        );
        // Monday 09:00 of this week.
        assert_eq!(
            latest_due(
                Repeat::Weekly {
                    weekday: 0,
                    hour: 9,
                    minute: 0
                },
                &now
            ),
            ms("2026-11-09T09:00:00+08:00[Asia/Shanghai]")
        );
        // Wednesday 09:00 is later today: last week's.
        assert_eq!(
            latest_due(
                Repeat::Weekly {
                    weekday: 2,
                    hour: 9,
                    minute: 0
                },
                &now
            ),
            ms("2026-11-04T09:00:00+08:00[Asia/Shanghai]")
        );
        let later = at("2026-11-12T00:00:00+08:00[Asia/Shanghai]")
            .timestamp()
            .as_millisecond();
        assert_eq!(latest_due(Repeat::Once { at: later }, &now), None);
    }

    #[test]
    fn missed_runs_catch_up_once_or_are_noted() {
        let now = at("2026-11-11T10:00:00+08:00[Asia/Shanghai]");
        let due = at("2026-11-11T09:00:00+08:00[Asia/Shanghai]")
            .timestamp()
            .as_millisecond();
        let started = now.timestamp().as_millisecond() - 60_000;
        let daily = Repeat::Daily { hour: 9, minute: 0 };
        // Closed at 09:00, started at 09:59.
        assert_eq!(
            decide(&schedule(daily, true), None, started, &now),
            Decision::Run { due }
        );
        assert_eq!(
            decide(&schedule(daily, false), None, started, &now),
            Decision::Missed { due }
        );
        // Handled once, never again.
        assert_eq!(
            decide(&schedule(daily, true), Some(due), started, &now),
            Decision::Wait
        );
        // Reached while running.
        let early = due - 3_600_000;
        assert_eq!(
            decide(&schedule(daily, false), None, early, &now),
            Decision::Run { due }
        );
        // Made after its time today: not due until tomorrow.
        let mut fresh = schedule(daily, true);
        fresh.created_at = due + 1;
        assert_eq!(decide(&fresh, None, early, &now), Decision::Wait);
        let mut off = schedule(daily, true);
        off.enabled = false;
        assert_eq!(decide(&off, None, early, &now), Decision::Wait);
    }
    #[test]
    fn a_due_time_can_be_reserved_only_once_even_after_restart() {
        use rove_journal::SqliteJournal;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        let first = SqliteJournal::open(&path).unwrap();
        let second = SqliteJournal::open(&path).unwrap();
        std::thread::scope(|scope| {
            let left = scope.spawn(|| reserve_run(&first, "task", 100));
            let right = scope.spawn(|| reserve_run(&second, "task", 100));
            assert_ne!(left.join().unwrap().is_ok(), right.join().unwrap().is_ok());
        });
        drop((first, second));
        let restarted = SqliteJournal::open(&path).unwrap();
        assert!(reserve_run(&restarted, "task", 100).is_err());
        assert!(reserve_run(&restarted, "task", 200).is_ok());
    }
}

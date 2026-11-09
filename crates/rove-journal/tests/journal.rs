use std::sync::Arc;
use std::time::{Duration, Instant};

use rove_journal::*;
use rove_kernel::EventBus;
use rove_session::{ExitStatus, SessionExited, SessionId, SessionInfo, SessionKind, SessionOpened};
use serde_json::json;

fn record(kind: &str, session: Option<&str>) -> NewRecord {
    NewRecord {
        kind: kind.into(),
        session_id: session.map(str::to_owned),
        payload: json!({ "n": kind }),
    }
}

#[test]
fn append_and_filter() {
    let journal = SqliteJournal::open_in_memory().unwrap();
    let first = journal.append(record("a/x", Some("s1"))).unwrap();
    journal.append(record("a/y", Some("s2"))).unwrap();
    journal.append(record("a/x", Some("s1"))).unwrap();

    let all = journal.read(&JournalQuery::default()).unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.windows(2).all(|pair| pair[0].seq < pair[1].seq));

    let s1 = journal
        .read(&JournalQuery {
            session_id: Some("s1".into()),
            ..JournalQuery::default()
        })
        .unwrap();
    assert_eq!(s1.len(), 2);

    let after = journal
        .read(&JournalQuery {
            after_seq: Some(first),
            kind: Some("a/x".into()),
            limit: Some(10),
            ..JournalQuery::default()
        })
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].payload, json!({ "n": "a/x" }));
}

#[test]
fn rejects_future_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    drop(SqliteJournal::open(&path).unwrap());
    rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    assert!(matches!(
        SqliteJournal::open(&path),
        Err(JournalError::FutureSchema(99))
    ));
}

#[test]
fn recorder_persists_session_lifecycle() {
    let journal = Arc::new(SqliteJournal::open_in_memory().unwrap());
    let events = EventBus::default();
    let recorder = Recorder::start(journal.clone()).unwrap();
    let _subscriptions = recorder.subscribe(&events);

    let id = SessionId::new();
    events.emit(&SessionOpened {
        id,
        info: SessionInfo {
            kind: SessionKind::Local {
                shell: "/bin/sh".into(),
            },
            title: "sh".into(),
        },
    });
    events.emit(&SessionExited {
        id,
        status: ExitStatus {
            code: Some(0),
            signal: None,
        },
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let records = loop {
        let records = journal.read(&JournalQuery::default()).unwrap();
        if records.len() == 2 || Instant::now() > deadline {
            break records;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let kinds: Vec<_> = records.iter().map(|record| record.kind.as_str()).collect();
    assert_eq!(kinds, [KIND_SESSION_OPENED, KIND_SESSION_EXITED]);
    assert_eq!(
        records[0].session_id.as_deref(),
        Some(id.to_string().as_str())
    );
    assert_eq!(records[0].payload["info"]["kind"], "local");
    assert_eq!(records[1].payload["status"]["code"], 0);
}

#[test]
fn newest_first_reads_the_latest_record_of_a_kind() {
    let journal = SqliteJournal::open_in_memory().unwrap();
    journal.append(record("ui/layout", None)).unwrap();
    journal.append(record("other", None)).unwrap();
    let last = journal.append(record("ui/layout", None)).unwrap();
    let latest = journal
        .read(&JournalQuery {
            kind: Some("ui/layout".into()),
            limit: Some(1),
            newest_first: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].seq, last);
}

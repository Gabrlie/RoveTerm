//! Keeps native background execution in sync with the mobile profile's active work.

use tauri_plugin_rove_mobile::KeepAliveUpdate;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KeepAliveCounts {
    pub sessions: u32,
    pub forwards: u32,
    pub transfers: u32,
    pub ops_tasks: u32,
    pub ops_waiting: u32,
}

impl KeepAliveCounts {
    pub fn is_active(self) -> bool {
        self.sessions > 0
            || self.forwards > 0
            || self.transfers > 0
            || self.ops_tasks > 0
            || self.ops_waiting > 0
    }

    pub fn to_update(self, zh: bool) -> KeepAliveUpdate {
        let counters = [
            (self.sessions, "session", "sessions", "个终端"),
            (self.forwards, "forward", "forwards", "个转发"),
            (self.transfers, "transfer", "transfers", "个传输"),
        ];
        let mut parts: Vec<String> = counters
            .into_iter()
            .filter(|(count, ..)| *count > 0)
            .map(|(count, singular, plural, chinese)| {
                let label = match (zh, count) {
                    (true, _) => chinese,
                    (false, 1) => singular,
                    (false, _) => plural,
                };
                format!("{count} {label}")
            })
            .collect();
        if self.ops_tasks > 0 {
            let count = self.ops_tasks;
            let waiting = self.ops_waiting.min(count);
            let label = match (zh, count) {
                (true, _) => "个运维任务",
                (false, 1) => "ops task",
                (false, _) => "ops tasks",
            };
            let part = match (zh, waiting) {
                (_, 0) => format!("{count} {label}"),
                (true, _) => format!("{count} {label}（{waiting} 待审批）"),
                (false, _) => format!("{count} {label} ({waiting} awaiting approval)"),
            };
            parts.push(part);
        }
        KeepAliveUpdate {
            active: self.is_active(),
            sessions: self.sessions,
            forwards: self.forwards,
            transfers: self.transfers,
            ops_tasks: self.ops_tasks,
            title: "RoveTerm".into(),
            body: parts.join(" · "),
            disconnect_label: if zh { "断开全部" } else { "Disconnect all" }.into(),
        }
    }
}

#[derive(Default)]
struct UpdateTracker {
    last_sent: Option<KeepAliveUpdate>,
}

impl UpdateTracker {
    fn send_if_changed(
        &mut self,
        update: KeepAliveUpdate,
        send: impl FnOnce(&KeepAliveUpdate) -> bool,
    ) {
        if self.last_sent.as_ref() != Some(&update) && send(&update) {
            self.last_sent = Some(update);
        }
    }
}

#[cfg(mobile)]
mod mobile {
    use std::sync::Arc;

    use rove_kernel::{Context, Disposer, Event};
    use rove_ops::OpsTaskState;
    use rove_vault::Language;
    use tauri::async_runtime;
    use tauri_plugin_rove_mobile::{KeepAliveAction, MobileBridgeExt};
    use tokio::sync::{Mutex, Notify};
    use tokio_util::sync::CancellationToken;

    use super::{KeepAliveCounts, KeepAliveUpdate, UpdateTracker};
    use crate::state::AppState;

    struct Services {
        sessions: Arc<rove_session::SessionManager>,
        forwards: Arc<rove_ssh::Forwards>,
        transfers: Arc<rove_fs::TransferQueue>,
        ops: Arc<rove_ops::OpsSupervisor>,
        vault: Arc<rove_vault::Vault>,
        ssh: Arc<rove_ssh::SshConnector>,
    }

    fn active_ops_task(state: &OpsTaskState) -> bool {
        matches!(state, OpsTaskState::Running | OpsTaskState::WaitingApproval)
    }

    impl Services {
        fn snapshot(&self) -> KeepAliveCounts {
            fn count(rows: impl Iterator) -> u32 {
                u32::try_from(rows.count()).unwrap_or(u32::MAX)
            }

            let ops = self.ops.list().unwrap_or_default();
            KeepAliveCounts {
                sessions: count(
                    self.sessions
                        .list()
                        .into_iter()
                        .filter(|s| s.exited.is_none()),
                ),
                forwards: count(
                    self.forwards
                        .list()
                        .into_iter()
                        .filter(|f| f.state == rove_ssh::ForwardState::Running),
                ),
                transfers: count(
                    self.transfers
                        .list()
                        .into_iter()
                        .filter(|t| !t.state.is_final()),
                ),
                ops_tasks: count(ops.iter().filter(|t| active_ops_task(&t.state))),
                ops_waiting: count(
                    ops.iter()
                        .filter(|t| t.state == OpsTaskState::WaitingApproval),
                ),
            }
        }

        fn chinese(&self) -> bool {
            match self.vault.preferences() {
                Ok(prefs) => prefs.appearance.language == Language::ZhCn,
                Err(error) => {
                    tracing::warn!(%error, "could not read keepalive notification language");
                    false
                }
            }
        }

        async fn disconnect_all(&self) {
            self.transfers.cancel_all();
            self.forwards.stop_all();
            match self.ops.list() {
                Ok(tasks) => {
                    for task in tasks
                        .into_iter()
                        .filter(|task| active_ops_task(&task.state))
                    {
                        if let Err(error) = self.ops.cancel(task.id) {
                            tracing::warn!(%error, "could not cancel operations task");
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "could not list operations tasks to cancel"),
            }
            self.sessions.close_all().await;
            self.ssh.disconnect_all().await;
        }
    }

    fn wake_on<E: Event>(ctx: &Context, notify: &Arc<Notify>) -> Disposer {
        let notify = notify.clone();
        ctx.events().on::<E>(move |_| notify.notify_one())
    }

    fn send_update(app: &tauri::AppHandle, update: &KeepAliveUpdate) -> bool {
        match app.rove_mobile().update_keepalive(update) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "could not update mobile keepalive");
                false
            }
        }
    }

    pub fn start(app: tauri::AppHandle, ctx: &Context, state: &AppState) -> Vec<Disposer> {
        let services = Arc::new(Services {
            sessions: state.sessions.clone(),
            forwards: state.forwards.clone(),
            transfers: state.transfers.clone(),
            ops: state.ops.clone(),
            vault: state.vault.clone(),
            ssh: state.ssh.clone(),
        });
        let notify = Arc::new(Notify::new());
        let mut disposers = vec![
            wake_on::<rove_session::SessionOpened>(ctx, &notify),
            wake_on::<rove_session::SessionExited>(ctx, &notify),
            wake_on::<rove_session::SessionClosed>(ctx, &notify),
            wake_on::<rove_ssh::ForwardStatus>(ctx, &notify),
            wake_on::<rove_fs::TransferUpdate>(ctx, &notify),
        ];

        let mut ops_changes = services.ops.subscribe();
        let ops_wake = notify.clone();
        let ops_watch = async_runtime::spawn(async move {
            while ops_changes.changed().await.is_ok() {
                ops_wake.notify_one();
            }
        });
        disposers.push(Disposer::new(move || ops_watch.abort()));

        let stopped = CancellationToken::new();
        // Serialize native updates with disconnect and teardown, including an in-flight native call.
        let updates = Arc::new(Mutex::new(UpdateTracker::default()));
        let worker = {
            let (app, services, notify, stopped, updates) = (
                app.clone(),
                services.clone(),
                notify.clone(),
                stopped.clone(),
                updates.clone(),
            );
            async_runtime::spawn(async move {
                loop {
                    let tracker = updates.clone().lock_owned().await;
                    let (app, services, stopped) = (app.clone(), services.clone(), stopped.clone());
                    let job = async_runtime::spawn_blocking(move || {
                        let mut tracker = tracker;
                        if !stopped.is_cancelled() {
                            let update = services.snapshot().to_update(services.chinese());
                            tracker.send_if_changed(update, |update| send_update(&app, update));
                        }
                    });
                    if let Err(error) = job.await {
                        tracing::warn!(%error, "mobile keepalive worker failed to update");
                    }
                    notify.notified().await;
                }
            })
        };
        disposers.push(Disposer::new(move || worker.abort()));

        let action_app = app.clone();
        let (action_updates, action_stopped) = (updates.clone(), stopped.clone());
        app.rove_mobile()
            .set_keepalive_action_handler(Some(Arc::new(move |action| {
                let KeepAliveAction::DisconnectAll = action;
                let app = action_app.clone();
                let (services, notify, updates, stopped) = (
                    services.clone(),
                    notify.clone(),
                    action_updates.clone(),
                    action_stopped.clone(),
                );
                async_runtime::spawn(async move {
                    stopped
                        .run_until_cancelled(async {
                            let mut tracker = updates.lock_owned().await;
                            services.disconnect_all().await;
                            // An update already in flight when Android stopped the service may
                            // have restarted it. Clear the notification as soon as work is idle.
                            let result = async_runtime::spawn_blocking(move || {
                                if tracker.last_sent.as_ref().is_some_and(|update| update.active)
                                    && !services.snapshot().is_active()
                                {
                                    let update = KeepAliveCounts::default().to_update(services.chinese());
                                    tracker.send_if_changed(update, |update| send_update(&app, update));
                                }
                            })
                            .await;
                            if let Err(error) = result {
                                tracing::warn!(%error, "mobile keepalive worker failed to clear notification");
                            }
                            notify.notify_one();
                        })
                        .await;
                });
            })));
        disposers.push(Disposer::new(move || {
            stopped.cancel();
            app.rove_mobile().set_keepalive_action_handler(None);
            async_runtime::spawn(async move {
                let _tracker = updates.lock().await;
                let result = async_runtime::spawn_blocking(move || {
                    send_update(&app, &KeepAliveCounts::default().to_update(false));
                })
                .await;
                if let Err(error) = result {
                    tracing::warn!(%error, "mobile keepalive worker failed to stop");
                }
            });
        }));
        disposers
    }
}

#[cfg(mobile)]
pub use mobile::start;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_nonzero_count_is_active() {
        let empty = KeepAliveCounts::default();
        assert!(!empty.is_active());
        for counts in [
            KeepAliveCounts {
                sessions: 1,
                ..empty
            },
            KeepAliveCounts {
                forwards: 1,
                ..empty
            },
            KeepAliveCounts {
                transfers: 1,
                ..empty
            },
            KeepAliveCounts {
                ops_tasks: 1,
                ..empty
            },
            KeepAliveCounts {
                ops_waiting: 1,
                ..empty
            },
        ] {
            assert!(counts.is_active(), "{counts:?}");
        }
    }

    #[test]
    fn english_copy_uses_singular_and_plural_for_each_count() {
        let single = KeepAliveCounts {
            sessions: 1,
            forwards: 1,
            transfers: 1,
            ops_tasks: 1,
            ops_waiting: 0,
        };
        assert_eq!(
            single.to_update(false),
            KeepAliveUpdate {
                active: true,
                sessions: 1,
                forwards: 1,
                transfers: 1,
                ops_tasks: 1,
                title: "RoveTerm".into(),
                body: "1 session · 1 forward · 1 transfer · 1 ops task".into(),
                disconnect_label: "Disconnect all".into(),
            }
        );
        let plural = KeepAliveCounts {
            sessions: 2,
            forwards: 3,
            transfers: 4,
            ops_tasks: 5,
            ops_waiting: 0,
        };
        assert_eq!(
            plural.to_update(false).body,
            "2 sessions · 3 forwards · 4 transfers · 5 ops tasks"
        );
    }

    #[test]
    fn chinese_copy_uses_counters_and_both_languages_omit_zero_counts() {
        let counts = KeepAliveCounts {
            sessions: 1,
            forwards: 2,
            transfers: 3,
            ops_tasks: 4,
            ops_waiting: 0,
        };
        let zh = counts.to_update(true);
        assert_eq!(zh.title, "RoveTerm");
        assert_eq!(zh.body, "1 个终端 · 2 个转发 · 3 个传输 · 4 个运维任务");
        assert_eq!(zh.disconnect_label, "断开全部");

        let partial = KeepAliveCounts {
            forwards: 2,
            ops_tasks: 1,
            ..KeepAliveCounts::default()
        };
        assert_eq!(partial.to_update(false).body, "2 forwards · 1 ops task");
        assert_eq!(partial.to_update(true).body, "2 个转发 · 1 个运维任务");
        for zh in [false, true] {
            let empty = KeepAliveCounts::default().to_update(zh);
            assert!(!empty.active);
            assert!(empty.body.is_empty());
        }
    }

    #[test]
    fn ops_copy_includes_the_waiting_count_in_both_languages() {
        for (ops_tasks, ops_waiting, en, zh) in [
            (
                1,
                1,
                "1 ops task (1 awaiting approval)",
                "1 个运维任务（1 待审批）",
            ),
            (
                3,
                2,
                "3 ops tasks (2 awaiting approval)",
                "3 个运维任务（2 待审批）",
            ),
        ] {
            let counts = KeepAliveCounts {
                ops_tasks,
                ops_waiting,
                ..KeepAliveCounts::default()
            };
            assert_eq!(counts.to_update(false).body, en);
            assert_eq!(counts.to_update(true).body, zh);
            assert_eq!(counts.to_update(false).ops_tasks, ops_tasks);
        }
    }

    #[test]
    fn ops_copy_clamps_the_waiting_count_to_the_task_count() {
        let counts = KeepAliveCounts {
            ops_tasks: 1,
            ops_waiting: 3,
            ..KeepAliveCounts::default()
        };
        assert_eq!(
            counts.to_update(false).body,
            "1 ops task (1 awaiting approval)"
        );
        assert_eq!(counts.to_update(true).body, "1 个运维任务（1 待审批）");
        assert_eq!(counts.to_update(false).ops_tasks, 1);
    }

    #[test]
    fn tracker_sends_an_update_when_a_task_starts_waiting_for_approval() {
        let mut tracker = UpdateTracker::default();
        let running = KeepAliveCounts {
            ops_tasks: 1,
            ..KeepAliveCounts::default()
        };
        let waiting = KeepAliveCounts {
            ops_waiting: 1,
            ..running
        };
        let mut bodies = Vec::new();
        for counts in [running, waiting, waiting] {
            tracker.send_if_changed(counts.to_update(false), |update| {
                assert_eq!(update.ops_tasks, 1);
                bodies.push(update.body.clone());
                true
            });
        }
        assert_eq!(bodies, ["1 ops task", "1 ops task (1 awaiting approval)"]);
    }

    #[test]
    fn tracker_sends_transitions_once_and_includes_language_changes() {
        let mut tracker = UpdateTracker::default();
        let mut sent = Vec::new();
        let idle = KeepAliveCounts::default();
        let active = KeepAliveCounts {
            sessions: 1,
            ..idle
        };
        let expanded = KeepAliveCounts {
            transfers: 1,
            ..active
        };
        let transitions = [
            idle.to_update(false),
            active.to_update(false),
            expanded.to_update(false),
            expanded.to_update(true),
            idle.to_update(true),
        ];
        for update in &transitions {
            for _ in 0..2 {
                tracker.send_if_changed(update.clone(), |update| {
                    sent.push(update.clone());
                    true
                });
            }
        }
        assert_eq!(sent, transitions);
    }

    #[test]
    fn tracker_retries_an_update_after_a_dispatch_failure() {
        let mut tracker = UpdateTracker::default();
        let update = KeepAliveCounts {
            sessions: 1,
            ..KeepAliveCounts::default()
        }
        .to_update(false);
        let mut attempts = 0;
        for success in [false, true, true] {
            tracker.send_if_changed(update.clone(), |_| {
                attempts += 1;
                success
            });
        }
        assert_eq!(attempts, 2);
    }
}

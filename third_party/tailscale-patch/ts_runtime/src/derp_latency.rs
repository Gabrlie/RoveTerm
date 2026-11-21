use std::{sync::Arc, time::Duration};

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::time::{Instant, sleep_until, timeout};

use kameo::{
    actor::ActorRef,
    message::{Context, Message},
};
use ts_netcheck::RegionResult;

use crate::{Error, env::Env};

#[derive(Clone, PartialEq, Eq)]
pub struct DerpLatencyMeasurement {
    pub measurement: Arc<Vec<RegionResult>>,
}

pub struct DerpLatencyMeasurer {
    env: Env,
}

impl kameo::Actor for DerpLatencyMeasurer {
    type Args = Env;
    type Error = Error;

    async fn on_start(env: Env, slf: ActorRef<Self>) -> Result<Self, Self::Error> {
        env.subscribe::<Arc<ts_control::StateUpdate>>(&slf).await?;
        env.register(None, &slf).await?;

        tracing::trace!("derp latency measurer running");

        Ok(Self { env })
    }
}

impl Message<Arc<ts_control::StateUpdate>> for DerpLatencyMeasurer {
    type Reply = ();

    async fn handle(
        &mut self,
        state_update: Arc<ts_control::StateUpdate>,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let Some(derp_map) = &state_update.derp else {
            return;
        };

        tracing::trace!("new derp map: beginning measurement");

        let latencies = measure_regions(derp_map).await;
        if let Some(home) = latencies.first() {
            tracing::info!(region_id = %home.id, latency_ms = home.latency.as_millis(), "selected home DERP");
        }

        tracing::trace!(?latencies, "measurement complete");

        if let Err(e) = self
            .env
            .publish(DerpLatencyMeasurement {
                measurement: Arc::new(latencies),
            })
            .await
        {
            tracing::error!(error = %e, "publishing");
        };
    }
}

async fn measure_regions(map: &ts_control::DerpMap) -> Vec<RegionResult> {
    let start = Instant::now();
    let mut probes = map
        .iter()
        .filter(|(_, region)| !region.info.no_measure_no_home)
        .map(|(&id, region)| async move {
            let probe = ts_netcheck::measure_https_latency(&region.servers, Default::default());
            let (latency, _, connected_remote) =
                timeout(Duration::from_secs(3), probe).await.ok()??;
            Some(RegionResult {
                latency,
                id,
                latency_map_key: format!("{id}-v4"),
                connected_remote,
            })
        })
        .collect::<FuturesUnordered<_>>();
    let mut results = Vec::new();
    let fast = start + Duration::from_millis(250);
    let fallback = start + Duration::from_millis(500);
    let deadline = start + Duration::from_millis(3500);

    loop {
        let now = Instant::now();
        if (results.len() >= 3 && now >= fast) || (!results.is_empty() && now >= fallback) {
            break;
        }
        if probes.is_empty() {
            break;
        }
        let next_deadline = if results.len() >= 3 {
            fast
        } else if results.is_empty() {
            deadline
        } else {
            fallback
        };
        tokio::select! {
            biased;
            _ = sleep_until(deadline) => break,
            _ = sleep_until(next_deadline), if next_deadline != deadline => {},
            Some(result) = probes.next(), if !probes.is_empty() => {
                results.extend(result);
            }
        }
    }
    results.sort();
    results
}

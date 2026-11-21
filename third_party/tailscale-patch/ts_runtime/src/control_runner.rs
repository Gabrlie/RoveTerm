use core::net::{Ipv4Addr, Ipv6Addr};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

use http::{HeaderName, HeaderValue, StatusCode};
use tokio::sync::watch;
use ts_capabilityversion::CapabilityVersion;
use ts_control_serde::{HostInfo, RegisterAuth, RegisterRequest};
use ts_http_util::{ClientExt, ResponseExt};
use zeroize::Zeroizing;

use futures::StreamExt;
use kameo::{
    actor::{ActorRef, Spawn, WeakActorRef},
    message::{Context, StreamMessage},
    prelude::{Message, ReplySender},
    reply::DelegatedReply,
    supervision::RestartPolicy,
};
use ts_control::{
    ControlDialer, DialPlan, Endpoint, Error as ControlError, Node, StateUpdate,
    client::{HttpConn, handle_ping, map_stream, send_map_request},
};

use crate::{
    Task,
    derp_latency::{DerpLatencyMeasurement, DerpLatencyMeasurer},
    direct,
    env::Env,
};

/// Actor responsible for maintaining the connection to control.
///
/// This actor is responsible for proxying the map response stream onto the message bus.
pub struct ControlRunner {
    params: Params,
    state: RegState,
    auth_status: watch::Sender<AuthStatus>,

    derp_latency_measurement: Option<DerpLatencyMeasurement>,

    endpoints: Vec<Endpoint>,
    derp_regions: BTreeMap<ts_derp::RegionId, crate::DerpRegion>,

    self_node: Option<Node>,
    pending_node_requests: Vec<PendingNodeRequest>,
}

/// The response to [`AuthUrl`]: what the state of control server auth is.
#[derive(Debug, Clone, PartialEq, Eq, kameo::Reply)]
pub enum AuthResponse {
    /// The device is authorized.
    Authorized,
    /// The device is not authorized and an interactive auth URL is provided.
    InteractiveAuthRequired(url::Url),
    /// The device is not authorized because the key was rejected.
    ///
    /// Interactive auth is impossible (control server doesn't supply a URL in this case).
    KeyRejected,
}

/// Current control server authorization state.
#[derive(Clone, PartialEq, Eq, kameo::Reply)]
pub enum AuthStatus {
    /// Awaiting an initial registration response.
    Starting,
    /// Browser login is required.
    InteractiveAuthRequired(url::Url),
    /// A tailnet administrator must approve this machine.
    NeedsMachineAuth,
    /// The control plane accepted registration.
    Authorized,
    /// The control plane rejected the key and did not offer a login URL.
    KeyRejected,
}

enum RegState {
    NotRegistered {
        pending_auth_requests: Vec<ReplySender<AuthResponse>>,
    },
    AuthRequired(url::Url),
    Registered(HttpConn),
    AuthKeyRejected,
}

impl RegState {
    /// Update this to `new_state`, sending `value_for_pending` to any pending auth
    /// requests.
    fn update(&mut self, new_state: Self, value_for_pending: AuthResponse) {
        let old_state = core::mem::replace(self, new_state);

        if let RegState::NotRegistered {
            pending_auth_requests,
        } = old_state
        {
            for req in pending_auth_requests {
                req.send(value_for_pending.clone());
            }
        }
    }
}

/// Control runner args.
#[derive(Clone)]
pub struct Params {
    /// Control config.
    pub(crate) config: ts_control::Config,

    /// Auth key (if needed).
    pub(crate) auth_key: Option<Arc<Zeroizing<String>>>,

    /// The [`Env`] for this actor.
    pub(crate) env: Env,
}

#[doc(hidden)]
#[derive(Debug, Clone, thiserror::Error)]
pub enum ControlRunnerError {
    #[error(transparent)]
    Control(#[from] ControlError),

    #[error(transparent)]
    Crate(#[from] crate::Error),
}

#[derive(Clone)]
struct AuthRequired(Option<url::Url>);

struct RegisterResult(Result<HttpConn, RegisterFailure>);

#[derive(Debug, thiserror::Error)]
enum RegisterFailure {
    #[error("control connection failed")]
    Network,
    #[error("control returned HTTP {0}")]
    Status(StatusCode),
    #[error("invalid registration request")]
    Request,
    #[error("invalid registration response")]
    Response,
    #[error("registration rejected")]
    Rejected,
}

impl RegisterFailure {
    fn retryable(&self) -> bool {
        match self {
            Self::Network => true,
            Self::Status(status) => {
                status.is_server_error()
                    || matches!(
                        *status,
                        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
                    )
            }
            _ => false,
        }
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct RegisterResponse {
    machine_authorized: Option<bool>,
    node_key_expired: Option<bool>,
    #[serde(rename = "AuthURL")]
    auth_url: Option<String>,
    error: Option<String>,
}

async fn register(
    config: &ts_control::Config,
    keys: &ts_keys::NodeState,
    auth_key: Option<&str>,
    followup: Option<url::Url>,
    client: &HttpConn,
    attempt: u64,
) -> Result<RegisterResponse, RegisterFailure> {
    let node_key = keys.node_keys.public;
    let app = config.format_client_name();
    let request = RegisterRequest {
        version: CapabilityVersion::CURRENT,
        node_key,
        hostinfo: HostInfo {
            hostname: config.hostname.as_deref(),
            app: &app,
            ipn_version: ts_control::PKG_VERSION,
            ..Default::default()
        },
        nl_key: Some(keys.network_lock_keys.public),
        auth: auth_key.map(RegisterAuth::from),
        followup,
        ephemeral: config.ephemeral,
        ..Default::default()
    };
    let body = serde_json::to_vec(&request).map_err(|_| RegisterFailure::Request)?;
    let url = config
        .server_url
        .join("machine/register")
        .map_err(|_| RegisterFailure::Request)?;
    let node_header =
        HeaderValue::from_str(&node_key.to_string()).map_err(|_| RegisterFailure::Request)?;
    let response = client
        .post(
            &url,
            [(HeaderName::from_static("ts-lb"), node_header)],
            bytes::Bytes::from(body).into(),
        )
        .await
        .map_err(|_| RegisterFailure::Network)?;
    let status = response.status();
    if !status.is_success() {
        tracing::warn!(%status, attempt, "control registration HTTP failure");
        if RegisterFailure::Status(status).retryable() {
            return Err(RegisterFailure::Status(status));
        }
    }
    let body = response
        .collect_bytes()
        .await
        .map_err(|_| RegisterFailure::Network)?;
    let parsed: RegisterResponse = serde_json::from_slice(&body).map_err(|_| {
        if status.is_success() {
            RegisterFailure::Response
        } else {
            RegisterFailure::Status(status)
        }
    })?;
    let machine_authorized = parsed.machine_authorized.unwrap_or(false);
    let has_auth_url = parsed.auth_url.as_ref().is_some_and(|url| !url.is_empty());
    let has_error = parsed.error.as_ref().is_some_and(|error| !error.is_empty());
    tracing::info!(%status, attempt, machine_authorized, has_auth_url, has_error,
        node_key_expired = parsed.node_key_expired.unwrap_or(false), "control registration response");
    if !status.is_success() && !has_error {
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
            && auth_key.is_some()
        {
            return Err(RegisterFailure::Rejected);
        }
        return Err(RegisterFailure::Status(status));
    }
    Ok(parsed)
}

async fn register_until_authorized(
    actor: WeakActorRef<ControlRunner>,
    env: Env,
    config: ts_control::Config,
    mut auth_key: Option<Arc<Zeroizing<String>>>,
) -> Result<HttpConn, RegisterFailure> {
    let mut client = None;
    let mut followup = None;
    let mut pending = false;
    let mut retries = 0;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let request_timeout = Duration::from_secs(match (followup.is_some(), pending) {
            (true, _) => 90,
            (false, true) => 12,
            (false, false) => 12,
        });
        let result = tokio::time::timeout(request_timeout, async {
            if client.is_none() {
                let conn = env
                    .ask::<DialerActor, _>(
                        None,
                        DialNext {
                            url: config.server_url.clone(),
                            timeout: Duration::from_secs(8),
                        },
                        true,
                    )
                    .await
                    .map_err(|_| RegisterFailure::Network)?;
                client = Some(conn);
            }
            let conn = client.as_ref().ok_or(RegisterFailure::Network)?;
            register(
                &config,
                &env.keys,
                auth_key.as_ref().map(|key| key.as_str()),
                followup.clone(),
                conn,
                attempt,
            )
            .await
        })
        .await
        .unwrap_or(Err(RegisterFailure::Network));

        let response = match result {
            Ok(response) => response,
            Err(error) if error.retryable() && (pending || retries < 2) => {
                client = None;
                retries += 1;
                let delay = Duration::from_secs(if pending {
                    (2 << (retries - 1).min(3)).min(10)
                } else {
                    retries
                });
                tracing::warn!(%error, attempt, retry_in_secs = delay.as_secs(), "retrying control registration");
                tokio::time::sleep(delay).await;
                continue;
            }
            Err(error) => return Err(error),
        };

        if response.machine_authorized == Some(true) {
            return client.ok_or(RegisterFailure::Network);
        }
        if response
            .error
            .as_ref()
            .is_some_and(|error| !error.is_empty())
        {
            return Err(RegisterFailure::Rejected);
        }
        let next = response
            .auth_url
            .filter(|url| !url.is_empty())
            .map(|url| url::Url::parse(&url).map_err(|_| RegisterFailure::Response))
            .transpose()?;

        // Once the control server accepted the registration into browser-login or manual-approval
        // state, stop re-sending a one-time auth key on subsequent status polls.
        auth_key = None;

        let delay = if next.is_none() {
            Duration::from_secs(4)
        } else if next == followup {
            Duration::from_secs(2)
        } else {
            Duration::ZERO
        };
        actor
            .upgrade()
            .ok_or(RegisterFailure::Network)?
            .tell(AuthRequired(next.clone()))
            .await
            .map_err(|_| RegisterFailure::Network)?;
        followup = next;
        retries = 0;
        pending = true;
        tokio::time::sleep(delay).await;
    }
}

impl kameo::Actor for ControlRunner {
    type Args = Params;
    type Error = ControlRunnerError;

    async fn on_start(mut params: Params, slf: ActorRef<Self>) -> Result<Self, Self::Error> {
        Task::supervise_with(&slf, {
            let aref = slf.downgrade();
            let env = params.env.clone();
            let config = params.config.clone();
            let auth_key = params.auth_key.take();

            move || {
                let aref = aref.clone();
                let env = env.clone();
                let config = config.clone();
                let auth_key = auth_key.clone();

                async move {
                    let result =
                        register_until_authorized(aref.clone(), env, config, auth_key).await;
                    if let Some(actor) = aref.upgrade() {
                        drop(actor.tell(RegisterResult(result)).await);
                    }
                }
            }
        })
        .restart_policy(RestartPolicy::Transient)
        .spawn()
        .await;

        params.env.subscribe::<DerpLatencyMeasurement>(&slf).await?;
        params.env.subscribe::<direct::NewEndpoints>(&slf).await?;

        DerpLatencyMeasurer::supervise(&slf, params.env.clone())
            .spawn()
            .await;

        params.env.register(None, &slf).await?;

        Ok(Self {
            state: RegState::NotRegistered {
                pending_auth_requests: Default::default(),
            },
            auth_status: watch::Sender::new(AuthStatus::Starting),
            params,
            derp_latency_measurement: None,
            endpoints: Default::default(),
            derp_regions: BTreeMap::new(),
            self_node: None,
            pending_node_requests: Default::default(),
        })
    }
}

#[kameo::messages]
impl ControlRunner {
    /// Read the current registration state without waiting for a control server response.
    #[message]
    pub fn query_auth_status(&self) -> AuthStatus {
        self.auth_status.borrow().clone()
    }

    /// Subscribe to registration state changes.
    #[message]
    pub fn subscribe_auth(&self) -> (watch::Receiver<AuthStatus>,) {
        (self.auth_status.subscribe(),)
    }

    /// Read the selected home DERP region.
    #[message]
    pub fn home_derp(&self) -> Option<crate::DerpRegion> {
        let home = self
            .derp_latency_measurement
            .as_ref()?
            .measurement
            .first()?;
        self.derp_regions.get(&home.id).cloned()
    }

    #[message]
    pub(crate) fn probe_config(
        &self,
    ) -> (
        Vec<Endpoint>,
        BTreeMap<ts_derp::RegionId, crate::DerpRegion>,
    ) {
        (self.endpoints.clone(), self.derp_regions.clone())
    }

    /// Fetch the IPv4 address for this tailscale device.
    #[message(ctx)]
    pub fn ipv4(
        &mut self,
        ctx: &mut Context<Self, DelegatedReply<Option<Ipv4Addr>>>,
    ) -> DelegatedReply<Option<Ipv4Addr>> {
        if let Some(node) = &self.self_node {
            return ctx.reply(Some(node.tailnet_address.ipv4.addr()));
        }

        let (deleg, replier) = ctx.reply_sender();
        if let Some(replier) = replier {
            self.pending_node_requests
                .push(PendingNodeRequest::Ipv4(replier));
        }

        deleg
    }

    /// Fetch the IPv6 address for this tailscale device.
    #[message(ctx)]
    pub fn ipv6(
        &mut self,
        ctx: &mut Context<Self, DelegatedReply<Option<Ipv6Addr>>>,
    ) -> DelegatedReply<Option<Ipv6Addr>> {
        if let Some(node) = &self.self_node {
            return ctx.reply(Some(node.tailnet_address.ipv6.addr()));
        }

        let (deleg, replier) = ctx.reply_sender();
        if let Some(replier) = replier {
            self.pending_node_requests
                .push(PendingNodeRequest::Ipv6(replier));
        }

        deleg
    }

    /// Fetch the self node for this tailscale device.
    #[message(ctx)]
    pub fn self_node(
        &mut self,
        ctx: &mut Context<Self, DelegatedReply<Option<Node>>>,
    ) -> DelegatedReply<Option<Node>> {
        if let Some(node) = &self.self_node {
            return ctx.reply(Some(node.clone()));
        }

        let (deleg, replier) = ctx.reply_sender();
        if let Some(replier) = replier {
            self.pending_node_requests
                .push(PendingNodeRequest::SelfNode(replier));
        }

        deleg
    }

    /// Wait for a report of whether interactive auth is needed, and if so, what the URL is.
    #[message(ctx)]
    pub fn auth_url(
        &mut self,
        ctx: &mut Context<Self, DelegatedReply<AuthResponse>>,
    ) -> DelegatedReply<AuthResponse> {
        match &mut self.state {
            RegState::Registered(..) => ctx.reply(AuthResponse::Authorized),
            RegState::AuthRequired(u) => {
                ctx.reply(AuthResponse::InteractiveAuthRequired(u.clone()))
            }
            RegState::NotRegistered {
                pending_auth_requests,
            } => {
                let (deleg, replier) = ctx.reply_sender();
                if let Some(replier) = replier {
                    pending_auth_requests.push(replier);
                }

                deleg
            }
            RegState::AuthKeyRejected => ctx.reply(AuthResponse::KeyRejected),
        }
    }
}

impl ControlRunner {
    /// Call `f` with a map request built from the current control actor state.
    ///
    /// `stream` dictates whether the request is built for a streaming netmap response or as a
    /// request to update to this node's fields in control.
    ///
    /// This takes a closure rather than returning the built request for lifetime reasons.
    async fn with_map_request<T>(
        &self,
        stream: bool,
        f: impl AsyncFnOnce(ts_control::MapRequest) -> T,
    ) -> T {
        let mut mrb = ts_control::MapRequestBuilder::new(&self.params.env.keys);

        mrb = if stream {
            mrb.as_stream()
        } else {
            mrb.as_request()
        };

        mrb = mrb.endpoints(self.endpoints.clone());

        if let Some(hostname) = self.params.config.hostname.as_deref() {
            mrb = mrb.hostname(hostname);
        }

        if let Some(latency) = &self.derp_latency_measurement {
            if let Some(result) = latency.measurement.first() {
                mrb = mrb.preferred_derp(result.id);
            };

            let iter = latency.measurement.iter().map(|result| {
                (
                    result.latency_map_key.as_str(),
                    result.latency.as_secs_f64(),
                )
            });

            mrb = mrb.derp_latencies(iter);
        }

        let client_name = self.params.config.format_client_name();

        let mut request = mrb.build();
        let host_info = request.host_info.get_or_insert_default();
        host_info.app = &client_name;
        host_info.ipn_version = ts_control::PKG_VERSION;

        f(request).await
    }

    async fn update_map_request(&self) {
        let RegState::Registered(conn) = &self.state else {
            tracing::debug!("attempt to update map request while not registered");
            return;
        };

        let Ok(map_url) = self.params.config.server_url.join("machine/map") else {
            tracing::warn!("invalid control map URL");
            return;
        };
        if self
            .with_map_request(false, async |req| {
                send_map_request(req, &map_url, conn).await
            })
            .await
            .is_err()
        {
            tracing::warn!("unable to update control map request");
        }
    }
}

impl Message<AuthRequired> for ControlRunner {
    type Reply = ();

    async fn handle(
        &mut self,
        AuthRequired(auth_url): AuthRequired,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        if !matches!(
            self.state,
            RegState::NotRegistered { .. } | RegState::AuthRequired(_)
        ) {
            return;
        }
        let next = if let Some(auth_url) = auth_url {
            self.state.update(
                RegState::AuthRequired(auth_url.clone()),
                AuthResponse::InteractiveAuthRequired(auth_url.clone()),
            );
            AuthStatus::InteractiveAuthRequired(auth_url)
        } else {
            if matches!(self.state, RegState::AuthRequired(_)) {
                self.state = RegState::NotRegistered {
                    pending_auth_requests: Vec::new(),
                };
            }
            AuthStatus::NeedsMachineAuth
        };
        self.auth_status.send_if_modified(|status| {
            if *status == next {
                return false;
            }
            *status = next;
            true
        });
    }
}

impl Message<RegisterResult> for ControlRunner {
    type Reply = ();

    async fn handle(&mut self, msg: RegisterResult, ctx: &mut Context<Self, Self::Reply>) {
        if matches!(self.state, RegState::Registered(..)) {
            tracing::warn!("got register result after already in registered state");
            return;
        }

        let conn = match msg.0 {
            Ok(conn) => conn,
            Err(RegisterFailure::Rejected) => {
                tracing::warn!("control rejected registration");
                self.state
                    .update(RegState::AuthKeyRejected, AuthResponse::KeyRejected);
                self.auth_status.send_replace(AuthStatus::KeyRejected);

                return;
            }
            Err(error) => {
                tracing::error!(%error, "unable to register with control server");
                ctx.stop();
                return;
            }
        };

        self.state
            .update(RegState::Registered(conn.clone()), AuthResponse::Authorized);
        self.auth_status.send_replace(AuthStatus::Authorized);

        let Ok(map_url) = self.params.config.server_url.join("machine/map") else {
            tracing::error!("invalid control map URL");
            ctx.stop();
            return;
        };
        let reader = self
            .with_map_request(true, async |req| {
                send_map_request(req, &map_url, &conn).await
            })
            .await;
        let Ok(reader) = reader else {
            tracing::error!("unable to subscribe to the control server map");
            ctx.stop();
            return;
        };

        let stream = map_stream(reader).map(Arc::new);

        ctx.actor_ref().attach_stream(stream.boxed(), (), ());
    }
}

enum PendingNodeRequest {
    Ipv4(ReplySender<Option<Ipv4Addr>>),
    Ipv6(ReplySender<Option<Ipv6Addr>>),
    SelfNode(ReplySender<Option<Node>>),
}

impl Message<StreamMessage<Arc<StateUpdate>, (), ()>> for ControlRunner {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<Arc<StateUpdate>, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) {
        let msg = match msg {
            StreamMessage::Started(_) => {
                tracing::trace!("started listening to state updates");
                return;
            }

            StreamMessage::Next(msg) => msg,

            StreamMessage::Finished(_) => {
                tracing::error!("state update stream terminated");
                ctx.stop();
                return;
            }
        };

        if let RegState::Registered(conn) = &self.state {
            let _ = handle_ping(&msg, &self.params.config.server_url, conn).await;
        }

        if let Some(dial_plan) = &msg.dial_plan
            && self
                .params
                .env
                .ask::<DialerActor, _>(
                    None,
                    UpdateDialPlan {
                        dial_plan: dial_plan.clone(),
                    },
                    true,
                )
                .await
                .is_ok_and(|changed| changed)
        {
            tracing::trace!(new_dial_plan = ?dial_plan);
        }

        if let Some(node) = msg.node.as_ref() {
            self.self_node = Some(node.clone());
        }

        if let Some(map) = &msg.derp {
            self.derp_regions = map
                .iter()
                .map(|(&id, region)| {
                    (
                        id,
                        crate::DerpRegion {
                            id: id.0.get(),
                            code: region.info.code.clone(),
                            name: region.info.name.clone(),
                        },
                    )
                })
                .collect();
        }

        if let Err(e) = self.params.env.publish(msg).await {
            tracing::error!(error = %e, "publishing netmap update");
        }

        if let Some(node) = &self.self_node {
            for req in self.pending_node_requests.drain(..) {
                match req {
                    PendingNodeRequest::Ipv4(sender) => {
                        sender.send(Some(node.tailnet_address.ipv4.addr()));
                    }
                    PendingNodeRequest::Ipv6(sender) => {
                        sender.send(Some(node.tailnet_address.ipv6.addr()));
                    }
                    PendingNodeRequest::SelfNode(sender) => {
                        sender.send(Some(node.clone()));
                    }
                }
            }
        }
    }
}

impl Message<direct::NewEndpoints> for ControlRunner {
    type Reply = ();

    async fn handle(&mut self, msg: direct::NewEndpoints, _ctx: &mut Context<Self, Self::Reply>) {
        if self.endpoints == msg.0.as_ref() {
            return;
        }

        self.endpoints = msg.0.to_vec();
        self.update_map_request().await;
    }
}

impl Message<DerpLatencyMeasurement> for ControlRunner {
    type Reply = ();

    async fn handle(&mut self, msg: DerpLatencyMeasurement, _ctx: &mut Context<Self, Self::Reply>) {
        if self.derp_latency_measurement.as_ref() == Some(&msg) {
            return;
        }

        self.derp_latency_measurement = Some(msg);
        self.update_map_request().await;
    }
}

/// Control server dialer.
pub struct DialerActor {
    dialer: ControlDialer,
    env: Env,
}

impl kameo::Actor for DialerActor {
    type Args = Env;
    type Error = crate::Error;

    async fn on_start(env: Env, slf: ActorRef<Self>) -> Result<Self, Self::Error> {
        env.register(None, &slf).await?;

        Ok(Self {
            dialer: Default::default(),
            env,
        })
    }
}

#[kameo::messages]
impl DialerActor {
    #[message]
    async fn dial_next(
        &mut self,
        url: url::Url,
        timeout: Duration,
    ) -> Result<HttpConn, ts_control::Error> {
        tokio::time::timeout(
            timeout,
            self.dialer
                .full_connect_next(&url, &self.env.keys.machine_keys),
        )
        .await
        .unwrap_or(Err(ControlError::NetworkError(
            ts_control::Operation::ConnectToControlServer,
        )))
    }

    #[message]
    fn update_dial_plan(&mut self, dial_plan: DialPlan) -> bool {
        self.dialer.update_dial_plan(&dial_plan)
    }
}

#[cfg(test)]
mod tests {
    use super::RegisterResponse;

    #[test]
    fn registration_accepts_missing_and_null_fields() {
        for body in [
            "{}",
            r#"{"User":null,"Login":null,"MachineAuthorized":null,"NodeKeyExpired":null,"AuthURL":null,"Error":null}"#,
            r#"{"MachineAuthorized":false,"AuthURL":"","Error":""}"#,
        ] {
            let result: RegisterResponse = serde_json::from_str(body).unwrap();
            assert!(!result.machine_authorized.unwrap_or(false));
            assert!(result.auth_url.unwrap_or_default().is_empty());
            assert!(result.error.unwrap_or_default().is_empty());
        }
    }

    #[test]
    fn registration_preserves_status_fields() {
        let pending: RegisterResponse = serde_json::from_str(
            r#"{"AuthURL":"https://example.test/authorize","NodeKeyExpired":true}"#,
        )
        .unwrap();
        assert_eq!(
            pending.auth_url.as_deref(),
            Some("https://example.test/authorize")
        );
        assert_eq!(pending.node_key_expired, Some(true));
        let rejected: RegisterResponse = serde_json::from_str(r#"{"Error":"rejected"}"#).unwrap();
        assert_eq!(rejected.error.as_deref(), Some("rejected"));
        let approved: RegisterResponse =
            serde_json::from_str(r#"{"MachineAuthorized":true}"#).unwrap();
        assert_eq!(approved.machine_authorized, Some(true));
    }
}

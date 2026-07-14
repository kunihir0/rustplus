use crate::camera::Camera;
use crate::error::{Error, Result};
use crate::proto::{
    AppCameraInput, AppCameraSubscribe, AppEmpty, AppFlag, AppGetNexusAuth, AppMessage,
    AppPromoteToLeader, AppRequest, AppSendMessage, AppSetEntityValue, Vector2,
};
use crate::ratelimit::RateLimiter;
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::{
    connect_async, tungstenite::client::IntoClientRequest,
    tungstenite::protocol::Message as WsMessage,
};
use tracing::{debug, error, info, warn};

type RequestMap = Arc<Mutex<HashMap<u32, oneshot::Sender<AppMessage>>>>;

/// Client for connecting to a Rust+ server.
#[derive(Clone)]
pub struct RustPlusClient {
    server: String,
    port: u16,
    player_id: u64,
    player_token: i32,
    use_facepunch_proxy: bool,

    seq: Arc<Mutex<u32>>,
    requests: RequestMap,
    rate_limiter: Arc<Mutex<RateLimiter>>,

    /// Channel for sending requests to the WebSocket connection task
    tx: Option<mpsc::Sender<AppRequest>>,
    /// Channel for broadcasting incoming messages
    broadcast_tx: Arc<std::sync::Mutex<Option<broadcast::Sender<AppMessage>>>>,
}

#[must_use]
fn get_request_cost(req: &AppRequest) -> f64 {
    if req.get_map.is_some() {
        return 5.0;
    }
    if req.send_team_message.is_some() {
        return 2.0;
    }
    if req.camera_input.is_some() {
        return 0.01;
    }
    1.0
}

impl RustPlusClient {
    /// Creates a new `RustPlusClient`.
    #[must_use]
    pub fn new(
        server: impl Into<String>,
        port: u16,
        player_id: u64,
        player_token: i32,
        use_facepunch_proxy: bool,
    ) -> Self {
        Self {
            server: server.into(),
            port,
            player_id,
            player_token,
            use_facepunch_proxy,
            seq: Arc::new(Mutex::new(0)),
            requests: Arc::new(Mutex::new(HashMap::new())),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new())),
            tx: None,
            broadcast_tx: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Connects to the Rust+ Server via WebSocket.
    ///
    /// # Errors
    /// Returns an error if the WebSocket connection fails.
    #[allow(clippy::too_many_lines)]
    pub async fn connect(&mut self) -> Result<()> {
        let address = if self.use_facepunch_proxy {
            let version = crate::proxy::get_proxy_version().await?;
            format!(
                "wss://companion-rust.facepunch.com/game/{}/{}?v={}",
                self.server, self.port, version
            )
        } else {
            format!("ws://{}:{}", self.server, self.port)
        };

        debug!("Connecting to Rust+ server at {}", address);

        let mut request = address
            .clone()
            .into_client_request()
            .map_err(Error::WebSocket)?;

        if self.use_facepunch_proxy {
            // Community 'Gold' standard headers
            request.headers_mut().insert(
                http::header::ORIGIN,
                http::HeaderValue::from_static("https://rustplus.facepunch.com"),
            );
            request.headers_mut().insert(
                http::header::USER_AGENT,
                http::HeaderValue::from_static("@facepunch/RustCompanion"),
            );
        }

        info!(
            "WebSocket Handshake: {} (Headers: {:?})",
            address,
            request.headers()
        );

        let (ws_stream, _) = connect_async(request).await.map_err(Error::WebSocket)?;
        debug!("Connected to Rust+ server");

        let (mut write, mut read) = ws_stream.split();
        let (tx, mut rx) = mpsc::channel::<AppRequest>(100);
        let (broadcast_tx, _) = broadcast::channel::<AppMessage>(100);

        let requests = Arc::clone(&self.requests);
        let seq_counter = Arc::clone(&self.seq);

        // Reset state on reconnect
        *seq_counter.lock().await = 0;
        requests.lock().await.clear();

        self.tx = Some(tx);
        if let Ok(mut guard) = self.broadcast_tx.lock() {
            *guard = Some(broadcast_tx.clone());
        }

        // Read task
        let read_requests = Arc::clone(&self.requests);
        let self_broadcast_tx = Arc::clone(&self.broadcast_tx);
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(WsMessage::Binary(data)) => match AppMessage::decode(data.as_ref()) {
                        Ok(app_message) => {
                            let mut handled = false;
                            if let Some(response) = &app_message.response {
                                let seq = response.seq;
                                let mut reqs = read_requests.lock().await;
                                if let Some(callback) = reqs.remove(&seq) {
                                    let _ = callback.send(app_message.clone());
                                    handled = true;
                                }
                            }

                            if !handled {
                                let _ = broadcast_tx.send(app_message);
                            }
                        }
                        Err(e) => {
                            warn!("Failed to decode AppMessage: {}", e);
                        }
                    },
                    Ok(WsMessage::Close(_)) => {
                        debug!("WebSocket closed");
                        break;
                    }
                    Ok(_) => {} // Ignore other message types
                    Err(e) => {
                        warn!("WebSocket connection dropped: {}", e);
                        break;
                    }
                }
            }
            if let Ok(mut guard) = self_broadcast_tx.lock() {
                *guard = None;
            }
        });

        // Write task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    req_opt = rx.recv() => {
                        match req_opt {
                            Some(req) => {
                                let mut buf = Vec::new();
                                if let Err(e) = req.encode(&mut buf) {
                                    error!("Failed to encode AppRequest: {}", e);
                                    continue;
                                }
                                if let Err(e) = write.send(WsMessage::Binary(buf.into())).await {
                                    error!("Failed to send AppRequest: {}", e);
                                    break;
                                }
                            }
                            None => break, // tx dropped
                        }
                    }
                    _ = interval.tick() => {
                        if let Err(e) = write.send(WsMessage::Ping(vec![].into())).await {
                            error!("Failed to send Ping: {}", e);
                            break;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Disconnects the client.
    pub fn disconnect(&mut self) {
        self.tx = None;
        if let Ok(mut guard) = self.broadcast_tx.lock() {
            *guard = None;
        }
    }

    /// Checks if the client is connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.tx.is_some()
    }

    /// Takes the broadcast receiver from the client, allowing the consumer to listen for broadcasts.
    #[must_use]
    pub fn take_broadcast_receiver(&self) -> Option<broadcast::Receiver<AppMessage>> {
        self.broadcast_tx.lock().ok().and_then(|guard| {
            guard
                .as_ref()
                .map(tokio::sync::broadcast::Sender::subscribe)
        })
    }

    async fn send_request_inner(&self, mut request: AppRequest) -> Result<AppMessage> {
        let cost = get_request_cost(&request);
        let wait_time = self.rate_limiter.lock().await.acquire(cost);
        if wait_time > Duration::ZERO {
            tokio::time::sleep(wait_time).await;
        }

        let Some(tx) = &self.tx else {
            return Err(Error::Disconnected);
        };

        let mut seq_guard = self.seq.lock().await;
        *seq_guard += 1;
        let seq = *seq_guard;
        drop(seq_guard);

        request.seq = seq;
        request.player_id = self.player_id;
        request.player_token = self.player_token;

        let (res_tx, res_rx) = oneshot::channel();
        self.requests.lock().await.insert(seq, res_tx);

        tx.send(request).await.map_err(|_| Error::Disconnected)?;

        let response = timeout(Duration::from_secs(10), res_rx)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Disconnected)?;

        #[allow(clippy::collapsible_if)]
        if let Some(resp) = &response.response {
            if let Some(err) = &resp.error {
                return Err(Error::Api(err.error.clone()));
            }
        }

        Ok(response)
    }

    /// Send a custom request.
    ///
    /// # Errors
    /// Returns an error if the request times out or the server returns an API error.
    pub async fn send_request(&self, request: AppRequest) -> Result<AppMessage> {
        self.send_request_inner(request).await
    }

    /// Get the server info
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_info(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_info: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Get the ingame time
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_time(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_time: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Get the Map
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_map(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_map: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Get all map markers
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_map_markers(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_map_markers: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Get team info
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_team_info(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_team_info: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Get info for an Entity
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_entity_info(&self, entity_id: u32) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            entity_id: Some(entity_id),
            get_entity_info: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Send a message to Team Chat
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn send_team_message(&self, message: &str) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            send_team_message: Some(AppSendMessage {
                message: message.to_string(),
            }),
            ..Default::default()
        })
        .await
    }

    /// Set an Entity Value (e.g. Smart Switch)
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn set_entity_value(&self, entity_id: u32, value: bool) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            entity_id: Some(entity_id),
            set_entity_value: Some(AppSetEntityValue { value }),
            ..Default::default()
        })
        .await
    }

    /// Turn a Smart Switch On
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn turn_smart_switch_on(&self, entity_id: u32) -> Result<AppMessage> {
        self.set_entity_value(entity_id, true).await
    }

    /// Turn a Smart Switch Off
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn turn_smart_switch_off(&self, entity_id: u32) -> Result<AppMessage> {
        self.set_entity_value(entity_id, false).await
    }

    /// Subscribes to a Camera
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn subscribe_to_camera(&self, identifier: &str) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            camera_subscribe: Some(AppCameraSubscribe {
                camera_id: identifier.to_string(),
            }),
            ..Default::default()
        })
        .await
    }

    /// Unsubscribes from a Camera
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn unsubscribe_from_camera(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            camera_unsubscribe: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Sends camera input to the server (mouse movement)
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn send_camera_input(&self, buttons: i32, x: f32, y: f32) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            camera_input: Some(AppCameraInput {
                buttons,
                mouse_delta: Vector2 {
                    x: Some(x),
                    y: Some(y),
                },
            }),
            ..Default::default()
        })
        .await
    }

    /// Gets a camera instance for controlling CCTV Cameras, PTZ Cameras, and Auto Turrets.
    #[must_use]
    pub fn get_camera(&self, identifier: &str) -> Camera {
        Camera::new(self.clone(), identifier)
    }

    /// Get the team chat history
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_team_chat(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_team_chat: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Promote a team member to leader
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn promote_to_leader(&self, steam_id: u64) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            promote_to_leader: Some(AppPromoteToLeader { steam_id }),
            ..Default::default()
        })
        .await
    }

    /// Check subscription to an entity
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn check_subscription(&self, entity_id: u32) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            entity_id: Some(entity_id),
            check_subscription: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Set subscription to an entity
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn set_subscription(&self, entity_id: u32, value: bool) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            entity_id: Some(entity_id),
            set_subscription: Some(AppFlag { value }),
            ..Default::default()
        })
        .await
    }

    /// Get clan info
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_clan_info(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_clan_info: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Get clan chat history
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_clan_chat(&self) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_clan_chat: Some(AppEmpty {}),
            ..Default::default()
        })
        .await
    }

    /// Send a message to clan chat
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn send_clan_message(&self, message: &str) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            send_clan_message: Some(AppSendMessage {
                message: message.to_string(),
            }),
            ..Default::default()
        })
        .await
    }

    /// Set clan MOTD
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn set_clan_motd(&self, message: &str) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            set_clan_motd: Some(AppSendMessage {
                message: message.to_string(),
            }),
            ..Default::default()
        })
        .await
    }

    /// Get Nexus Auth
    ///
    /// # Errors
    /// Returns an error on timeout or API failure.
    pub async fn get_nexus_auth(&self, app_key: &str) -> Result<AppMessage> {
        self.send_request_inner(AppRequest {
            get_nexus_auth: Some(AppGetNexusAuth {
                app_key: app_key.to_string(),
            }),
            ..Default::default()
        })
        .await
    }
}

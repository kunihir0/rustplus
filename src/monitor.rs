use crate::client::RustPlusClient;
use crate::events::{EventMonitor, RustEvent};
use tokio::sync::broadcast;
use tokio::time::{Duration, interval};
use tracing::warn;

pub struct MonitorLoop {
    client: RustPlusClient,
    monitors: Vec<Box<dyn EventMonitor>>,
    tx: broadcast::Sender<RustEvent>,
}

impl MonitorLoop {
    #[must_use]
    pub fn new(client: RustPlusClient, tx: broadcast::Sender<RustEvent>) -> Self {
        Self {
            client,
            monitors: Vec::new(),
            tx,
        }
    }

    pub fn register(&mut self, monitor: Box<dyn EventMonitor>) {
        self.monitors.push(monitor);
    }

    pub async fn run(mut self) {
        // Tick every 5 seconds.
        let mut ticker = interval(Duration::from_secs(5));

        loop {
            ticker.tick().await;

            let map_markers = match self.client.get_map_markers().await {
                Ok(msg) => msg,
                Err(e) => {
                    warn!("Monitor loop failed to fetch map markers: {}", e);
                    continue;
                }
            };

            let markers = map_markers
                .response
                .and_then(|r| r.map_markers)
                .map(|m| m.markers)
                .unwrap_or_default();

            // Emit the MarkerSnapshot
            let _ = self.tx.send(RustEvent::MarkerSnapshot(markers.clone()));

            // Call all registered monitors
            for monitor in &mut self.monitors {
                let tx = self.tx.clone();
                monitor.on_tick(&markers, &mut |event| {
                    let _ = tx.send(event);
                });
            }
        }
    }
}

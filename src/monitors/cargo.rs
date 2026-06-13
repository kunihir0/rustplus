use crate::events::{EventMonitor, RustEvent};
use crate::proto::{AppMarker, AppMarkerType};
use std::time::Instant;

#[derive(Debug, Default)]
enum CargoPhase {
    #[default]
    Waiting,
    Active {
        spawned_at: Instant,
        last_seen_at: Instant,
    },
}

#[derive(Debug, Default)]
pub struct CargoMonitor {
    phase: CargoPhase,
}

impl CargoMonitor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventMonitor for CargoMonitor {
    fn on_tick(&mut self, markers: &[AppMarker], emit: &mut dyn FnMut(RustEvent)) {
        let cargo_marker = markers
            .iter()
            .find(|m| m.r#type() == AppMarkerType::CargoShip);

        match &mut self.phase {
            CargoPhase::Waiting => {
                if cargo_marker.is_some() {
                    let now = Instant::now();
                    self.phase = CargoPhase::Active {
                        spawned_at: now,
                        last_seen_at: now,
                    };
                    emit(RustEvent::CargoSpawned);
                }
            }
            CargoPhase::Active {
                spawned_at,
                last_seen_at,
            } => {
                if cargo_marker.is_some() {
                    *last_seen_at = Instant::now();
                } else {
                    let was_out_for = spawned_at.elapsed();
                    emit(RustEvent::CargoDespawned { was_out_for });
                    self.phase = CargoPhase::Waiting;
                }
            }
        }
    }
}

use crate::events::{EventMonitor, RustEvent};
use crate::proto::{AppMarker, AppMarkerType};
use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct ExplosionMonitor {
    known_explosions: HashSet<u32>,
    first_tick: bool,
}

impl ExplosionMonitor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            known_explosions: HashSet::new(),
            first_tick: true,
        }
    }
}

impl EventMonitor for ExplosionMonitor {
    fn on_tick(&mut self, markers: &[AppMarker], emit: &mut dyn FnMut(RustEvent)) {
        let current_explosions: HashSet<u32> = markers
            .iter()
            .filter(|m| m.r#type() == AppMarkerType::Explosion)
            .map(|m| m.id)
            .collect();

        if self.first_tick {
            self.known_explosions = current_explosions;
            self.first_tick = false;
            return;
        }

        // Detect new explosions
        for marker in markers {
            if marker.r#type() == AppMarkerType::Explosion
                && !self.known_explosions.contains(&marker.id)
            {
                emit(RustEvent::ExplosionOccurred {
                    position: (marker.x, marker.y),
                });
            }
        }

        self.known_explosions = current_explosions;
    }
}

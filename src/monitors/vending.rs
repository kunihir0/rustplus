use crate::events::{EventMonitor, RustEvent};
use crate::proto::{AppMarker, AppMarkerType};
use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct VendingMonitor {
    known_vending_machines: HashSet<u32>,
    first_tick: bool,
}

impl VendingMonitor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            known_vending_machines: HashSet::new(),
            first_tick: true,
        }
    }
}

impl EventMonitor for VendingMonitor {
    fn on_tick(&mut self, markers: &[AppMarker], emit: &mut dyn FnMut(RustEvent)) {
        let current_vending_machines: HashSet<u32> = markers
            .iter()
            .filter(|m| m.r#type() == AppMarkerType::VendingMachine)
            .map(|m| m.id)
            .collect();

        if self.first_tick {
            self.known_vending_machines = current_vending_machines;
            self.first_tick = false;
            return;
        }

        // Check for new vending machines
        for marker in markers {
            if marker.r#type() == AppMarkerType::VendingMachine
                && !self.known_vending_machines.contains(&marker.id)
            {
                emit(RustEvent::VendingMachineNew {
                    position: (marker.x, marker.y),
                    id: marker.id,
                });
            }
        }

        self.known_vending_machines = current_vending_machines;
    }
}

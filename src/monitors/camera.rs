use crate::events::RustEvent;
use crate::proto::app_camera_rays::EntityType;
use crate::proto::{AppCameraRays, app_camera_rays::Entity};
use std::time::{Duration, Instant};

pub struct CameraMonitor {
    camera_id: String,
    last_entities: Vec<Entity>,
    cooldown: Option<Instant>,
}

impl CameraMonitor {
    #[must_use]
    pub fn new(camera_id: String) -> Self {
        Self {
            camera_id,
            last_entities: Vec::new(),
            cooldown: None,
        }
    }

    pub fn on_rays(&mut self, msg: &AppCameraRays, emit: &mut dyn FnMut(RustEvent)) {
        let players: Vec<_> = msg
            .entities
            .iter()
            .filter(|e| e.r#type == EntityType::Player as i32)
            .collect();

        // New players appeared since last tick
        let new_ids: Vec<u32> = players
            .iter()
            .map(|e| e.entity_id)
            .filter(|id| !self.last_entities.iter().any(|e| &e.entity_id == id))
            .collect();

        if !new_ids.is_empty()
            && self
                .cooldown
                .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
        {
            emit(RustEvent::CameraMotion {
                camera_id: self.camera_id.clone(),
                player_count: players.len(),
                names: players.iter().filter_map(|e| e.name.clone()).collect(),
            });
            self.cooldown = Some(Instant::now());
        }
        self.last_entities.clone_from(&msg.entities);
    }
}

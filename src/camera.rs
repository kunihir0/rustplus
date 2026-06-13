#![allow(clippy::pedantic)]
#![allow(clippy::collapsible_if)]
#![allow(unused_assignments)]

use crate::client::RustPlusClient;
use crate::error::Result;
use crate::proto::{AppCameraInfo, AppCameraRays};
use image::{ImageBuffer, Rgba};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::warn;

pub const BUTTON_NONE: i32 = 0;
pub const BUTTON_FORWARD: i32 = 2;
pub const BUTTON_BACKWARD: i32 = 4;
pub const BUTTON_LEFT: i32 = 8;
pub const BUTTON_RIGHT: i32 = 16;
pub const BUTTON_JUMP: i32 = 32;
pub const BUTTON_DUCK: i32 = 64;
pub const BUTTON_SPRINT: i32 = 128;
pub const BUTTON_USE: i32 = 256;
pub const BUTTON_FIRE_PRIMARY: i32 = 1024;
pub const BUTTON_FIRE_SECONDARY: i32 = 2048;
pub const BUTTON_RELOAD: i32 = 8192;
pub const BUTTON_FIRE_THIRD: i32 = 134_217_728;

pub const CONTROL_FLAG_NONE: i32 = 0;
pub const CONTROL_FLAG_MOVEMENT: i32 = 1;
pub const CONTROL_FLAG_MOUSE: i32 = 2;
pub const CONTROL_FLAG_SPRINT_AND_DUCK: i32 = 4;
pub const CONTROL_FLAG_FIRE: i32 = 8;
pub const CONTROL_FLAG_RELOAD: i32 = 16;
pub const CONTROL_FLAG_CROSSHAIR: i32 = 32;

/// A PRNG used by the Rust server to determine ray sample positions.
struct IndexGenerator {
    state: u32,
}

impl IndexGenerator {
    fn new(seed: u32) -> Self {
        let mut generator = Self { state: seed };
        generator.next_state();
        generator
    }

    fn next_int(&mut self, max: u32) -> u32 {
        let mut t = ((u64::from(self.next_state()) * u64::from(max)) / 4_294_967_295) as i32;
        if t < 0 {
            #[allow(clippy::cast_possible_wrap)]
            let max_i32 = max as i32;
            t = max_i32 + t - 1;
        }
        #[allow(clippy::cast_sign_loss)]
        let res = t as u32;
        res
    }

    fn next_state(&mut self) -> u32 {
        let t = self.state as i32;
        let mut e = self.state as i32;
        e ^= e << 13;
        e ^= (e as u32 >> 17) as i32;
        e ^= e << 5;
        self.state = e as u32;
        if t >= 0 {
            #[allow(clippy::cast_sign_loss)]
            let res = t as u32;
            res
        } else {
            #[allow(clippy::cast_sign_loss)]
            let res = (4_294_967_295_i64 + i64::from(t) - 1) as u32;
            res
        }
    }
}

pub struct FrameBuilder {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>, // RGBA, width*height*4
    pub received: u32,   // samples filled so far
    pub sample_pos: Vec<u16>,
}

impl FrameBuilder {
    pub fn new(width: u32, height: u32) -> Self {
        let mut sample_pos = vec![0u16; (width * height * 2) as usize];
        let mut w = 0;
        for _h in 0..height {
            for g in 0..width {
                sample_pos[w] = g as u16;
                sample_pos[w + 1] = _h as u16;
                w += 2;
            }
        }

        let mut generator = IndexGenerator::new(1337);
        for r in (1..=(width * height - 1)).rev() {
            let c = (2 * r) as usize;
            let i = (2 * generator.next_int(r + 1)) as usize;

            let p = sample_pos[c];
            let k = sample_pos[c + 1];
            let a = sample_pos[i];
            let f = sample_pos[i + 1];

            sample_pos[i] = p;
            sample_pos[i + 1] = k;
            sample_pos[c] = a;
            sample_pos[c + 1] = f;
        }

        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
            received: 0,
            sample_pos,
        }
    }

    pub fn ingest(&mut self, msg: &AppCameraRays) {
        let ray_data = &msg.ray_data;
        let mut sample_offset = (2 * msg.sample_offset) as usize;

        let mut data_pointer = 0;
        let mut ray_lookback = [[0u8; 3]; 64];

        let mut t = 0u8;
        let mut r = 0u8;
        let mut i = 0u8;

        while data_pointer < ray_data.len().saturating_sub(1) {
            let n = ray_data[data_pointer];
            data_pointer += 1;

            if n == 255 {
                let l = ray_data[data_pointer];
                data_pointer += 1;
                let o = ray_data[data_pointer];
                data_pointer += 1;
                let s = ray_data[data_pointer];
                data_pointer += 1;

                t = (l << 2) | (o >> 6);
                r = 63 & o;
                i = s;

                let u = (3 * (t / 128) + 5 * (r / 16) + 7 * i) & 63;
                ray_lookback[u as usize] = [t, r, i];
            } else {
                let c = 192 & n;
                if c == 0 {
                    let h = (63 & n) as usize;
                    let y = ray_lookback[h];
                    t = y[0];
                    r = y[1];
                    i = y[2];
                } else if c == 64 {
                    let p = (63 & n) as usize;
                    let v = ray_lookback[p];
                    let g = ray_data[data_pointer];
                    data_pointer += 1;

                    t = v[0].wrapping_add(g >> 3).wrapping_sub(15);
                    r = v[1].wrapping_add(7 & g).wrapping_sub(3);
                    i = v[2];
                } else if c == 128 {
                    let r_idx = (63 & n) as usize;
                    let c_arr = ray_lookback[r_idx];
                    let next_byte = ray_data[data_pointer];
                    data_pointer += 1;

                    t = c_arr[0].wrapping_add(next_byte).wrapping_sub(127);
                    r = c_arr[1];
                    i = c_arr[2];
                } else {
                    let a = ray_data[data_pointer];
                    data_pointer += 1;
                    let f = ray_data[data_pointer];
                    data_pointer += 1;

                    t = (a << 2) | (f >> 6);
                    r = 63 & f;
                    i = 63 & n;

                    let d = (3 * (t / 128) + 5 * (r / 16) + 7 * i) & 63;
                    ray_lookback[d as usize] = [t, r, i];
                }
            }

            let distance = f64::from(t) / 1023.0;
            let alignment = f64::from(r) / 63.0;
            let material = i as usize;

            let colours = [
                [0.5, 0.5, 0.5],
                [0.8, 0.7, 0.7],
                [0.3, 0.7, 1.0],
                [0.6, 0.6, 0.6],
                [0.7, 0.7, 0.7],
                [0.8, 0.6, 0.4],
                [1.0, 0.4, 0.4],
                [1.0, 0.1, 0.1],
            ];

            sample_offset %= (2 * self.width * self.height) as usize;
            let idx1 = sample_offset;
            sample_offset += 1;
            let idx2 = sample_offset;
            sample_offset += 1;

            let x = self.sample_pos[idx1] as u32;
            let y = self.height - 1 - (self.sample_pos[idx2] as u32);
            let pixel_idx = ((y * self.width + x) * 4) as usize;

            let sky_color = [
                208.0 - (f64::from(y) / f64::from(self.height)) * 40.0, // Darken slightly towards top
                230.0 - (f64::from(y) / f64::from(self.height)) * 20.0,
                252.0,
            ];

            let target_colour =
                if (distance - 1.0).abs() < f64::EPSILON && alignment == 0.0 && material == 0 {
                    sky_color
                } else {
                    let default_colour = [1.0, 1.0, 1.0];
                    let colour = match colours.get(material) {
                        Some(c) => c,
                        None => &default_colour,
                    };

                    // Improved Lighting: 30% ambient light, 70% diffuse directional (alignment)
                    // This prevents shadowed sides of objects from being completely pitch black
                    let lighting = 0.3 + (0.7 * alignment);
                    let shaded = [
                        lighting * colour[0] * 255.0,
                        lighting * colour[1] * 255.0,
                        lighting * colour[2] * 255.0,
                    ];

                    // Depth Fog / Atmospheric Perspective
                    // Fade objects towards the sky color the further away they are
                    let fog_factor = distance.powf(1.5).clamp(0.0, 1.0);
                    [
                        shaded[0] * (1.0 - fog_factor) + sky_color[0] * fog_factor,
                        shaded[1] * (1.0 - fog_factor) + sky_color[1] * fog_factor,
                        shaded[2] * (1.0 - fog_factor) + sky_color[2] * fog_factor,
                    ]
                };

            if pixel_idx + 3 < self.pixels.len() {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    self.pixels[pixel_idx] = target_colour[0] as u8; // R
                    self.pixels[pixel_idx + 1] = target_colour[1] as u8; // G
                    self.pixels[pixel_idx + 2] = target_colour[2] as u8; // B
                    self.pixels[pixel_idx + 3] = 255; // A
                }
            }

            self.received += 1;
        }
    }

    pub fn is_complete(&self) -> bool {
        self.received >= self.width * self.height
    }

    pub fn encode_png(&self) -> Option<Vec<u8>> {
        let img =
            ImageBuffer::<Rgba<u8>, _>::from_raw(self.width, self.height, self.pixels.clone())?;

        // Upscale the image 3x so it's larger in the Discord embed
        let new_width = self.width * 3;
        let new_height = self.height * 3;
        let scaled_img = image::imageops::resize(
            &img,
            new_width,
            new_height,
            image::imageops::FilterType::Nearest,
        );

        let mut buf = std::io::Cursor::new(Vec::new());
        scaled_img
            .write_to(&mut buf, image::ImageFormat::Png)
            .ok()?;
        Some(buf.into_inner())
    }
}

/// A Camera instance that handles subscribing, resubscribing, decoding rays, and controlling a camera.
pub struct Camera {
    client: RustPlusClient,
    pub identifier: String,

    // Internal state
    state: Arc<Mutex<CameraState>>,

    // Background task handles
    listen_task: Option<JoinHandle<()>>,
    resubscribe_task: Option<JoinHandle<()>>,
    input_task: Option<JoinHandle<()>>,

    // Channel for emitting rendered PNG frames
    frame_tx: broadcast::Sender<Vec<u8>>,
}

struct CameraState {
    is_subscribed: bool,
    subscribe_info: Option<AppCameraInfo>,
    frame_builder: Option<FrameBuilder>,
}

impl Camera {
    #[must_use]
    pub fn new(client: RustPlusClient, identifier: impl Into<String>) -> Self {
        let (frame_tx, _) = broadcast::channel(10);

        Self {
            client,
            identifier: identifier.into(),
            state: Arc::new(Mutex::new(CameraState {
                is_subscribed: false,
                subscribe_info: None,
                frame_builder: None,
            })),
            listen_task: None,
            resubscribe_task: None,
            input_task: None,
            frame_tx,
        }
    }

    #[must_use]
    pub fn subscribe_frames(&self) -> broadcast::Receiver<Vec<u8>> {
        self.frame_tx.subscribe()
    }

    pub async fn subscribe(&mut self) -> Result<()> {
        self.inner_subscribe().await?;

        let mut state = self.state.lock().await;
        state.is_subscribed = true;
        drop(state);

        // Start listening task
        if self.listen_task.is_none() {
            if let Some(mut rx) = self.client.take_broadcast_receiver() {
                let state_clone = Arc::clone(&self.state);
                let tx_clone = self.frame_tx.clone();

                self.listen_task = Some(tokio::spawn(async move {
                    while let Ok(msg) = rx.recv().await {
                        if let Some(broadcast) = msg.broadcast {
                            if let Some(rays) = broadcast.camera_rays {
                                let mut s = state_clone.lock().await;
                                if !s.is_subscribed {
                                    continue;
                                }

                                if s.frame_builder.is_none() {
                                    if let Some(info) = &s.subscribe_info {
                                        s.frame_builder = Some(FrameBuilder::new(
                                            info.width as u32,
                                            info.height as u32,
                                        ));
                                    }
                                }

                                if let Some(fb) = &mut s.frame_builder {
                                    fb.ingest(&rays);
                                    if fb.is_complete() {
                                        // Take the completed frame builder and replace with a new one
                                        let completed_fb = s.frame_builder.take().unwrap();
                                        if let Some(info) = &s.subscribe_info {
                                            s.frame_builder = Some(FrameBuilder::new(
                                                info.width as u32,
                                                info.height as u32,
                                            ));
                                        }

                                        let tx_inner = tx_clone.clone();
                                        tokio::task::spawn_blocking(move || {
                                            if let Some(png_bytes) = completed_fb.encode_png() {
                                                let _ = tx_inner.send(png_bytes);
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                }));
            }
        }

        // Start resubscribe task
        if self.resubscribe_task.is_none() {
            let state_clone = Arc::clone(&self.state);
            let client_clone = self.client.clone();
            let identifier = self.identifier.clone();

            self.resubscribe_task = Some(tokio::spawn(async move {
                let mut ticker = interval(Duration::from_secs(10));
                loop {
                    ticker.tick().await;
                    let is_subscribed = state_clone.lock().await.is_subscribed;
                    if is_subscribed {
                        if let Err(e) = client_clone.subscribe_to_camera(&identifier).await {
                            warn!("Failed to resubscribe to camera: {e}");
                        }
                    }
                }
            }));
        }

        // Start input task to keep stream alive
        if self.input_task.is_none() {
            let state_clone = Arc::clone(&self.state);
            let client_clone = self.client.clone();

            self.input_task = Some(tokio::spawn(async move {
                let mut ticker = interval(Duration::from_millis(100));
                loop {
                    ticker.tick().await;
                    let is_subscribed = state_clone.lock().await.is_subscribed;
                    if is_subscribed {
                        let _ = client_clone.send_camera_input(BUTTON_NONE, 0.0, 0.0).await;
                    } else {
                        break;
                    }
                }
            }));
        }

        Ok(())
    }

    async fn inner_subscribe(&self) -> Result<()> {
        let response = self.client.subscribe_to_camera(&self.identifier).await?;
        if let Some(resp) = response.response {
            if let Some(info) = resp.camera_subscribe_info {
                let mut state = self.state.lock().await;
                state.subscribe_info = Some(info);
                state.frame_builder =
                    Some(FrameBuilder::new(info.width as u32, info.height as u32));
            }
        }
        Ok(())
    }

    pub async fn unsubscribe(&mut self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.is_subscribed = false;
        state.subscribe_info = None;
        state.frame_builder = None;
        drop(state);

        if let Some(task) = self.listen_task.take() {
            task.abort();
        }
        if let Some(task) = self.resubscribe_task.take() {
            task.abort();
        }
        if let Some(task) = self.input_task.take() {
            task.abort();
        }

        if self.client.is_connected() {
            let _ = self.client.unsubscribe_from_camera().await;
        }

        Ok(())
    }

    pub async fn move_camera(&self, buttons: i32, x: f32, y: f32) -> Result<()> {
        self.client
            .send_camera_input(buttons, x, y)
            .await
            .map(|_| ())
    }

    pub async fn zoom(&self) -> Result<()> {
        self.move_camera(BUTTON_FIRE_PRIMARY, 0.0, 0.0).await?;
        self.move_camera(BUTTON_NONE, 0.0, 0.0).await?;
        Ok(())
    }

    pub async fn shoot(&self) -> Result<()> {
        self.move_camera(BUTTON_FIRE_PRIMARY, 0.0, 0.0).await?;
        self.move_camera(BUTTON_NONE, 0.0, 0.0).await?;
        Ok(())
    }

    pub async fn reload(&self) -> Result<()> {
        self.move_camera(BUTTON_RELOAD, 0.0, 0.0).await?;
        self.move_camera(BUTTON_NONE, 0.0, 0.0).await?;
        Ok(())
    }

    pub async fn is_auto_turret(&self) -> bool {
        let state = self.state.lock().await;
        if let Some(info) = &state.subscribe_info {
            (info.control_flags & CONTROL_FLAG_CROSSHAIR) == CONTROL_FLAG_CROSSHAIR
        } else {
            false
        }
    }
}

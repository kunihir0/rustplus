#![allow(clippy::module_name_repetitions)]
#![allow(clippy::manual_unwrap_or_default, clippy::manual_unwrap_or)]
#![allow(clippy::multiple_crate_versions)]

pub mod camera;
pub mod client;
pub mod error;
pub mod events;
pub mod monitor;
pub mod monitors;
pub(crate) mod proxy;
pub(crate) mod ratelimit;

pub mod proto {
    #![allow(clippy::all, clippy::pedantic, clippy::nursery)]
    #![allow(non_snake_case)]
    #![allow(non_camel_case_types)]
    include!(concat!(env!("OUT_DIR"), "/rustplus.rs"));
}

pub use client::RustPlusClient;
pub use error::{Error, Result};

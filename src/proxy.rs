use crate::error::{Error, Result};
use std::sync::OnceLock;
use std::time::Instant;
use tokio::sync::Mutex;

const VERSION_URL: &str = "https://companion-rust.facepunch.com/api/version";

/// Cached proxy version value and the time it was fetched.
static CACHE: OnceLock<Mutex<(i64, Instant)>> = OnceLock::new();

/// How long to reuse a cached value before re-fetching.
const CACHE_TTL: std::time::Duration = std::time::Duration::from_mins(10);

/// Fallback value used when the API is unreachable.
const FALLBACK: i64 = 9_999_999_999_999;

/// Fetches the `minPublishedTime` from the Facepunch companion API
/// and returns `minPublishedTime + 1`.  The result is cached for 10 minutes.
///
/// # Errors
///
/// Returns `Error::Http` if the request fails *and* no cached value exists.
pub(crate) async fn get_proxy_version() -> Result<i64> {
    let cache = CACHE.get_or_init(|| Mutex::new((0, Instant::now().checked_sub(CACHE_TTL).unwrap_or_else(Instant::now))));
    let mut guard = cache.lock().await;

    if guard.0 != 0 && guard.1.elapsed() < CACHE_TTL {
        return Ok(guard.0);
    }

    match fetch_version().await {
        Ok(v) => {
            *guard = (v, Instant::now());
            Ok(v)
        }
        Err(e) => {
            if guard.0 != 0 {
                tracing::warn!("Failed to refresh proxy version, using cached value: {e}");
                return Ok(guard.0);
            }
            tracing::warn!("Failed to fetch proxy version, using fallback: {e}");
            Ok(FALLBACK)
        }
    }
}

async fn fetch_version() -> Result<i64> {
    let resp: serde_json::Value = reqwest::get(VERSION_URL)
        .await
        .map_err(Error::Http)?
        .json()
        .await
        .map_err(Error::Http)?;

    let ts = resp
        .get("minPublishedTime")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(FALLBACK - 1);

    Ok(ts + 1)
}

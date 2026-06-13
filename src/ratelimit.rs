use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RateLimiter {
    ip_tat: Instant,
    player_tat: Instant,
}

impl RateLimiter {
    #[must_use]
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            ip_tat: match now.checked_sub(Duration::from_secs_f64(50.0 / 15.0)) {
                Some(t) => t,
                None => now,
            },
            player_tat: match now.checked_sub(Duration::from_secs_f64(25.0 / 3.0)) {
                Some(t) => t,
                None => now,
            },
        }
    }

    /// Acquires the given number of tokens and returns the duration to wait before executing.
    pub fn acquire(&mut self, tokens: f64) -> Duration {
        let now = Instant::now();

        // IP Bucket (50 max, 15 per sec)
        let ip_burst = Duration::from_secs_f64(50.0 / 15.0);
        let min_ip_tat = match now.checked_sub(ip_burst) {
            Some(t) => t,
            None => now,
        };
        if self.ip_tat < min_ip_tat {
            self.ip_tat = min_ip_tat;
        }
        self.ip_tat += Duration::from_secs_f64((1.0 / 15.0) * tokens);

        // Player Bucket (25 max, 3 per sec)
        let player_burst = Duration::from_secs_f64(25.0 / 3.0);
        let min_player_tat = match now.checked_sub(player_burst) {
            Some(t) => t,
            None => now,
        };
        if self.player_tat < min_player_tat {
            self.player_tat = min_player_tat;
        }
        self.player_tat += Duration::from_secs_f64((1.0 / 3.0) * tokens);

        let ip_wait = self.ip_tat.saturating_duration_since(now);
        let player_wait = self.player_tat.saturating_duration_since(now);

        std::cmp::max(ip_wait, player_wait)
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

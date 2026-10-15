//! Effective speed limit: the global limit, lowered by the speed rules whose time window is open
//! (`Settings::speed_limit_at`). Re-evaluated every second, so the limit follows both the clock
//! and setting changes without restart.

use super::Engine;
use crate::{settings::Settings, util::human_bytes};

impl Engine {
    /// Applies the speed limit in effect at the current local time. Returns whether a speed rule
    /// is active.
    pub(super) fn refresh_speed_limit(&self, settings: &Settings) -> bool {
        let (limit, rule_active) = settings.speed_limit_at(chrono::Local::now().naive_local());
        if self.limiter.rate() != limit {
            self.limiter.set_rate(limit);
            match limit {
                Some(limit) => tracing::info!("speed limit: {}/s", human_bytes(limit)),
                None => tracing::info!("speed limit removed"),
            }
        }
        rule_active
    }
}

//! Disk-space guard: when the free space of the download root falls below `storage.min_free_mib`,
//! running transfers go back to the queue and nothing starts until the free space is back above
//! the threshold plus 10 %, so that downloads do not flap around the limit.

use std::sync::atomic::Ordering;

use super::{Engine, live::StopReason};
use crate::{
    events::{Notice, NoticeLevel},
    integrations::notify::Message,
    settings::{NotificationEvent, Settings},
    util::human_bytes,
};

impl Engine {
    /// Updates the low-disk state from the (cached) disk information. Returns whether downloads
    /// may run.
    pub(super) fn update_disk_guard(&self, settings: &Settings) -> bool {
        let min_free = settings.storage.min_free_mib.saturating_mul(1024 * 1024);
        let was_low = self.disk_low();
        let disk = self.disk_info();
        let low = disk.is_some_and(|disk| is_low(was_low, disk.free, min_free));
        if low == was_low {
            return !low;
        }
        self.disk_low.store(low, Ordering::Relaxed);
        let free = disk.map_or(0, |disk| disk.free);
        if low {
            let message = format!("Low disk space: {} free, downloads paused", human_bytes(free));
            tracing::warn!("{message} (minimum {})", human_bytes(min_free));
            self.live.stop_all(StopReason::Requeue);
            self.events.notice(Notice::new(NoticeLevel::Warning, "disk_low", &message).param("free", free.to_string()));
            let event = NotificationEvent::DiskLow;
            self.notifier.send(&settings.notifications.targets, event, Message::disk_low(&message));
        } else {
            tracing::info!("disk space available again ({} free): resuming downloads", human_bytes(free));
        }
        self.events.queue_changed();
        !low
    }

    /// Whether downloads are held back because the disk is almost full.
    pub fn disk_low(&self) -> bool {
        self.disk_low.load(Ordering::Relaxed)
    }
}

/// Low below `min_free`; once low, back to normal only above `min_free` + 10 %. `0` disables.
fn is_low(was_low: bool, free: u64, min_free: u64) -> bool {
    if min_free == 0 {
        return false;
    }
    let threshold = if was_low { min_free.saturating_add(min_free / 10) } else { min_free };
    free < threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn applies_hysteresis() {
        assert!(!is_low(false, 2 * GIB, GIB));
        assert!(is_low(false, GIB - 1, GIB));
        // Recovering: still low until 10 % above the threshold.
        assert!(is_low(true, GIB + GIB / 20, GIB));
        assert!(!is_low(true, GIB + GIB / 10, GIB));
        // Not low yet: the plain threshold applies.
        assert!(!is_low(false, GIB + GIB / 20, GIB));
    }

    #[test]
    fn zero_disables_the_guard() {
        assert!(!is_low(false, 0, 0));
        assert!(!is_low(true, 0, 0));
    }
}

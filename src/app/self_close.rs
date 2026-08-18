//! Debounce for the sidebar's "I have been left alone in my window"
//! self-close.
//!
//! A single observation is not enough to act on. `after-kill-pane` wakes
//! the sidebar via SIGUSR1 within milliseconds of a pane disappearing, so
//! the ordinary "kill a pane, then create a replacement" sequence — two
//! separate `tmux` invocations, tens of milliseconds apart — would be
//! observed mid-flight as "nothing but us" and cost the user their whole
//! window. Requiring the condition to hold continuously for a short grace
//! period lets any such respawn land first, while still reacting to a
//! genuine `prefix + x` far sooner than the 1s refresh tick would.

use std::time::{Duration, Instant};

/// How long the sidebar must observe itself alone before it tears itself
/// down. Comfortably longer than the gap between two back-to-back `tmux`
/// process spawns, and short enough that `prefix + x` still feels instant.
pub(super) const SELF_CLOSE_GRACE: Duration = Duration::from_millis(250);

/// Tracks how long [`crate::state::RefreshOutcome::self_close_eligible`]
/// has been continuously true.
#[derive(Debug, Default)]
pub(super) struct SelfCloseDebounce {
    alone_since: Option<Instant>,
}

impl SelfCloseDebounce {
    /// Feed one observation, returning `true` only once the sidebar has
    /// looked alone for [`SELF_CLOSE_GRACE`] without interruption. Any
    /// observation to the contrary resets the timer, so a window that
    /// gets a new pane during the grace period is never torn down.
    pub(super) fn observe(&mut self, eligible: bool, now: Instant) -> bool {
        if !eligible {
            self.alone_since = None;
            return false;
        }
        match self.alone_since {
            None => {
                self.alone_since = Some(now);
                false
            }
            Some(since) => now.duration_since(since) >= SELF_CLOSE_GRACE,
        }
    }

    /// Whether a close is awaiting confirmation. The event loop polls
    /// tmux again after [`SELF_CLOSE_GRACE`] instead of waiting out the
    /// full refresh interval, so confirmation costs one extra query and
    /// only in the rare tick where the sidebar looks alone.
    pub(super) fn is_pending(&self) -> bool {
        self.alone_since.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_observation_never_closes() {
        let mut debounce = SelfCloseDebounce::default();
        let t0 = Instant::now();
        assert!(!debounce.observe(true, t0));
        assert!(debounce.is_pending());
    }

    #[test]
    fn closes_once_the_grace_period_elapses() {
        let mut debounce = SelfCloseDebounce::default();
        let t0 = Instant::now();
        assert!(!debounce.observe(true, t0));
        assert!(!debounce.observe(true, t0 + SELF_CLOSE_GRACE - Duration::from_millis(1)));
        assert!(debounce.observe(true, t0 + SELF_CLOSE_GRACE));
    }

    #[test]
    fn a_replacement_pane_during_the_grace_period_cancels_the_close() {
        // `tmux kill-pane` followed ~50ms later by `tmux split-window`:
        // the sidebar sees itself alone, then sees the new neighbour.
        let mut debounce = SelfCloseDebounce::default();
        let t0 = Instant::now();
        assert!(!debounce.observe(true, t0));
        assert!(!debounce.observe(false, t0 + Duration::from_millis(50)));
        assert!(!debounce.is_pending());
        // ...and the old timer must not carry over into a later spell.
        assert!(!debounce.observe(true, t0 + Duration::from_millis(60)));
        assert!(!debounce.observe(true, t0 + Duration::from_millis(100)));
    }

    #[test]
    fn repeated_sigusr1_wakeups_inside_the_grace_period_do_not_close_early() {
        let mut debounce = SelfCloseDebounce::default();
        let t0 = Instant::now();
        assert!(!debounce.observe(true, t0));
        for ms in [1, 5, 20, 100, 249] {
            assert!(
                !debounce.observe(true, t0 + Duration::from_millis(ms)),
                "closed early at {ms}ms"
            );
        }
        assert!(debounce.observe(true, t0 + Duration::from_millis(250)));
    }

    #[test]
    fn stays_idle_while_a_neighbour_remains() {
        let mut debounce = SelfCloseDebounce::default();
        let t0 = Instant::now();
        for ms in [0, 500, 5_000] {
            assert!(!debounce.observe(false, t0 + Duration::from_millis(ms)));
            assert!(!debounce.is_pending());
        }
    }
}

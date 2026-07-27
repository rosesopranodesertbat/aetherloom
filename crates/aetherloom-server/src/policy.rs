//! Fixed online lifecycle policy expressed in authoritative ticks.

use crate::scheduler::{TickId, AUTHORITATIVE_HZ};
use std::time::Duration;

pub const CLEAR_INPUT_ON_DISCONNECT: bool = true;
pub const BOT_TAKEOVER_AFTER: Duration = Duration::from_secs(3);
pub const RECONNECT_GRACE_PERIOD: Duration = Duration::from_secs(90);
pub const BOT_TAKEOVER_AFTER_TICKS: u64 = 3 * AUTHORITATIVE_HZ as u64;
pub const RECONNECT_GRACE_TICKS: u64 = 90 * AUTHORITATIVE_HZ as u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisconnectAction {
    AwaitReconnect,
    BotControl,
    Defeat,
}

/// Determines server-owned control from an absolute disconnect tick.
///
/// Inputs are cleared by the caller immediately when the disconnect is
/// observed. At 3 seconds the bot takes over; at 90 seconds the reconnect
/// window closes and the expedition records a defeat.
pub fn disconnect_action(disconnected_at: TickId, now: TickId) -> DisconnectAction {
    let elapsed = now.0.saturating_sub(disconnected_at.0);
    if elapsed >= RECONNECT_GRACE_TICKS {
        DisconnectAction::Defeat
    } else if elapsed >= BOT_TAKEOVER_AFTER_TICKS {
        DisconnectAction::BotControl
    } else {
        DisconnectAction::AwaitReconnect
    }
}

pub fn can_reconnect(disconnected_at: TickId, now: TickId) -> bool {
    now.0.saturating_sub(disconnected_at.0) < RECONNECT_GRACE_TICKS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnect_policy_has_exact_boundary_behavior() {
        let disconnected = TickId(100);

        assert_eq!(
            disconnect_action(
                disconnected,
                TickId(disconnected.0 + BOT_TAKEOVER_AFTER_TICKS - 1)
            ),
            DisconnectAction::AwaitReconnect
        );
        assert_eq!(
            disconnect_action(
                disconnected,
                TickId(disconnected.0 + BOT_TAKEOVER_AFTER_TICKS)
            ),
            DisconnectAction::BotControl
        );
        assert!(can_reconnect(
            disconnected,
            TickId(disconnected.0 + RECONNECT_GRACE_TICKS - 1)
        ));
        assert_eq!(
            disconnect_action(
                disconnected,
                TickId(disconnected.0 + RECONNECT_GRACE_TICKS)
            ),
            DisconnectAction::Defeat
        );
        assert!(!can_reconnect(
            disconnected,
            TickId(disconnected.0 + RECONNECT_GRACE_TICKS)
        ));
    }

    #[test]
    fn clock_regression_does_not_underflow_policy() {
        assert_eq!(
            disconnect_action(TickId(100), TickId(99)),
            DisconnectAction::AwaitReconnect
        );
    }
}

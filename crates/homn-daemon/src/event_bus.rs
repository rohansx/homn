//! Broadcast event bus for the daemon.
//!
//! Thin wrapper over `tokio::sync::broadcast`. Subscribers see events emitted after their
//! subscription; lagging subscribers receive a `Lagged` error and can choose to skip or
//! reconnect.

use homn_types::BusEvent;
use tokio::sync::broadcast;

/// Broadcast bus for `BusEvent`s. Cloning is cheap (Arc-backed).
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<BusEvent>,
}

impl EventBus {
    /// Create a new bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an event. Returns `Ok(n)` where `n` is the number of active subscribers that
    /// received it, or `Err` if no subscribers are connected (a soft failure — common at boot).
    pub fn publish(&self, event: BusEvent) -> Result<usize, broadcast::error::SendError<BusEvent>> {
        self.sender.send(event)
    }

    /// Subscribe to future events.
    pub fn subscribe(&self) -> Subscriber {
        Subscriber {
            inner: self.sender.subscribe(),
        }
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// A handle that yields `BusEvent`s as they're published.
pub struct Subscriber {
    inner: broadcast::Receiver<BusEvent>,
}

impl Subscriber {
    /// Await the next event. Returns `None` when the bus is closed (i.e., all senders dropped).
    pub async fn recv(&mut self) -> Option<BusEvent> {
        loop {
            match self.inner.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "event bus subscriber lagged");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use homn_types::Decision;

    #[tokio::test]
    async fn publish_with_no_subscribers_is_a_soft_failure() {
        let bus = EventBus::new(16);
        let ev = BusEvent::DecisionMade {
            decision_id: 1,
            tool: "Bash".into(),
            decision: Decision::Allow,
            rule: None,
        };
        assert!(bus.publish(ev).is_err());
    }

    #[tokio::test]
    async fn subscriber_receives_subsequently_published_events() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe();
        let ev = BusEvent::DecisionMade {
            decision_id: 2,
            tool: "Read".into(),
            decision: Decision::Deny,
            rule: None,
        };
        bus.publish(ev.clone()).unwrap();
        let got = sub.recv().await.unwrap();
        assert_eq!(got, ev);
    }

    #[tokio::test]
    async fn multiple_subscribers_each_receive_events() {
        let bus = EventBus::new(16);
        let mut s1 = bus.subscribe();
        let mut s2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
        let ev = BusEvent::SessionStarted {
            session_id: homn_types::SessionId::new("01HXY"),
            cwd: std::path::PathBuf::from("/tmp"),
        };
        bus.publish(ev.clone()).unwrap();
        assert_eq!(s1.recv().await.unwrap(), ev);
        assert_eq!(s2.recv().await.unwrap(), ev);
    }
}

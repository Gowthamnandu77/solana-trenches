use crate::listener::LogEvent;
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

pub enum PushResult {
    Accepted { evicted: Vec<LogEvent> },
    Full,
    Closed,
}
struct State {
    items: VecDeque<LogEvent>,
    closed: bool,
}

/// Bounded admission removes expired work before rejecting a fresh notification.
/// Newest work is drained first so fresh candidates do not wait behind a burst.
pub struct FreshQueue {
    capacity: usize,
    max_age: u64,
    state: Mutex<State>,
    notify: Notify,
}

impl FreshQueue {
    pub fn new(capacity: usize, max_age: u64) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            max_age,
            state: Mutex::new(State {
                items: VecDeque::with_capacity(capacity),
                closed: false,
            }),
            notify: Notify::new(),
        }
    }
    pub fn push(&self, event: LogEvent) -> PushResult {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return PushResult::Closed;
        }
        let now = Instant::now();
        let mut evicted = Vec::new();
        state.items.retain(|queued| {
            let stale = now
                .saturating_duration_since(queued.received_at)
                .as_millis()
                > self.max_age as u128;
            if stale {
                evicted.push(queued.clone());
            }
            !stale
        });
        if state.items.len() >= self.capacity {
            return PushResult::Full;
        }
        state.items.push_back(event);
        drop(state);
        self.notify.notify_one();
        PushResult::Accepted { evicted }
    }
    pub async fn recv(&self) -> Option<LogEvent> {
        loop {
            let notified = self.notify.notified();
            if let Some(event) = self.try_recv() {
                return Some(event);
            }
            if self.is_closed() {
                return None;
            }
            notified.await;
        }
    }
    pub fn try_recv(&self) -> Option<LogEvent> {
        self.state.lock().unwrap().items.pop_back()
    }
    pub fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.notify.notify_waiters();
    }
    fn is_closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }
    pub fn len(&self) -> usize {
        self.state.lock().unwrap().items.len()
    }
}

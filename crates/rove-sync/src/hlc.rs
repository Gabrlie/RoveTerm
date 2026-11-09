//! Hybrid logical clock: wall time in milliseconds, a counter for events within the same
//! millisecond (or while the wall clock is behind), and the device id as the final tie-breaker.
//! Timestamps from different devices are totally ordered and respect causality: a device that has
//! seen a timestamp only issues later ones.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// One timestamp. Ordered by `wall`, then `counter`, then `device`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hlc {
    /// Unix milliseconds.
    pub wall: u64,
    /// Events at the same `wall`.
    pub counter: u32,
    /// Device that issued it.
    pub device: String,
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.wall, self.counter, &self.device).cmp(&(other.wall, other.counter, &other.device))
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The clock of one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    /// Last timestamp issued or observed.
    pub last: Hlc,
}

impl Clock {
    pub fn new(device: &str) -> Self {
        Self {
            last: Hlc {
                wall: 0,
                counter: 0,
                device: device.to_owned(),
            },
        }
    }

    /// Issues a timestamp later than every one issued or observed so far. `now` is the wall clock
    /// in Unix milliseconds.
    pub fn tick(&mut self, now: u64) -> Hlc {
        if now > self.last.wall {
            self.last.wall = now;
            self.last.counter = 0;
        } else {
            self.last.counter = self.last.counter.saturating_add(1);
        }
        self.last.clone()
    }

    /// Takes a remote timestamp into account so later ticks come after it.
    pub fn observe(&mut self, remote: &Hlc) {
        if (remote.wall, remote.counter) > (self.last.wall, self.last.counter) {
            self.last.wall = remote.wall;
            self.last.counter = remote.counter;
        }
    }
}

/// Current Unix time in milliseconds.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_increase_even_when_the_wall_clock_stalls_or_goes_back() {
        let mut clock = Clock::new("a");
        let first = clock.tick(1000);
        let same_ms = clock.tick(1000);
        let earlier = clock.tick(900);
        assert!(first < same_ms && same_ms < earlier);
        assert_eq!(earlier.wall, 1000);
    }

    #[test]
    fn observing_a_later_remote_moves_the_clock_past_it() {
        let mut clock = Clock::new("a");
        clock.tick(1000);
        let remote = Hlc {
            wall: 5000,
            counter: 3,
            device: "b".into(),
        };
        clock.observe(&remote);
        let next = clock.tick(1200);
        assert!(next > remote);
        // Equal wall and counter: the device id decides.
        let tie_a = Hlc {
            wall: 1,
            counter: 0,
            device: "a".into(),
        };
        let tie_b = Hlc {
            wall: 1,
            counter: 0,
            device: "b".into(),
        };
        assert!(tie_a < tie_b);
    }
}

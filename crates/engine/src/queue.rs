//! Queues: groups of downloads with their own concurrency and an optional weekly time window.

use serde::{Deserialize, Serialize};

pub type QueueId = u32;

/// Always exists; its concurrency is the "downloads at the same time" setting.
pub const MAIN_QUEUE: QueueId = 0;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub id: QueueId,
    pub name: String,
    /// Downloads from this queue running at the same time.
    pub max_active: usize,
    /// Stopped by the user: nothing in it starts until it is started again.
    #[serde(default)]
    pub stopped: bool,
    /// `None` runs any time.
    #[serde(default)]
    pub schedule: Option<Schedule>,
}

/// A weekly time window, in local time.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    /// Minutes after midnight. `start == stop` means all day.
    pub start: u16,
    pub stop: u16,
    /// Bit 0 is Monday, bit 6 is Sunday.
    pub days: u8,
}

impl Schedule {
    pub const EVERY_DAY: u8 = 0x7f;

    /// Whether downloads may run on `weekday` (0 is Monday) at `minute` after midnight.
    /// A window that crosses midnight (23:00 to 06:00) belongs to the day it starts on.
    pub fn is_open(&self, weekday: u8, minute: u16) -> bool {
        let day = |d: u8| self.days & (1 << (d % 7)) != 0;
        let (start, stop) = (self.start.min(1439), self.stop.min(1439));
        if start == stop {
            day(weekday)
        } else if start < stop {
            day(weekday) && (start..stop).contains(&minute)
        } else {
            (minute >= start && day(weekday)) || (minute < stop && day(weekday + 6))
        }
    }
}

/// Local weekday (0 is Monday) and minute of the day.
pub(crate) fn local_now() -> (u8, u16) {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    (
        now.weekday().num_days_from_monday() as u8,
        (now.hour() * 60 + now.minute()) as u16,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: u8 = 0;
    const SUN: u8 = 6;

    #[test]
    fn day_window() {
        let s = Schedule {
            start: 9 * 60,
            stop: 17 * 60,
            days: 0b001_1111, // weekdays
        };
        assert!(s.is_open(MON, 9 * 60));
        assert!(!s.is_open(MON, 17 * 60));
        assert!(!s.is_open(MON, 8 * 60 + 59));
        assert!(!s.is_open(SUN, 12 * 60));
    }

    #[test]
    fn night_window_belongs_to_the_day_it_starts() {
        let s = Schedule {
            start: 23 * 60,
            stop: 6 * 60,
            days: 1 << SUN,
        };
        assert!(s.is_open(SUN, 23 * 60 + 30));
        // Monday early morning is the tail of Sunday night.
        assert!(s.is_open(MON, 5 * 60));
        assert!(!s.is_open(MON, 6 * 60));
        assert!(!s.is_open(MON, 23 * 60 + 30));
        assert!(!s.is_open(SUN, 5 * 60));
    }

    #[test]
    fn same_start_and_stop_is_all_day() {
        let s = Schedule {
            start: 0,
            stop: 0,
            days: 1 << 2,
        };
        assert!(s.is_open(2, 0) && s.is_open(2, 1439));
        assert!(!s.is_open(3, 600));
    }
}

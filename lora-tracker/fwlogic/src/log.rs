//! A small ring of recent events, shown on the OLED and echoed to serial.

use core::fmt::{self, Write};

use heapless::{Deque, String};

/// Longest event text kept.
pub const TEXT: usize = 32;

/// One event, stamped with board uptime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Increases by one per event, so a reader can ask for what's new.
    pub id: u32,
    pub at_s: u32,
    pub text: String<TEXT>,
}

/// The newest `N` events; older ones fall off.
#[derive(Debug, Default)]
pub struct EventLog<const N: usize> {
    entries: Deque<Entry, N>,
    next_id: u32,
}

impl<const N: usize> EventLog<N> {
    /// An empty log.
    pub const fn new() -> Self {
        Self { entries: Deque::new(), next_id: 0 }
    }

    /// Events newer than `id` (all of them for `None`), oldest first.
    pub fn after(&self, id: Option<u32>) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| id.is_none_or(|i| e.id > i))
    }

    /// Records an event, truncating text that doesn't fit, and returns it.
    pub fn push(&mut self, at_s: u32, args: fmt::Arguments<'_>) -> &Entry {
        let mut text = String::new();
        let _ = Truncate(&mut text).write_fmt(args);
        if self.entries.is_full() {
            self.entries.pop_front();
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let _ = self.entries.push_back(Entry { id, at_s, text });
        self.entries.back().unwrap()
    }

    /// Newest first.
    pub fn newest(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().rev()
    }
}

/// A writer that keeps whatever fits in the string and drops the rest.
pub struct Truncate<'a, const N: usize>(pub &'a mut String<N>);

impl<const N: usize> Write for Truncate<'_, N> {
    /// Pushes chars until the string is full, then silently stops.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if self.0.push(c).is_err() {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_newest_and_truncates() {
        let mut log: EventLog<3> = EventLog::new();
        for i in 0..5 {
            log.push(i, format_args!("event {i}"));
        }
        let texts: Vec<_> = log.newest().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["event 4", "event 3", "event 2"]);
        let e = log.push(9, format_args!("{}", "x".repeat(50)));
        assert_eq!(e.text.len(), TEXT);
        let after: Vec<_> = log.after(Some(3)).map(|e| e.id).collect();
        assert_eq!(after, [4, 5]);
        assert_eq!(log.after(None).count(), 3);
    }
}

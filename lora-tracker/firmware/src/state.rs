//! State shared between tasks: the status the OLED shows, the event log, and
//! the outbox of sealed packets waiting for an HTTP path.

use core::cell::RefCell;
use core::fmt;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use fwlogic::log::EventLog;
use fwlogic::screens::Status;
use heapless::{Deque, Vec};
use tracker_proto::packet::MAX_PACKET;

/// A sealed report, ready to send.
pub type Packet = Vec<u8, MAX_PACKET>;
/// Events kept for the OLED events page.
pub const LOG_LEN: usize = 24;
/// Reports kept while no HTTP path is up; the oldest is dropped when full.
pub const OUTBOX_LEN: usize = 32;

pub struct Shared {
    pub status: Status,
    pub log: EventLog<LOG_LEN>,
    pub outbox: Deque<Packet, OUTBOX_LEN>,
}

static SHARED: Mutex<CriticalSectionRawMutex, RefCell<Option<Shared>>> = Mutex::new(RefCell::new(None));

/// Installs the initial status. Call once, before spawning tasks.
pub fn init(status: Status) {
    SHARED.lock(|s| {
        *s.borrow_mut() = Some(Shared { status, log: EventLog::new(), outbox: Deque::new() });
    });
}

/// Runs `f` with the shared state locked. Don't call [`event`] from inside `f`.
pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    SHARED.lock(|s| {
        let mut s = s.borrow_mut();
        let s = s.as_mut().expect("state::init not called");
        s.status.uptime_s = uptime();
        f(s)
    })
}

/// Seconds since boot.
pub fn uptime() -> u32 {
    embassy_time::Instant::now().as_secs() as u32
}

/// Prints an event to serial in full and keeps a truncated copy for the OLED.
pub fn event(args: fmt::Arguments<'_>) {
    let now = uptime();
    esp_println::println!("[{now:>6}] {args}");
    with(|s| {
        s.log.push(now, args);
    });
}

/// `event!("wifi up {}", ssid)`: see [`event`].
#[macro_export]
macro_rules! event {
    ($($t:tt)*) => { $crate::state::event(format_args!($($t)*)) };
}

/// Queues a packet for HTTP upload, dropping the oldest if the outbox is full.
pub fn enqueue(pkt: Packet) {
    let dropped = with(|s| {
        let full = s.outbox.is_full();
        if full {
            s.outbox.pop_front();
        }
        let _ = s.outbox.push_back(pkt);
        s.status.queued = s.outbox.len() as u16;
        full
    });
    if dropped {
        event!("outbox full, dropped oldest");
    }
}

//  Wall-clock, monotonic clock and sleep for hosted std.
//  POSIX and Windows Rust targets share the same implementation.

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static MONO_START: OnceLock<Instant> = OnceLock::new();

fn mono_start() -> &'static Instant {
    MONO_START.get_or_init(Instant::now)
}

#[no_mangle]
pub extern "C" fn mylis_time_now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn mylis_time_now_nanos() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn mylis_time_monotonic_millis() -> i64 {
    mono_start().elapsed().as_millis() as i64
}

#[no_mangle]
pub extern "C" fn mylis_time_now_seconds() -> f64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(_) => 0.0,
    }
}

#[no_mangle]
pub extern "C" fn mylis_time_sleep_millis(milliseconds: i64) {
    if milliseconds > 0 {
        std::thread::sleep(Duration::from_millis(milliseconds as u64));
    }
}

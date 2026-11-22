//  Wall-clock, monotonic clock and sleep for hosted std.
//  POSIX and Windows Rust targets share the same implementation.

use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use std::os::raw::{c_char, c_int, c_long};

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

// 当前本地时区相对 UTC 的偏移（分钟）；夏令时按操作系统当前状态处理。
#[cfg(unix)]
#[repr(C)]
struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

#[cfg(unix)]
extern "C" {
    fn localtime_r(timep: *const c_long, result: *mut Tm) -> *mut Tm;
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_time_local_offset_minutes() -> i64 {
    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as c_long,
        Err(_) => 0,
    };
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    let result = unsafe { localtime_r(&now, &mut tm) };
    if result.is_null() {
        return 0;
    }
    (tm.tm_gmtoff / 60) as i64
}

#[cfg(windows)]
#[repr(C)]
struct WinSystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[cfg(windows)]
#[repr(C)]
struct WinTimeZoneInformation {
    bias: i32,
    standard_name: [u16; 32],
    standard_date: WinSystemTime,
    standard_bias: i32,
    daylight_name: [u16; 32],
    daylight_date: WinSystemTime,
    daylight_bias: i32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetTimeZoneInformation(info: *mut WinTimeZoneInformation) -> u32;
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_time_local_offset_minutes() -> i64 {
    const TIME_ZONE_ID_INVALID: u32 = 0xffff_ffff;
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    let mut info: WinTimeZoneInformation = unsafe { std::mem::zeroed() };
    let state = unsafe { GetTimeZoneInformation(&mut info) };
    if state == TIME_ZONE_ID_INVALID {
        return 0;
    }
    let mut bias = info.bias;
    if state == TIME_ZONE_ID_DAYLIGHT {
        bias = bias + info.daylight_bias;
    }
    -(bias as i64)
}

#[cfg(not(any(unix, windows)))]
#[no_mangle]
pub extern "C" fn mylis_time_local_offset_minutes() -> i64 { 0 }

#[no_mangle]
pub extern "C" fn mylis_time_sleep_millis(milliseconds: i64) {
    if milliseconds > 0 {
        std::thread::sleep(Duration::from_millis(milliseconds as u64));
    }
}

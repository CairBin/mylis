use crate::abi::MyStr;
use crate::gc::Gc;
use crate::plat;
use core::cmp::min;

pub trait Runtime: Sync {
    fn gc(&self) -> &dyn Gc;

    fn alloc(&self, size: usize) -> *mut u8 { self.gc().alloc(size) }

    fn str_from_buf(&self, s: *const u8, len: usize) -> *mut MyStr {
        unsafe {
            let total = core::mem::size_of::<MyStr>() + len + 1;
            let p = self.alloc(total) as *mut u8;
            let m = p as *mut MyStr;
            let data = p.add(core::mem::size_of::<MyStr>());
            if len > 0 && !s.is_null() { core::ptr::copy_nonoverlapping(s, data, len); }
            *data.add(len) = 0;
            (*m).len = len as u64;
            (*m).data = data;
            m
        }
    }

    fn str_lit(&self, s: *const u8, len: usize) -> *mut MyStr { self.str_from_buf(s, len) }

    fn str_concat(&self, a: *mut MyStr, b: *mut MyStr) -> *mut MyStr {
        unsafe {
            let alen = if a.is_null() { 0 } else { (*a).len as usize };
            let blen = if b.is_null() { 0 } else { (*b).len as usize };
            let total = core::mem::size_of::<MyStr>() + alen + blen + 1;
            let p = self.alloc(total) as *mut u8;
            let m = p as *mut MyStr;
            let data = p.add(core::mem::size_of::<MyStr>());
            if alen > 0 { core::ptr::copy_nonoverlapping((*a).data, data, alen); }
            if blen > 0 { core::ptr::copy_nonoverlapping((*b).data, data.add(alen), blen); }
            *data.add(alen + blen) = 0;
            (*m).len = (alen + blen) as u64;
            (*m).data = data;
            m
        }
    }

    fn str_eq(&self, a: *mut MyStr, b: *mut MyStr) -> bool {
        unsafe {
            if a == b { return true; }
            if a.is_null() || b.is_null() { return false; }
            if (*a).len != (*b).len { return false; }
            core::slice::from_raw_parts((*a).data, (*a).len as usize)
                == core::slice::from_raw_parts((*b).data, (*b).len as usize)
        }
    }

    fn str_cmp(&self, a: *mut MyStr, b: *mut MyStr) -> i32 {
        unsafe {
            if a == b { return 0; }
            if a.is_null() { return -1; }
            if b.is_null() { return 1; }
            let n = min((*a).len, (*b).len) as usize;
            let x = core::slice::from_raw_parts((*a).data, n);
            let y = core::slice::from_raw_parts((*b).data, n);
            match x.cmp(y) {
                core::cmp::Ordering::Less => -1,
                core::cmp::Ordering::Greater => 1,
                core::cmp::Ordering::Equal => {
                    if (*a).len == (*b).len { 0 } else if (*a).len < (*b).len { -1 } else { 1 }
                }
            }
        }
    }

    fn print(&self, s: *mut MyStr) {
        if s.is_null() { return; }
        unsafe {
            let bytes = core::slice::from_raw_parts((*s).data, (*s).len as usize);
            plat::platform().write_stdout(bytes);
        }
    }

    fn println(&self, s: *mut MyStr) {
        self.print(s);
        plat::platform().write_stdout(b"\n");
    }

    fn panic(&self, msg: *mut MyStr) -> ! {
        let text = if msg.is_null() { "" } else {
            unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts((*msg).data, (*msg).len as usize)) }
        };
        self.panic_text(text)
    }

    fn panic_text(&self, text: &str) -> ! {
        plat::platform().write_stderr(b"panic: ");
        plat::platform().write_stderr(text.as_bytes());
        plat::platform().write_stderr(b"\n");
        plat::platform().abort()
    }

    fn arr_alloc(&self, elem_size: usize, len: usize) -> *mut u8 {
        let total = core::mem::size_of::<u64>() + elem_size.saturating_mul(len);
        let p = self.alloc(total);
        unsafe {
            *(p as *mut u64) = len as u64;
            // `new T[n]` 的元素按语言规范必须全部零初始化，不能依赖分配器行为。
            let payload = elem_size.saturating_mul(len);
            core::ptr::write_bytes(p.add(core::mem::size_of::<u64>()), 0, payload);
        }
        p
    }

    fn arr_index(&self, len: usize, idx: i64) -> usize {
        if idx < 0 || (idx as usize) >= len {
            let mut text = Text::new();
            text.push_str("array index ");
            text.push_i64(idx);
            text.push_str(" out of bounds (len ");
            text.push_usize(len);
            text.push_str(")");
            self.panic_text(text.as_str());
        }
        idx as usize
    }

    fn i64_to_string(&self, v: i64) -> *mut MyStr {
        let mut text = Text::new();
        text.push_i64(v);
        let s = text.as_str();
        self.str_from_buf(s.as_ptr(), s.len())
    }

    fn u64_to_string(&self, v: u64) -> *mut MyStr {
        let mut text = Text::new();
        text.push_u64(v);
        let s = text.as_str();
        self.str_from_buf(s.as_ptr(), s.len())
    }

    fn f64_to_string(&self, v: f64) -> *mut MyStr {
        let (buf, n) = plat::platform().format_f64(v);
        self.str_from_buf(buf.as_ptr(), n)
    }

    fn f64_to_string_prec(&self, v: f64, precision: i32) -> *mut MyStr {
        let (buf, n) = plat::platform().format_f64_prec(v, precision);
        self.str_from_buf(buf.as_ptr(), n)
    } 
    fn f64_to_string_fmt(&self, v: f64, width: i32, precision: i32, flags: i32) -> *mut MyStr {
        let (buf, n) = plat::platform().format_f64_prec(v, precision);
        let mut out = [0u8; 128];
        let w = width.clamp(0, 100) as usize;
        let body_start = if n > 0 && buf[0] == b'-' { 1 } else { 0 };
        let body = &buf[body_start..n];
        let sign = if body_start == 1 {
            b'-'
        } else if flags & 2 != 0 {
            b'+'
        } else if flags & 8 != 0 {
            b' '
        } else {
            0
        };
        let sign_len = if sign != 0 { 1 } else { 0 };
        let used = body.len() + sign_len;
        let pad = w.saturating_sub(used);
        let mut o = 0usize;
        if flags & 1 != 0 {
            if sign != 0 { out[o] = sign; o += 1; }
            out[o..o + body.len()].copy_from_slice(body); o += body.len();
            for _ in 0..pad { out[o] = b' '; o += 1; }
        } else if flags & 4 != 0 {
            if sign != 0 { out[o] = sign; o += 1; }
            for _ in 0..pad { out[o] = b'0'; o += 1; }
            out[o..o + body.len()].copy_from_slice(body); o += body.len();
        } else {
            for _ in 0..pad { out[o] = b' '; o += 1; }
            if sign != 0 { out[o] = sign; o += 1; }
            out[o..o + body.len()].copy_from_slice(body); o += body.len();
        }
        self.str_from_buf(out.as_ptr(), o)
    }

    fn str_starts_with(&self, s: *mut MyStr, prefix: *mut MyStr) -> bool {
        unsafe {
            if s.is_null() || prefix.is_null() { return false; }
            if (*prefix).len > (*s).len { return false; }
            let n = (*prefix).len as usize;
            core::slice::from_raw_parts((*s).data, n) == core::slice::from_raw_parts((*prefix).data, n)
        }
    }

    fn str_ends_with(&self, s: *mut MyStr, suffix: *mut MyStr) -> bool {
        unsafe {
            if s.is_null() || suffix.is_null() { return false; }
            let sl = (*s).len as usize;
            let fl = (*suffix).len as usize;
            if fl > sl { return false; }
            core::slice::from_raw_parts((*s).data.add(sl - fl), fl)
                == core::slice::from_raw_parts((*suffix).data, fl)
        }
    }

    fn str_index_of(&self, s: *mut MyStr, needle: *mut MyStr) -> i64 {
        unsafe {
            if s.is_null() || needle.is_null() { return -1; }
            let sl = (*s).len as usize;
            let nl = (*needle).len as usize;
            if nl == 0 { return 0; }
            if nl > sl { return -1; }
            let hay = core::slice::from_raw_parts((*s).data, sl);
            let pat = core::slice::from_raw_parts((*needle).data, nl);
            for i in 0..=(sl - nl) {
                if hay[i..i + nl] == *pat { return i as i64; }
            }
            -1
        }
    }

    fn str_substring(&self, s: *mut MyStr, start: u64, length: u64) -> *mut MyStr {
        unsafe {
            if s.is_null() { return self.str_from_buf(core::ptr::null(), 0); }
            let sl = (*s).len;
            let start = start.min(sl);
            let length = length.min(sl - start);
            self.str_from_buf((*s).data.add(start as usize), length as usize)
        }
    }

    fn str_trim(&self, s: *mut MyStr) -> *mut MyStr {
        unsafe {
            if s.is_null() { return self.str_from_buf(core::ptr::null(), 0); }
            let sl = (*s).len as usize;
            let bytes = core::slice::from_raw_parts((*s).data, sl);
            let mut begin = 0usize;
            let mut end = sl;
            while begin < end {
                let (c, step) = utf8_decode(bytes, begin);
                if !is_space(c) { break; }
                begin += step;
            }
            while end > begin {
                let mut i = begin;
                let mut last = begin;
                while i < end {
                    last = i;
                    i += utf8_step(bytes, i);
                }
                let (c, _) = utf8_decode(bytes, last);
                if !is_space(c) { break; }
                end = last;
            }
            self.str_from_buf((*s).data.add(begin), end - begin)
        }
    }

    fn str_replace(&self, s: *mut MyStr, from: *mut MyStr, to: *mut MyStr) -> *mut MyStr {
        unsafe {
            if s.is_null() || from.is_null() || to.is_null() { return s; }
            let sl = (*s).len as usize;
            let fl = (*from).len as usize;
            let tl = (*to).len as usize;
            if fl == 0 || fl > sl { return s; }
            let hay = core::slice::from_raw_parts((*s).data, sl);
            let pat = core::slice::from_raw_parts((*from).data, fl);
            let rep = core::slice::from_raw_parts((*to).data, tl);
            let mut count = 0usize;
            let mut i = 0usize;
            while i + fl <= sl {
                if hay[i..i + fl] == *pat { count += 1; i += fl; } else { i += 1; }
            }
            if count == 0 { return s; }
            let extra = match count.checked_mul(if tl >= fl { tl - fl } else { fl - tl }) {
                Some(v) => v,
                None => return self.str_from_buf(core::ptr::null(), 0),
            };
            let total = if tl >= fl {
                match sl.checked_add(extra) {
                    Some(v) => v,
                    None => return self.str_from_buf(core::ptr::null(), 0),
                }
            } else {
                match sl.checked_sub(extra) {
                    Some(v) => v,
                    None => return self.str_from_buf(core::ptr::null(), 0),
                }
            };
            let p = self.alloc(core::mem::size_of::<MyStr>() + total + 1) as *mut u8;
            let m = p as *mut MyStr;
            let data = p.add(core::mem::size_of::<MyStr>());
            let mut out = 0usize;
            let mut j = 0usize;
            while j + fl <= sl {
                if hay[j..j + fl] == *pat {
                    core::ptr::copy_nonoverlapping(rep.as_ptr(), data.add(out), tl);
                    out += tl;
                    j += fl;
                } else {
                    *data.add(out) = hay[j];
                    out += 1;
                    j += 1;
                }
            }
            while j < sl {
                *data.add(out) = hay[j];
                out += 1;
                j += 1;
            }
            *data.add(out) = 0;
            (*m).len = out as u64;
            (*m).data = data;
            m
        }
    }

    fn str_char_len(&self, s: *mut MyStr) -> u64 {
        unsafe {
            if s.is_null() { return 0; }
            let bytes = core::slice::from_raw_parts((*s).data, (*s).len as usize);
            let mut i = 0usize;
            let mut count = 0u64;
            while i < bytes.len() {
                i += utf8_step(bytes, i);
                count += 1;
            }
            count
        }
    }

    fn str_char_index_of(&self, s: *mut MyStr, needle: *mut MyStr) -> i64 {
        let byte_pos = self.str_index_of(s, needle);
        if byte_pos < 0 { return -1; }
        unsafe {
            let bytes = core::slice::from_raw_parts((*s).data, (*s).len as usize);
            let mut i = 0usize;
            let mut count = 0i64;
            while i < byte_pos as usize {
                i += utf8_step(bytes, i);
                count += 1;
            }
            count
        }
    }

    fn str_char_substring(&self, s: *mut MyStr, start: u64, count: u64) -> *mut MyStr {
        unsafe {
            if s.is_null() { return self.str_from_buf(core::ptr::null(), 0); }
            let bytes = core::slice::from_raw_parts((*s).data, (*s).len as usize);
            let mut begin = 0usize;
            let mut chars = 0u64;
            while begin < bytes.len() && chars < start {
                begin += utf8_step(bytes, begin);
                chars += 1;
            }
            let mut end = begin;
            let mut n = 0u64;
            while end < bytes.len() && n < count {
                end += utf8_step(bytes, end);
                n += 1;
            }
            self.str_from_buf((*s).data.add(begin), end - begin)
        }
    }

    fn str_char_at(&self, s: *mut MyStr, index: u64) -> *mut MyStr {
        unsafe {
            if s.is_null() { return self.str_from_buf(core::ptr::null(), 0); }
            let bytes = core::slice::from_raw_parts((*s).data, (*s).len as usize);
            let mut i = 0usize;
            let mut n = 0u64;
            while i < bytes.len() && n < index {
                i += utf8_step(bytes, i);
                n += 1;
            }
            if i >= bytes.len() || n < index { return self.str_from_buf(core::ptr::null(), 0); }
            let step = utf8_step(bytes, i);
            self.str_from_buf((*s).data.add(i), step)
        }
    }

    fn str_char_code_at(&self, s: *mut MyStr, index: u64) -> u32 {
        unsafe {
            if s.is_null() { return 0xFFFF_FFFF; }
            let bytes = core::slice::from_raw_parts((*s).data, (*s).len as usize);
            let mut i = 0usize;
            let mut n = 0u64;
            while i < bytes.len() && n < index {
                i += utf8_step(bytes, i);
                n += 1;
            }
            if i >= bytes.len() || n < index { return 0xFFFF_FFFF; }
            utf8_decode(bytes, i).0
        }
    }

    fn str_to_bytes(&self, s: *mut MyStr) -> *mut u8 {
        unsafe {
            let len = if s.is_null() { 0 } else { (*s).len as usize };
            let p = self.arr_alloc(1, len);
            let data = p.add(core::mem::size_of::<u64>());
            if !s.is_null() && len > 0 {
                core::ptr::copy_nonoverlapping((*s).data, data, len);
            }
            p
        }
    }

    fn str_from_bytes(&self, arr: *mut u8) -> *mut MyStr {
        unsafe {
            if arr.is_null() { return self.str_from_buf(core::ptr::null(), 0); }
            let len = *(arr as *const u64) as usize;
            let data = arr.add(core::mem::size_of::<u64>());
            self.str_from_buf(data, len)
        }
    }

    fn str_repeat(&self, s: *mut MyStr, count: u64) -> *mut MyStr {
        unsafe {
            if s.is_null() || count == 0 || (*s).len == 0 {
                return self.str_from_buf(core::ptr::null(), 0);
            }
            let len = (*s).len;
            let Some(total) = len.checked_mul(count) else {
                return self.str_from_buf(core::ptr::null(), 0);
            };
            let total = total as usize;
            let p = self.alloc(core::mem::size_of::<MyStr>() + total + 1) as *mut u8;
            let m = p as *mut MyStr;
            let data = p.add(core::mem::size_of::<MyStr>());
            for i in 0..count as usize {
                core::ptr::copy_nonoverlapping((*s).data, data.add(i * len as usize), len as usize);
            }
            *data.add(total) = 0;
            (*m).len = total as u64;
            (*m).data = data;
            m
        }
    }

    fn clock(&self) -> f64 { plat::platform().clock() }

    fn stats_dump(&self) {}
}

pub struct StandardRuntime<G: Gc> { pub gc: G }

impl<G: Gc> Runtime for StandardRuntime<G> { fn gc(&self) -> &dyn Gc { &self.gc } }

fn utf8_decode(bytes: &[u8], i: usize) -> (u32, usize) {
    if i >= bytes.len() { return (0, 1); }
    let b = bytes[i];
    if b & 0x80 == 0 { return (b as u32, 1); }
    if b & 0xE0 == 0xC0 && i + 1 < bytes.len() && bytes[i + 1] & 0xC0 == 0x80 {
        return (((b & 0x1F) as u32) << 6 | (bytes[i + 1] & 0x3F) as u32, 2);
    }
    if b & 0xF0 == 0xE0 && i + 2 < bytes.len() && bytes[i + 1] & 0xC0 == 0x80
        && bytes[i + 2] & 0xC0 == 0x80 {
        return (((b & 0x0F) as u32) << 12
            | (((bytes[i + 1] & 0x3F) as u32) << 6)
            | (bytes[i + 2] & 0x3F) as u32, 3);
    }
    if b & 0xF8 == 0xF0 && i + 3 < bytes.len() && bytes[i + 1] & 0xC0 == 0x80
        && bytes[i + 2] & 0xC0 == 0x80 && bytes[i + 3] & 0xC0 == 0x80 {
        return (((b & 0x07) as u32) << 18
            | (((bytes[i + 1] & 0x3F) as u32) << 12)
            | (((bytes[i + 2] & 0x3F) as u32) << 6)
            | (bytes[i + 3] & 0x3F) as u32, 4);
    }
    (b as u32, 1)
}

fn utf8_step(bytes: &[u8], i: usize) -> usize { utf8_decode(bytes, i).1 }

fn is_space(c: u32) -> bool {
    matches!(c, 0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0x85 | 0xA0 | 0x1680
        | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000)
        || (0x2000..=0x200A).contains(&c)
}

/// 小型栈上文本缓冲：避免在 no_std 里依赖 `format!` / `String`。
struct Text { buf: [u8; 128], len: usize }

impl Text {
    fn new() -> Text { Text { buf: [0; 128], len: 0 } }
    fn push_bytes(&mut self, s: &[u8]) {
        let n = s.len().min(self.buf.len().saturating_sub(self.len));
        self.buf[self.len..self.len + n].copy_from_slice(&s[..n]);
        self.len += n;
    }
    fn push_str(&mut self, s: &str) { self.push_bytes(s.as_bytes()); }
    fn push_u64(&mut self, mut v: u64) {
        let mut digits = [0u8; 20];
        let mut n = 0;
        if v == 0 { self.push_bytes(b"0"); return; }
        while v > 0 { digits[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
        let mut out = [0u8; 20];
        for i in 0..n { out[i] = digits[n - 1 - i]; }
        self.push_bytes(&out[..n]);
    }
    fn push_usize(&mut self, v: usize) { self.push_u64(v as u64); }
    fn push_i64(&mut self, v: i64) {
        if v < 0 { self.push_bytes(b"-"); self.push_u64(v.unsigned_abs()); }
        else { self.push_u64(v as u64); }
    }
    fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }
}

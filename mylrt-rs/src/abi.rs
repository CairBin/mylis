#[repr(C)]
pub struct MyStr {
    pub len: u64,
    pub data: *mut u8,
}

use crate::gc::{BackendGc, Gc};
use crate::runtime::{Runtime, StandardRuntime};

static GC: BackendGc = BackendGc;
static RT: OnceRt = OnceRt;

struct OnceRt;
impl Runtime for OnceRt { fn gc(&self) -> &dyn Gc { &GC } }

pub fn runtime() -> &'static dyn Runtime { &RT }

// 显式引用 StandardRuntime，保持默认实现可被第三方复用。
#[allow(dead_code)]
fn _default() -> StandardRuntime<BackendGc> { StandardRuntime { gc: BackendGc } }

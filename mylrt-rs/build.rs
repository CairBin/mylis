use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=MYLRT_IMPL_RS");
    println!("cargo:rerun-if-env-changed=MYLRT_PLATFORM_RS");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    let impl_rs = env::var("MYLRT_IMPL_RS").unwrap_or_else(|_| "src/impls/own.rs".into());
    let impl_abs = PathBuf::from(&impl_rs);
    let impl_abs = if impl_abs.is_absolute() { impl_abs } else { PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join(impl_abs) };
    println!("cargo:rerun-if-changed={}", impl_abs.display());
    fs::write(out.join("selected.rs"), format!("include!({:?});\n", impl_abs)).unwrap();

    let plat_rs = env::var("MYLRT_PLATFORM_RS").unwrap_or_else(|_| "src/platforms/host.rs".into());
    let plat_abs = PathBuf::from(&plat_rs);
    let plat_abs = if plat_abs.is_absolute() { plat_abs } else { PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join(plat_abs) };
    println!("cargo:rerun-if-changed={}", plat_abs.display());
    fs::write(out.join("selected_platform.rs"), format!("include!({:?});\n", plat_abs)).unwrap();
}

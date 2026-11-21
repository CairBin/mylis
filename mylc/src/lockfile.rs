//! `mylis.lock`：P0 的路径依赖锁定与包内容校验和。
//!
//! 只在锁文件不存在时生成；已存在时不静默改写（后续版本再做完整性校验与更新命令）。

use crate::manifest::{self, LockPin, LockPins};
use crate::sha256;
use std::path::{Path, PathBuf};

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else { return; };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().map(|n| n == "target" || n == "build" || n == ".git").unwrap_or(false) {
                continue;
            }
            collect_files(&p, out);
        } else if p.extension().map(|x| x == "myl").unwrap_or(false)
            || p.file_name().map(|x| x == manifest::FILE).unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// 包内容校验和：所有 `.myl` 与 `mylis.toml` 按路径排序后，依次哈希“相对路径 + 内容”。
pub fn package_checksum(root: &Path) -> String {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files.sort();
    let mut buf = String::new();
    for f in files {
        let rel = f.strip_prefix(root).unwrap_or(&f).to_string_lossy();
        buf.push_str(&rel);
        buf.push('\0');
        if let Ok(data) = std::fs::read(&f) {
            buf.push_str(&sha256::hex(&data));
        }
        buf.push('\0');
    }
    format!("sha256-{}", sha256::hex(buf.as_bytes()))
}

pub fn build_lock(root_manifest: &Path) -> Result<String, String> {
    build_lock_offline(root_manifest, false)
}

pub fn build_lock_offline(root_manifest: &Path, offline: bool) -> Result<String, String> {
    let graph = crate::manifest::build_graph_with_pins_offline(root_manifest, None, offline)?;
    let mut s = String::from("# 由 mylc 生成；路径、Git 与 .mylib 依赖的锁定结果。\nversion = 1\n");
    for p in &graph.packages {
        s.push_str(&format!(
            "\n[[package]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"{}\"\nchecksum = \"{}\"\n",
            p.name, p.version, p.source, p.checksum
        ));
    }
    Ok(s)
}

/// 确保锁文件存在。`force` 为 true 时重新生成。返回 true 表示本次写入锁文件。
pub fn ensure(root_manifest: &Path, force: bool) -> Result<bool, String> {
    ensure_offline(root_manifest, force, false)
}

pub fn ensure_offline(root_manifest: &Path, force: bool, offline: bool) -> Result<bool, String> {
    let lock = root_manifest.parent().unwrap_or(Path::new(".")).join("mylis.lock");
    if lock.is_file() && !force {
        return Ok(false);
    }
    let text = build_lock_offline(root_manifest, offline)?;
    std::fs::write(&lock, text).map_err(|e| format!("写入 {} 失败: {}", lock.display(), e))?;
    Ok(true)
}
/// 读取锁文件中的包版本/源；文件不存在时返回 None。
pub fn read_pins(root_manifest: &Path) -> Option<LockPins> {
    let lock = root_manifest.parent().unwrap_or(Path::new(".")).join("mylis.lock");
    let text = std::fs::read_to_string(&lock).ok()?;
    let mut pins = LockPins::default();
    let mut cur: Option<(String, String, String, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line == "[[package]]" {
            if let Some((n, v, s, c)) = cur.take() {
                pins.insert(n, LockPin { version: v, source: s, checksum: if c.is_empty() { None } else { Some(c) } });
            }
            cur = Some((String::new(), String::new(), String::new(), String::new()));
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if let Some(c) = cur.as_mut() {
                let val = v.trim().trim_matches('"').to_string();
                match k.trim() {
                    "name" => c.0 = val,
                    "version" => c.1 = val,
                    "source" => c.2 = val,
                    "checksum" => c.3 = val,
                    _ => {}
                }
            }
        }
    }
    if let Some((n, v, s, c)) = cur {
        pins.insert(n, LockPin { version: v, source: s, checksum: if c.is_empty() { None } else { Some(c) } });
    }
    Some(pins)
}


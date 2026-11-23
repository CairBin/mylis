#!/usr/bin/env python3
"""myls 语言服务器冒烟测试：诊断 / 符号 / 悬停 / 跳转 / 补全 / 引用 / 重命名 / 工作区符号 / 签名。"""
import json, os, subprocess, sys
from pathlib import Path

SERVER = sys.argv[1] if len(sys.argv) > 1 else "./build/myls"
fails = 0

proc = subprocess.Popen([SERVER], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
buf = b""

def send(obj):
    data = json.dumps(obj).encode()
    proc.stdin.write(("Content-Length: %d\r\n\r\n" % len(data)).encode() + data)
    proc.stdin.flush()

def recv():
    global buf
    while b"\r\n\r\n" not in buf:
        c = proc.stdout.read(1)
        if not c:
            return None
        buf += c
    head, rest = buf.split(b"\r\n\r\n", 1)
    length = int([l for l in head.decode().split("\r\n") if l.lower().startswith("content-length")][0].split(":")[1])
    while len(rest) < length:
        rest += proc.stdout.read(length - len(rest))
    buf = rest[length:]
    return json.loads(rest[:length])

def check(name, cond, extra=""):
    global fails
    if cond:
        print("  ok   %s" % name)
    else:
        print("  FAIL %s %s" % (name, extra))
        fails += 1

root = str(Path(".").resolve())
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"rootPath": root, "rootUri": Path(root).as_uri()}})
init = recv()
check("initialize", init and init["result"]["serverInfo"]["name"] == "myls")
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

src_path = Path("tests/positive/ComboIface.myl").resolve()
src = src_path.read_text(encoding="utf-8")
uri = src_path.as_uri()
send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
      "params": {"textDocument": {"uri": uri, "languageId": "mylis", "version": 1, "text": src}}})
diag = recv()
check("无错误文档诊断为空", diag["params"]["diagnostics"] == [])

bad = src.replace("interface Reader {", "interface Reader {{")
send({"jsonrpc": "2.0", "method": "textDocument/didChange",
      "params": {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": bad}]}})
diag = recv()
check("语法错误产生诊断", len(diag["params"]["diagnostics"]) > 0,
      str(diag["params"]["diagnostics"][:1]))

send({"jsonrpc": "2.0", "method": "textDocument/didChange",
      "params": {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": src}]}})
recv()

send({"jsonrpc": "2.0", "id": 2, "method": "textDocument/documentSymbol",
      "params": {"textDocument": {"uri": uri}}})
syms = recv()["result"]
names = [s["name"] for s in syms]
check("文档符号只含本文件", "ComboIface" not in names and "Reader" in names and "use_all" in names, str(names))

lines = src.split("\n")
ln = next(i for i, l in enumerate(lines) if "fn use_all" in l)
ch = lines[ln].find("ReadWriteCloser") + 2
send({"jsonrpc": "2.0", "id": 3, "method": "textDocument/hover",
      "params": {"textDocument": {"uri": uri}, "position": {"line": ln, "character": ch}}})
hover = recv()["result"]
check("悬停给出签名", hover and "interface ReadWriteCloser" in hover["contents"]["value"], str(hover))

send({"jsonrpc": "2.0", "id": 4, "method": "textDocument/definition",
      "params": {"textDocument": {"uri": uri}, "position": {"line": ln, "character": ch}}})
defloc = recv()["result"]
check("跳转到声明", defloc and defloc["uri"].lower().endswith("comboiface.myl")
      and defloc["range"]["start"]["line"] == next(i for i, l in enumerate(lines) if "interface ReadWriteCloser" in l), str(defloc))

send({"jsonrpc": "2.0", "id": 5, "method": "textDocument/completion",
      "params": {"textDocument": {"uri": uri}, "position": {"line": ln, "character": 0}}})
items = [x["label"] for x in recv()["result"]["items"]]
check("补全含项目类型与关键字", "ReadWriteCloser" in items and "Buf" in items and "fn" in items)

# ---- P1：references / prepareRename / rename / workspace symbol / signatureHelp ----
ref_ln = next(i for i, l in enumerate(lines) if "interface Reader" in l)
ref_ch = lines[ref_ln].find("Reader") + 2
base_params = {"textDocument": {"uri": uri}, "position": {"line": ref_ln, "character": ref_ch}}

send({"jsonrpc": "2.0", "id": 6, "method": "textDocument/references",
      "params": dict(base_params, context={"includeDeclaration": True})})
refs = recv()["result"]
check("查找引用包含声明与使用", len(refs) >= 2 and all(r["uri"].endswith("ComboIface.myl") for r in refs), str(refs))
send({"jsonrpc": "2.0", "id": 7, "method": "textDocument/references",
      "params": dict(base_params, context={"includeDeclaration": False})})
refs_no_decl = recv()["result"]
check("查找引用可排除声明", len(refs_no_decl) == len(refs) - 1, "%d vs %d" % (len(refs_no_decl), len(refs)))

send({"jsonrpc": "2.0", "id": 8, "method": "textDocument/prepareRename", "params": base_params})
prep = recv()["result"]
check("重命名预检返回旧名范围", prep and prep.get("placeholder") == "Reader"
      and prep["range"]["start"]["character"] == lines[ref_ln].find("Reader"), str(prep))

send({"jsonrpc": "2.0", "id": 9, "method": "textDocument/rename",
      "params": dict(base_params, newName="Input")})
ren = recv()["result"]
edits = list((ren or {}).get("changes", {}).values())
check("重命名返回工作区编辑", bool(edits) and all(e["newText"] == "Input" for group in edits for e in group), str(ren))
check("重命名不直接改文件", open("tests/positive/ComboIface.myl", encoding="utf-8").read() == src)

local_ln = next(i for i, l in enumerate(lines) if "Buf b" in l)
local_ch = lines[local_ln].find("b") + 1
send({"jsonrpc": "2.0", "id": 10, "method": "textDocument/prepareRename",
      "params": {"textDocument": {"uri": uri}, "position": {"line": local_ln, "character": local_ch}}})
check("局部变量暂不允许重命名", recv()["result"] is None)

send({"jsonrpc": "2.0", "id": 11, "method": "workspace/symbol", "params": {"query": "Buf"}})
wsyms = recv()["result"]
check("工作区符号能查到 Buf", any(s["name"] == "Buf" for s in wsyms), str([s["name"] for s in wsyms][:10]))

sig_ln = next(i for i, l in enumerate(lines) if "new Buf(" in l)
sig_ch = lines[sig_ln].find("new Buf(") + len("new Buf(")
send({"jsonrpc": "2.0", "id": 12, "method": "textDocument/signatureHelp",
      "params": {"textDocument": {"uri": uri}, "position": {"line": sig_ln, "character": sig_ch}}})
sig = recv()["result"]
check("构造器签名帮助", sig and sig["signatures"][0]["label"].startswith("new Buf("), str(sig))

call_ln = next(i for i, l in enumerate(lines) if "use_all(b)" in l)
call_ch = lines[call_ln].find("use_all(") + len("use_all(")
send({"jsonrpc": "2.0", "id": 13, "method": "textDocument/signatureHelp",
      "params": {"textDocument": {"uri": uri}, "position": {"line": call_ln, "character": call_ch}}})
sig = recv()["result"]
check("函数签名帮助", sig and "use_all" in sig["signatures"][0]["label"]
      and sig["activeParameter"] == 0, str(sig))

send({"jsonrpc": "2.0", "id": 14, "method": "shutdown", "params": {}})
recv()
send({"jsonrpc": "2.0", "method": "exit", "params": {}})
proc.wait(timeout=10)

print("\nLSP 通过：%d 项失败" % fails)
sys.exit(1 if fails else 0)

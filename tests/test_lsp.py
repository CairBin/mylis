#!/usr/bin/env python3
"""myls 语言服务器冒烟测试：初始化 → 打开文档（带错误）→ 诊断 / 符号 / 悬停 / 跳转 / 补全。"""
import json, os, subprocess, sys

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

root = os.path.abspath(".")
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"rootPath": root}})
init = recv()
check("initialize", init and init["result"]["serverInfo"]["name"] == "myls")
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

src = open("tests/positive/ComboIface.myl", encoding="utf-8").read()
uri = "file://" + os.path.join(root, "tests/positive/ComboIface.myl")
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
check("跳转到声明", defloc and defloc["uri"].endswith("ComboIface.myl")
      and defloc["range"]["start"]["line"] == next(i for i, l in enumerate(lines) if "interface ReadWriteCloser" in l), str(defloc))

send({"jsonrpc": "2.0", "id": 5, "method": "textDocument/completion",
      "params": {"textDocument": {"uri": uri}, "position": {"line": ln, "character": 0}}})
items = [x["label"] for x in recv()["result"]["items"]]
check("补全含项目类型与关键字", "ReadWriteCloser" in items and "Buf" in items and "fn" in items)

send({"jsonrpc": "2.0", "id": 9, "method": "shutdown", "params": {}})
recv()
send({"jsonrpc": "2.0", "method": "exit", "params": {}})
proc.wait(timeout=10)

print("\nLSP 通过：%d 项失败" % fails)
sys.exit(1 if fails else 0)

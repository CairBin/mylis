#!/usr/bin/env bash
# MIR-C 实验后端验收：基本类型/局部/算术/循环/直接调用 能生成 C、编译、运行。
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MYLC="$ROOT/build/mylc"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

cat > demo.myl <<'M'
fn add(i32 a, i32 b) -> i32 {
    return a + b;
}

pub fn main() -> i32 {
    i32 x = 3;
    while (x < 10) {
        x = add(x, 1);
    }
    return x;
}
M

"$MYLC" --backend mir --emit-c demo.c demo.myl
grep -q "MIR v0 subset" demo.c
grep -q "L_while_cond" demo.c

"$MYLC" --backend mir demo.myl -o demo
set +e
./demo
code=$?
set -e
[ "$code" -eq 10 ] || { echo "expected exit 10, got $code"; exit 1; }
cat > class_demo.myl <<'M'
class Counter {
    i32 v;
    fn init(i32 n) -> void { this.v = n; }
    fn inc() -> i32 {
        this.v = this.v + 1;
        return this.v;
    }
}

pub fn main() -> i32 {
    Counter c = new Counter(5);
    c.inc();
    return c.v;
}
M
"$MYLC" --backend mir class_demo.myl -o class_demo
set +e
./class_demo
code=$?
set -e
[ "$code" -eq 6 ] || { echo "expected class exit 6, got $code"; exit 1; }
echo "  ok 基本类/字段/构造器/方法"
cat > strarr.myl <<'M'
import { println, println_i32 } from "Io.myl";
pub fn main() -> i32 {
    string a = "hello";
    string b = "world";
    println("s = {a}, {b}");
    if (a == b) { println("eq"); } else { println("ne"); }
    string[] xs = ["x", "y", "z"];
    println(xs[1]);
    println("len = {xs.len()}");
    i32[] ns = [3, 1, 4];
    ns[0] = 9;
    println_i32(ns[0]);
    return 0;
}
M
"$MYLC" --backend mir strarr.myl -o strarr
./strarr > strarr.out
printf 's = hello, world\nne\ny\nlen = 3\n9\n' | diff -u - strarr.out
echo "  ok 字符串/插值/数组/len"

cat > enums.myl <<'M'
import { println, println_i32 } from "Io.myl";

interface Show {
    fn label() -> string;
    fn to_string() -> string;
}

enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Empty,

    pub fn area() -> f64 {
        match this {
            Shape::Circle(r) => { return 3.14 * r * r; }
            Shape::Rect(w, h) => { return w * h; }
            Shape::Empty => { return 0.0; }
        }
    }
    pub fn label() -> string {
        match this {
            Shape::Circle(_r) => { return "circle"; }
            Shape::Rect(_w, _h) => { return "rect"; }
            Shape::Empty => { return "empty"; }
        }
    }
    pub fn to_string() -> string {
        match this {
            Shape::Circle(r) => { return "C({r})"; }
            Shape::Rect(w, h) => { return "R({w}x{h})"; }
            Shape::Empty => { return "E"; }
        }
    }
}

fn main() -> i32 {
    Shape c = Shape::Circle(2.0);
    Shape e = Shape::Empty;
    Show s = Shape::Rect(1.0, 2.0);
    println("{c} area={c.area()} label={c.label()}");
    println("{e} area={e.area()} label={e.label()}");
    println("iface = {s} / {s.label()}");
    Shape[] all = [Shape::Circle(1.0), Shape::Empty, Shape::Rect(2.0, 2.0)];
    println("all = {all}");
    return 0;
}
M
"$MYLC" --backend mir enums.myl -o enums
./enums > enums.out
printf 'C(2) area=12.56 label=circle\nE area=0 label=empty\niface = R(1x2) / rect\nall = [C(1), E, R(2x2)]\n' | diff -u - enums.out
echo "  ok 枚举/接口/变体载荷/match/数组插值"

"$MYLC" --backend mir "$ROOT/tests/positive/Results.myl" -o results
./results > results.out
printf '5\ntrue\ntrue\n-1\n10\npropagated\n10\ntrue\n0\n' | diff -u - results.out
echo "  ok Result/Option/\`?\`/unwrap"

"$MYLC" --backend mir "$ROOT/tests/positive/Registry.myl" -o registry
./registry > registry.out
printf 'ids 1 2 3\nregistry: 3\nitems: 3\n' | diff -u - registry.out
echo "  ok 静态字段/静态方法"

"$MYLC" --backend mir "$ROOT/tests/positive/ComboIface.myl" -o combo
./combo > combo.out
printf '17\n23\n1\n' | diff -u - combo.out
echo "  ok 接口组合/适配 vtable"

"$MYLC" --backend mir "$ROOT/tests/positive/Unsafe.myl" -o unsafe
./unsafe > unsafe.out
printf '42\n' | diff -u - unsafe.out
echo "  ok 原生指针 load/store"

"$MYLC" --backend mir "$ROOT/tests/positive/Ffi.myl" -o ffi
./ffi > ffi.out
printf '5\n' | diff -u - ffi.out
echo "  ok 外部 C 符号"

echo "MIR-C 验收通过"

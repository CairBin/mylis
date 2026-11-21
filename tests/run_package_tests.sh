#!/usr/bin/env bash
# 包级回归：嵌套模块路径、选择性再导出可见性、.mylib 原生载荷 LLVM 声明。
set -u

cd "$(dirname "$0")/.." || exit 1
ROOT=$PWD
MYLC="$ROOT/build/mylc"
OUT="$ROOT/build/package-tests"
export MYLIS_ROOT="$ROOT"
: "${MYLIS_PATH:=$ROOT:$ROOT/std/src}"
export MYLIS_PATH
mkdir -p "$OUT"
rm -rf "$OUT/nested_modules" "$OUT/visibility" "$OUT/native_llvm" "$OUT/mixed_link" "$OUT/std_package" "$OUT/core_package"
cp -R tests/package/nested_modules "$OUT/nested_modules"
cp -R tests/package/std_package "$OUT/std_package"
cp -R tests/package/core_package "$OUT/core_package"
mkdir -p "$OUT/std_package/app/lib"
cp -R "$ROOT/core" "$OUT/std_package/app/lib/core"
cp -R "$ROOT/std" "$OUT/std_package/app/lib/std"
mkdir -p "$OUT/core_package/app/lib"
cp -R "$ROOT/core" "$OUT/core_package/app/lib/core"
cp -R "$ROOT/std" "$OUT/core_package/app/lib/std"
cp -R tests/package/visibility "$OUT/visibility"
cp -R tests/package/native_llvm "$OUT/native_llvm"
cp -R tests/package/mixed_link "$OUT/mixed_link"

pass=0
fail=0

check_run() {
  local name=$1 app=$2 expected=$3
  if ! "$MYLC" "$app/src/main.myl" --backend mir --update-lock -o "$app/app" >"$OUT/$name.build.log" 2>&1; then
    echo "  FAIL $name（编译失败）"
    sed -n '1,6p' "$OUT/$name.build.log"
    fail=$((fail + 1)); return
  fi
  if ! "$app/app" >"$OUT/$name.actual" 2>"$OUT/$name.err"; then
    echo "  FAIL $name（运行失败）"
    sed -n '1,4p' "$OUT/$name.err"
    fail=$((fail + 1)); return
  fi
  if diff -u "$expected" "$OUT/$name.actual" >"$OUT/$name.diff"; then
    echo "  ok   $name"
    pass=$((pass + 1))
  else
    echo "  FAIL $name（输出不符）"
    sed -n '1,10p' "$OUT/$name.diff"
    fail=$((fail + 1))
  fi
}

echo "== 包级用例 =="
check_run "std_package" "$OUT/std_package/app" "$OUT/std_package/expected.txt"
check_run "core_package" "$OUT/core_package/app" "$OUT/core_package/expected.txt"
check_run "nested_modules" "$OUT/nested_modules/app" "$OUT/nested_modules/expected.txt"
check_run "visibility" "$OUT/visibility/app" "$OUT/visibility/expected.txt"

# 选择性再导出只暴露名字，不暴露中间模块。
if "$MYLC" "$OUT/visibility/app/src/leak.myl" --check --update-lock >"$OUT/visibility.leak.log" 2>&1; then
  echo "  FAIL visibility_leak（本应报错，却通过了检查）"
  fail=$((fail + 1))
elif grep -q 'error\[MYL102\]' "$OUT/visibility.leak.log"; then
  echo "  ok   visibility_leak ( MYL102 )"
  pass=$((pass + 1))
else
  echo "  FAIL visibility_leak（未出现 MYL102）"
  sed -n '1,6p' "$OUT/visibility.leak.log"
  fail=$((fail + 1))
fi

# 原生载荷：用 cc/ar 生成静态库，嵌入 .mylib。
NATIVE="$OUT/native_llvm"
if ! cc -c "$NATIVE/native/answer.c" -o "$NATIVE/native/answer.o" >"$OUT/native.cc.log" 2>&1; then
  echo "  FAIL native_llvm（C 编译失败）"
  sed -n '1,4p' "$OUT/native.cc.log"; fail=$((fail + 1))
elif ! ar rcs "$NATIVE/native/libanswer.a" "$NATIVE/native/answer.o" >>"$OUT/native.ar.log" 2>&1; then
  echo "  FAIL native_llvm（ar 失败）"
  sed -n '1,4p' "$OUT/native.ar.log"; fail=$((fail + 1))
elif ! "$MYLC" "$NATIVE/lib/src/lib.myl" --emit-mylib "$NATIVE/native_lib.mylib" --embed-static "$NATIVE/native/libanswer.a" >"$OUT/native.mylib.log" 2>&1; then
  echo "  FAIL native_llvm（生成 .mylib 失败）"
  sed -n '1,6p' "$OUT/native.mylib.log"; fail=$((fail + 1))
else
  if grep -q '^native_target = "' "$NATIVE/native_lib.mylib" \
      && grep -q '^native_abi = "' "$NATIVE/native_lib.mylib"; then
    echo "  ok   native_llvm（target/ABI 元数据）"
    pass=$((pass + 1))
  else
    echo "  FAIL native_llvm（缺少 target/ABI 元数据）"
    sed -n '1,10p' "$NATIVE/native_lib.mylib"; fail=$((fail + 1))
  fi
  # C/MIR 后端真实链接运行
  if "$MYLC" "$NATIVE/app/src/main.myl" --backend mir --link-mylib "$NATIVE/native_lib.mylib" -o "$NATIVE/app/app" >"$OUT/native.app.log" 2>&1 \
      && "$NATIVE/app/app" >"$OUT/native.actual" 2>"$OUT/native.err" \
      && diff -u "$NATIVE/expected.txt" "$OUT/native.actual" >"$OUT/native.diff"; then
    echo "  ok   native_llvm（C 链接执行）"
    pass=$((pass + 1))
  else
    echo "  FAIL native_llvm（C 链接执行失败）"
    sed -n '1,8p' "$OUT/native.app.log"; sed -n '1,6p' "$OUT/native.err"; fail=$((fail + 1))
  fi

  # LLVM：验证 IR 声明/调用；有 clang 时再真实链接执行。
  if "$MYLC" "$NATIVE/app/src/main.myl" --emit-llvm "$OUT/native.lll" --link-mylib "$NATIVE/native_lib.mylib" >"$OUT/native.llvm.log" 2>&1; then
    if grep -q 'declare i32 @answer()' "$OUT/native.lll" \
        && grep -q 'call i32 @answer()' "$OUT/native.lll" \
        && ! grep -q 'define i32 @answer()' "$OUT/native.lll"; then
      echo "  ok   native_llvm（LLVM 外部声明）"
      pass=$((pass + 1))
    else
      echo "  FAIL native_llvm（LLVM IR 未生成外部声明）"
      sed -n '1,6p' "$OUT/native.llvm.log"; fail=$((fail + 1))
    fi
  else
    echo "  FAIL native_llvm（LLVM IR 生成失败）"
    sed -n '1,6p' "$OUT/native.llvm.log"; fail=$((fail + 1))
  fi

  if command -v clang >/dev/null 2>&1 || command -v clang-15 >/dev/null 2>&1; then
    if "$MYLC" "$NATIVE/app/src/main.myl" --backend llvm --link-mylib "$NATIVE/native_lib.mylib" -o "$NATIVE/app/app_llvm" >"$OUT/native.llvm.link.log" 2>&1 \
        && "$NATIVE/app/app_llvm" >"$OUT/native.llvm.actual" 2>"$OUT/native.llvm.err" \
        && diff -u "$NATIVE/expected.txt" "$OUT/native.llvm.actual" >"$OUT/native.llvm.diff"; then
      echo "  ok   native_llvm（LLVM 真实链接执行）"
      pass=$((pass + 1))
    else
      echo "  FAIL native_llvm（LLVM 真实链接执行失败）"
      sed -n '1,8p' "$OUT/native.llvm.link.log"; sed -n '1,6p' "$OUT/native.llvm.err"; fail=$((fail + 1))
    fi
  else
    echo "  skip native_llvm（未安装 clang，跳过 LLVM 真实链接）"
  fi
fi

# 混合链接：一个原生载荷 .mylib + 一个普通 MIR .mylib。
MIXED="$OUT/mixed_link"
if cc -c "$MIXED/native/answer.c" -o "$MIXED/native/answer.o" >"$OUT/mixed.cc.log" 2>&1 \
    && ar rcs "$MIXED/native/libanswer.a" "$MIXED/native/answer.o" >>"$OUT/mixed.ar.log" 2>&1 \
    && "$MYLC" "$MIXED/native_lib/src/lib.myl" --emit-mylib "$MIXED/native_lib.mylib" --embed-static "$MIXED/native/libanswer.a" >"$OUT/mixed.native.log" 2>&1 \
    && "$MYLC" "$MIXED/mir_lib/src/lib.myl" --emit-mylib "$MIXED/mir_lib.mylib" >"$OUT/mixed.mir.log" 2>&1; then
  for backend in mir llvm; do
    name="mixed_link（${backend}）"
    if "$MYLC" "$MIXED/app/src/main.myl" --backend "$backend" -o "$MIXED/app/app_$backend" >"$OUT/mixed.$backend.log" 2>&1 \
        && "$MIXED/app/app_$backend" >"$OUT/mixed.$backend.actual" 2>"$OUT/mixed.$backend.err" \
        && diff -u "$MIXED/expected.txt" "$OUT/mixed.$backend.actual" >"$OUT/mixed.$backend.diff"; then
      echo "  ok   $name"
      pass=$((pass + 1))
    else
      echo "  FAIL $name"
      sed -n '1,8p' "$OUT/mixed.$backend.log"; sed -n '1,6p' "$OUT/mixed.$backend.err"; fail=$((fail + 1))
    fi
  done
  # LLVM 混合链接：原生函数应为 declare，MIR 库函数应定义并调用，不能重复声明。
  if "$MYLC" "$MIXED/app/src/main.myl" --emit-llvm "$OUT/mixed.lll" >"$OUT/mixed.ir.log" 2>&1; then
    if grep -q 'declare i32 @answer()' "$OUT/mixed.lll" \
        && grep -q 'define i32 @myl_mir_lib__seven()' "$OUT/mixed.lll" \
        && grep -q 'call i32 @myl_mir_lib__seven()' "$OUT/mixed.lll" \
        && ! grep -q 'declare i32 @myl_mir_lib__seven' "$OUT/mixed.lll"; then
      echo "  ok   mixed_link（LLVM 外部/内部符号区分）"
      pass=$((pass + 1))
    else
      echo "  FAIL mixed_link（LLVM 外部/内部符号区分）"
      grep -n 'seven\|answer' "$OUT/mixed.lll" | head -8; fail=$((fail + 1))
    fi
  else
    echo "  FAIL mixed_link（LLVM IR 生成失败）"
    sed -n '1,6p' "$OUT/mixed.ir.log"; fail=$((fail + 1))
  fi
else
  echo "  FAIL mixed_link（准备归档失败）"
  sed -n '1,6p' "$OUT/mixed.native.log"; sed -n '1,6p' "$OUT/mixed.mir.log"; fail=$((fail + 1))
fi


# 本地 .mylib：API 与 std/lib.myl 同名，验证 --link-mylib 优先使用归档 API，
# 且 MIR 函数能在 MIR-C / AST-C 两个 C 后端里合并执行。
LOCAL="$OUT/local_mylib"
rm -rf "$LOCAL"
mkdir -p "$LOCAL/libpkg"
cat > "$LOCAL/libpkg/lib.myl" <<'MYL'
pub fn add(i64 a, i64 b) -> i64 {
    return a + b;
}
MYL
cat > "$LOCAL/main.myl" <<'MYL'
import { println } from "Io.myl";
import { add } from "lib";

fn main() -> i32 {
    println("{add(2, 3)}");
    return 0;
}
MYL
if ! "$MYLC" "$LOCAL/libpkg/lib.myl" --emit-mylib "$LOCAL/lib.mylib" >"$OUT/local_mylib.emit.log" 2>&1; then
  echo "  FAIL local_mylib（生成 .mylib 失败）"
  sed -n '1,6p' "$OUT/local_mylib.emit.log"; fail=$((fail + 1))
else
  # AST-C 不消费 MIR，.mylib 的 MIR 链接只支持 mir/llvm。
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$LOCAL/libpkg" "$MYLC" "$LOCAL/main.myl" --backend mir --link-mylib "$LOCAL/lib.mylib" -o "$LOCAL/main_mir" >"$OUT/local_mylib.mir.log" 2>&1 \
      && "$LOCAL/main_mir" >"$OUT/local_mylib.mir.actual" 2>"$OUT/local_mylib.mir.err" \
      && test "$(cat "$OUT/local_mylib.mir.actual")" = "5"; then
    echo "  ok   local_mylib（mir 运行）"
    pass=$((pass + 1))
  else
    echo "  FAIL local_mylib（mir 运行）"
    sed -n '1,8p' "$OUT/local_mylib.mir.log"; sed -n '1,4p' "$OUT/local_mylib.mir.err"; fail=$((fail + 1))
  fi
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$LOCAL/libpkg" "$MYLC" "$LOCAL/main.myl" --emit-llvm "$LOCAL/main.ll" --link-mylib "$LOCAL/lib.mylib" >"$OUT/local_mylib.ll.log" 2>&1 \
      && grep -q 'define i64 @myl_lib__add' "$LOCAL/main.ll" \
      && grep -q 'call i64 @myl_lib__add' "$LOCAL/main.ll"; then
    echo "  ok   local_mylib（LLVM MIR 定义）"
    pass=$((pass + 1))
  else
    echo "  FAIL local_mylib（LLVM MIR 定义）"
    sed -n '1,8p' "$OUT/local_mylib.ll.log"; fail=$((fail + 1))
  fi
fi

# 泛型 .mylib：模板 API + 稳定实例名 + 库内具体实例 MIR + 嵌套泛型跨归档链接。
GEN="$OUT/generic_mylib"
rm -rf "$GEN"
mkdir -p "$GEN/libpkg"
cat > "$GEN/libpkg/Box.myl" <<'MYL'
import { println } from "Io.myl";
import { Vec } from "Vec.myl";
import { Map } from "Map.myl";
pub class Box<T> {
    pub T v;
    pub fn init(T x) -> void { this.v = x; }
    pub fn get() -> T { return this.v; }
    pub fn list() -> Vec<T> {
        Vec<T> out = new Vec<T>();
        out.push(this.v);
        return out;
    }
    pub fn maybe(bool want) -> Option<Vec<T>> {
        if (!want) { return None; }
        return Some(this.list());
    }
    pub fn nested() -> Vec<Vec<T>> {
        Vec<Vec<T>> out = new Vec<Vec<T>>();
        out.push(this.list());
        return out;
    }
}
pub fn identity<T>(T x) -> T { return x; }
pub fn make() -> i64 { Box<i64> b = new Box(7); return b.get(); }
pub fn hello<T>(T x) -> void { println("hello"); }
pub fn countMap() -> i64 {
    Map<string, i64> m = new Map<string, i64>();
    m.set("a", 1);
    return m.len();
}
pub fn makeVec(i64 x) -> Vec<i64> {
    Vec<i64> v = new Vec<i64>();
    v.push(x);
    return v;
}
pub fn firstVec(Vec<i64> v) -> i64 { return v.get(0); }
pub fn maybeVec(i64 x, bool want) -> Option<Vec<i64>> {
    if (!want) { return None; }
    return Some(makeVec(x));
}
pub fn resultVec(i64 x) -> Result<Vec<i64>, string> {
    return Ok(makeVec(x));
}
MYL
cat > "$GEN/libpkg/Seq.myl" <<'MYL'
import { Vec } from "Vec.myl";
pub interface Seq<T> { fn list() -> Vec<T>; }
MYL
cat > "$GEN/libpkg/Node.myl" <<'MYL'
import { Vec } from "Vec.myl";
pub enum Node<T> { Leaf(T), Pair(Vec<T>) }
MYL
cat > "$GEN/main.myl" <<'MYL'
import { println_i64 } from "Io.myl";
import { Vec } from "Vec.myl";
import { Box, identity, make, hello, countMap, makeVec, maybeVec, resultVec, firstVec } from "Box";
import { Seq } from "Seq";
import { Node } from "Node";
fn main() -> i32 {
    Box<i64> b = new Box(7);
    println_i64(b.get());
    println_i64(identity(9));
    println_i64(make());
    hello(0);
    Vec<i64> v = b.list();
    println_i64(v.len());
    println_i64(v.get(0));
    Option<Vec<i64>> some = b.maybe(true);
    match (some) { Some(sv) => { println_i64(sv.len()); } None => { println_i64(-1); } }
    Vec<Vec<i64>> nested = b.nested();
    println_i64(nested.len());
    println_i64(nested.get(0).get(0));
    Seq<i64> s = b;
    println_i64(s.list().get(0));
    Node<i64> pair = Node<i64>::Pair(v);
    match (pair) { Node::Leaf(x) => { println_i64(x); } Node::Pair(vs) => { println_i64(vs.len()); } }
    println_i64(countMap());
    Vec<i64> mv = makeVec(2);
    println_i64(mv.len());
    println_i64(mv.get(0));
    println_i64(firstVec(mv));
    Option<Vec<i64>> ov = maybeVec(4, true);
    match (ov) { Some(msv) => { println_i64(msv.len()); } None => { println_i64(-1); } }
    Result<Vec<i64>, string> rv = resultVec(6);
    match (rv) { Ok(mrv) => { println_i64(mrv.get(0)); } Err(e) => { println_i64(-1); } }
    return 0;
}
MYL
EXPECT_GENERIC="$(printf '7\n9\n7\nhello\n1\n7\n1\n1\n7\n7\n1\n1\n1\n2\n2\n1\n6')"
emit_ok=1
for mod in Box Seq Node; do
  if ! MYLIS_PATH="$ROOT:$ROOT/std/src:$GEN/libpkg" "$MYLC" "$GEN/libpkg/$mod.myl" --emit-mylib "$GEN/$mod.mylib" >"$OUT/generic_mylib.$mod.emit.log" 2>&1; then
    echo "  FAIL generic_mylib（生成 $mod.mylib 失败）"
    sed -n '1,6p' "$OUT/generic_mylib.$mod.emit.log"; fail=$((fail + 1)); emit_ok=0
    break
  fi
done
if [ "$emit_ok" = 1 ]; then
  if ! MYLIS_PATH="$ROOT:$ROOT/std/src:$GEN/libpkg" "$MYLC" "$GEN/libpkg/Box.myl" --emit-mylib "$GEN/Box.repeat.mylib" >"$OUT/generic_mylib.repeat.log" 2>&1; then
    echo "  FAIL generic_mylib（重复生成归档失败）"
    sed -n '1,6p' "$OUT/generic_mylib.repeat.log"; fail=$((fail + 1)); emit_ok=0
  elif ! cmp -s "$GEN/Box.mylib" "$GEN/Box.repeat.mylib"; then
    echo "  FAIL generic_mylib（相同输入生成的归档不一致）"
    fail=$((fail + 1)); emit_ok=0
  fi
fi
if [ "$emit_ok" = 1 ]; then
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$GEN/libpkg" "$MYLC" "$GEN/main.myl" --backend mir --link-mylib "$GEN/Box.mylib" --link-mylib "$GEN/Seq.mylib" --link-mylib "$GEN/Node.mylib" -o "$GEN/main_mir" >"$OUT/generic_mylib.mir.log" 2>&1 \
      && "$GEN/main_mir" >"$OUT/generic_mylib.actual" 2>"$OUT/generic_mylib.err" \
      && test "$(cat "$OUT/generic_mylib.actual")" = "$EXPECT_GENERIC"; then
    echo "  ok   generic_mylib（泛型模板 + 嵌套实例链接）"
    pass=$((pass + 1))
  else
    echo "  FAIL generic_mylib（mir 运行）"
    sed -n '1,8p' "$OUT/generic_mylib.mir.log"; sed -n '1,4p' "$OUT/generic_mylib.err"; fail=$((fail + 1))
  fi

  if command -v clang >/dev/null 2>&1; then
    if MYLIS_PATH="$ROOT:$ROOT/std/src:$GEN/libpkg" "$MYLC" "$GEN/main.myl" --backend llvm --cc clang --link-mylib "$GEN/Box.mylib" --link-mylib "$GEN/Seq.mylib" --link-mylib "$GEN/Node.mylib" -o "$GEN/main_llvm" >"$OUT/generic_mylib.llvm.log" 2>&1 \
        && "$GEN/main_llvm" >"$OUT/generic_mylib.llvm.actual" 2>"$OUT/generic_mylib.llvm.err" \
        && test "$(cat "$OUT/generic_mylib.llvm.actual")" = "$EXPECT_GENERIC"; then
      echo "  ok   generic_mylib（LLVM 泛型嵌套实例运行）"
      pass=$((pass + 1))
    else
      echo "  FAIL generic_mylib（LLVM 运行）"
      sed -n '1,8p' "$OUT/generic_mylib.llvm.log"; sed -n '1,4p' "$OUT/generic_mylib.llvm.err"; fail=$((fail + 1))
    fi
  else
    echo "  skip generic_mylib（未安装 clang，跳过 LLVM）"
  fi
fi

# 多参数泛型：class / interface / enum 都经 .mylib 导出，消费端再实例化。
MULTI="$OUT/multi_generic_mylib"
rm -rf "$MULTI"
mkdir -p "$MULTI/libpkg"
cat > "$MULTI/libpkg/Pair.myl" <<'MYL'
pub class Pair<A, B> {
    pub A left;
    pub B right;
    pub fn init(A a, B b) -> void {
        this.left = a;
        this.right = b;
    }
    pub fn first() -> A { return this.left; }
    pub fn second() -> B { return this.right; }
    pub fn swap() -> Pair<B, A> { return new Pair<B, A>(this.right, this.left); }
}
MYL
cat > "$MULTI/libpkg/View.myl" <<'MYL'
pub interface PairView<A, B> {
    fn first() -> A;
    fn second() -> B;
}
MYL
cat > "$MULTI/libpkg/Either.myl" <<'MYL'
pub enum Either<A, B> { Left(A), Right(B) }
MYL
cat > "$MULTI/main.myl" <<'MYL'
import { println, println_i64 } from "Io.myl";
import { Pair } from "Pair";
import { PairView } from "View";
import { Either } from "Either";
fn main() -> i32 {
    Pair<i64, string> p = new Pair<i64, string>(7, "x");
    println_i64(p.first());
    println(p.second());
    Pair<string, i64> q = p.swap();
    println(q.first());
    println_i64(q.second());
    PairView<i64, string> view = p;
    println_i64(view.first());
    println(view.second());
    Either<i64, string> left = Either<i64, string>::Left(9);
    match (left) {
        Either::Left(x) => { println_i64(x); }
        Either::Right(y) => { println(y); }
    }
    return 0;
}
MYL
EXPECT_MULTI="$(printf '7\nx\nx\n7\n7\nx\n9')"
multi_ok=1
for mod in Pair View Either; do
  if ! MYLIS_PATH="$ROOT:$ROOT/std/src:$MULTI/libpkg" "$MYLC" "$MULTI/libpkg/$mod.myl" --emit-mylib "$MULTI/$mod.mylib" >"$OUT/multi_generic_mylib.$mod.emit.log" 2>&1; then
    echo "  FAIL multi_generic_mylib（生成 $mod.mylib 失败）"
    sed -n '1,6p' "$OUT/multi_generic_mylib.$mod.emit.log"; fail=$((fail + 1)); multi_ok=0
    break
  fi
done
if [ "$multi_ok" = 1 ]; then
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$MULTI/libpkg" "$MYLC" "$MULTI/main.myl" --backend mir --link-mylib "$MULTI/Pair.mylib" --link-mylib "$MULTI/View.mylib" --link-mylib "$MULTI/Either.mylib" -o "$MULTI/app" >"$OUT/multi_generic_mylib.mir.log" 2>&1 \
      && "$MULTI/app" >"$OUT/multi_generic_mylib.actual" 2>"$OUT/multi_generic_mylib.err" \
      && test "$(cat "$OUT/multi_generic_mylib.actual")" = "$EXPECT_MULTI"; then
    echo "  ok   multi_generic_mylib（多参数 class/interface/enum 跨归档）"
    pass=$((pass + 1))
  else
    echo "  FAIL multi_generic_mylib（mir 运行）"
    sed -n '1,8p' "$OUT/multi_generic_mylib.mir.log"; sed -n '1,4p' "$OUT/multi_generic_mylib.err"; fail=$((fail + 1))
  fi
fi

# 多个 .mylib 各自闭包带入同一批外部具体实例（Vec/Map）时，必须复用消费端实例，
# 不能把消费端已有 Vec::len 删掉后重新分配 id，否则其它归档方法的 Res::Method 会悬空。
CLO="$OUT/generic_closure"
rm -rf "$CLO"
mkdir -p "$CLO/libpkg"
cat > "$CLO/libpkg/FieldBox.myl" <<'MYL'
import { Vec } from "Vec.myl";
pub class FieldBox<T> {
    pub Vec<T> items;
    pub fn init(T x) -> void {
        this.items = new Vec<T>();
        this.items.push(x);
    }
    pub fn len() -> i64 { return this.items.len(); }
    pub fn first() -> T { return this.items.get(0); }
}
MYL
cat > "$CLO/libpkg/Search.myl" <<'MYL'
import { Vec } from "Vec.myl";
import { sort } from "Algorithms.myl";
pub fn sortedFirst(i64 a, i64 b) -> i64 {
    Vec<i64> v = new Vec<i64>();
    v.push(b);
    v.push(a);
    sort<i64>(v);
    return v.get(0);
}
MYL
cat > "$CLO/libpkg/NestMap.myl" <<'MYL'
import { Vec } from "Vec.myl";
import { Map } from "Map.myl";
pub fn nestedMap() -> i64 {
    Map<string, Vec<i64>> m = new Map<string, Vec<i64>>();
    Vec<i64> v = new Vec<i64>();
    v.push(11);
    m.set("a", v);
    Option<Vec<i64>> got = m.get("a");
    match (got) { Some(x) => { return x.get(0); } None => { return -1; } }
}
MYL
cat > "$CLO/libpkg/Seq2.myl" <<'MYL'
import { Vec } from "Vec.myl";
pub interface Seq2<T> { fn all() -> Vec<T>; }
class Src<T> {
    pub T v;
    pub fn init(T x) -> void { this.v = x; }
    pub fn all() -> Vec<T> {
        Vec<T> out = new Vec<T>();
        out.push(this.v);
        return out;
    }
}
pub fn viaIface() -> i64 {
    Seq2<i64> s = new Src<i64>(7);
    return s.all().get(0);
}
MYL
cat > "$CLO/libpkg/EHolder.myl" <<'MYL'
import { Vec } from "Vec.myl";
enum E<T> { One(T), Many(Vec<T>) }
pub fn enumVal() -> i64 {
    Vec<i64> v = new Vec<i64>();
    v.push(9);
    E<i64> e = E<i64>::Many(v);
    match (e) {
        E::One(x) => { return x; }
        E::Many(xs) => { return xs.get(0); }
    }
}
MYL
cat > "$CLO/libpkg/Deep.myl" <<'MYL'
import { Vec } from "Vec.myl";
import { Map } from "Map.myl";
pub class Deep<T> {
    pub Map<string, Vec<T>> m;
    pub fn init(T x) -> void {
        this.m = new Map<string, Vec<T>>();
        Vec<T> v = new Vec<T>();
        v.push(x);
        this.m.set("a", v);
    }
    pub fn deep() -> Option<Vec<Vec<T>>> {
        Option<Vec<T>> one = this.m.get("a");
        match (one) {
            Some(v) => {
                Vec<Vec<T>> out = new Vec<Vec<T>>();
                out.push(v);
                return Some(out);
            }
            None => { return None; }
        }
    }
}
MYL
cat > "$CLO/main.myl" <<'MYL'
import { println_i64 } from "Io.myl";
import { Vec } from "Vec.myl";
import { FieldBox } from "FieldBox";
import { sortedFirst } from "Search";
import { nestedMap } from "NestMap";
import { viaIface } from "Seq2";
import { enumVal } from "EHolder";
import { Deep } from "Deep";
fn main() -> i32 {
    FieldBox<i64> f = new FieldBox<i64>(3);
    println_i64(f.len());
    println_i64(f.first());
    println_i64(sortedFirst(5, 2));
    println_i64(nestedMap());
    println_i64(viaIface());
    println_i64(enumVal());
    Deep<i64> d = new Deep<i64>(5);
    Option<Vec<Vec<i64>>> deep = d.deep();
    match (deep) {
        Some(ds) => { println_i64(ds.len()); println_i64(ds.get(0).get(0)); }
        None => { println_i64(-1); }
    }
    return 0;
}
MYL
EXPECT_CLOSURE="$(printf '1\n3\n2\n11\n7\n9\n1\n5')"
closure_emit_ok=1
for mod in FieldBox Search NestMap Seq2 EHolder Deep; do
  if ! MYLIS_PATH="$ROOT:$ROOT/std/src:$CLO/libpkg" "$MYLC" "$CLO/libpkg/$mod.myl" --emit-mylib "$CLO/$mod.mylib" >"$OUT/generic_closure.$mod.emit.log" 2>&1; then
    echo "  FAIL generic_closure（生成 $mod.mylib 失败）"
    sed -n '1,6p' "$OUT/generic_closure.$mod.emit.log"; fail=$((fail + 1)); closure_emit_ok=0
    break
  fi
done
if [ "$closure_emit_ok" = 1 ]; then
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$CLO/libpkg" "$MYLC" "$CLO/main.myl" --backend mir --link-mylib "$CLO/FieldBox.mylib" --link-mylib "$CLO/Search.mylib" --link-mylib "$CLO/NestMap.mylib" --link-mylib "$CLO/Seq2.mylib" --link-mylib "$CLO/EHolder.mylib" --link-mylib "$CLO/Deep.mylib" -o "$CLO/app" >"$OUT/generic_closure.mir.log" 2>&1 \
      && "$CLO/app" >"$OUT/generic_closure.actual" 2>"$OUT/generic_closure.err" \
      && test "$(cat "$OUT/generic_closure.actual")" = "$EXPECT_CLOSURE"; then
    echo "  ok   generic_closure（多归档共享外部实例）"
    pass=$((pass + 1))
  else
    echo "  FAIL generic_closure（mir 运行）"
    sed -n '1,8p' "$OUT/generic_closure.mir.log"; sed -n '1,4p' "$OUT/generic_closure.err"; fail=$((fail + 1))
  fi
fi

# 同一个包内的兄弟模块既被单独打包，又作为依赖归档的子 API 段出现时，
# 不能把同一份 API 源码加载两次，否则同一泛型模板会实例化出两个同名具体类，
# 表现为三层嵌套返回最内层 0。
MULTI="$OUT/multinest"
rm -rf "$MULTI"
mkdir -p "$MULTI/libpkg"
cat > "$MULTI/libpkg/Wrap.myl" <<'MYL'
import { Vec } from "Vec.myl";
pub class Wrap<T> {
    pub T v;
    pub fn init(T x) -> void { this.v = x; }
    pub fn triple() -> Vec<Vec<Vec<T>>> {
        Vec<Vec<Vec<T>>> out = new Vec<Vec<Vec<T>>>();
        Vec<Vec<T>> mid = new Vec<Vec<T>>();
        Vec<T> inner = new Vec<T>();
        inner.push(this.v);
        mid.push(inner);
        out.push(mid);
        return out;
    }
    pub static fn Make(T x) -> Wrap<T> { return new Wrap<T>(x); }
}
MYL
cat > "$MULTI/libpkg/Multi.myl" <<'MYL'
import { Vec } from "Vec.myl";
import { Map } from "Map.myl";
import { Wrap } from "Wrap.myl";
pub fn v3(i64 x) -> Vec<Vec<Vec<i64>>> {
    Vec<Vec<Vec<i64>>> out = new Vec<Vec<Vec<i64>>>();
    Vec<Vec<i64>> mid = new Vec<Vec<i64>>();
    Vec<i64> inner = new Vec<i64>();
    inner.push(x);
    mid.push(inner);
    out.push(mid);
    return out;
}
pub fn mapDeep() -> Map<string, Vec<Vec<i64>>> {
    Map<string, Vec<Vec<i64>>> m = new Map<string, Vec<Vec<i64>>>();
    Vec<Vec<i64>> mid = new Vec<Vec<i64>>();
    Vec<i64> inner = new Vec<i64>();
    inner.push(8);
    mid.push(inner);
    m.set("a", mid);
    return m;
}
pub fn wrapBox() -> Vec<Wrap<Wrap<i64>>> {
    Vec<Wrap<Wrap<i64>>> out = new Vec<Wrap<Wrap<i64>>>();
    Wrap<Wrap<i64>> w = new Wrap<Wrap<i64>>(new Wrap<i64>(7));
    out.push(w);
    return out;
}
pub fn v5(i64 x) -> Vec<Vec<Vec<Vec<Vec<i64>>>>> {
    Vec<Vec<Vec<Vec<Vec<i64>>>>> out = new Vec<Vec<Vec<Vec<Vec<i64>>>>>();
    Vec<Vec<Vec<Vec<i64>>>> l4 = new Vec<Vec<Vec<Vec<i64>>>>();
    Vec<Vec<Vec<i64>>> l3 = new Vec<Vec<Vec<i64>>>();
    Vec<Vec<i64>> l2 = new Vec<Vec<i64>>();
    Vec<i64> l1 = new Vec<i64>();
    l1.push(x);
    l2.push(l1);
    l3.push(l2);
    l4.push(l3);
    out.push(l4);
    return out;
}
pub fn staticWrap(i64 x) -> i64 {
    Wrap<i64> w = Wrap<i64>::Make(x);
    return w.triple().get(0).get(0).get(0);
}
pub fn staticNested(i64 x) -> i64 {
    Wrap<Wrap<Wrap<i64>>> w = Wrap<Wrap<Wrap<i64>>>::Make(Wrap<Wrap<i64>>::Make(Wrap<i64>::Make(x)));
    return w.triple().get(0).get(0).get(0).v.v;
}
pub fn staticNested5(i64 x) -> i64 {
    Wrap<Wrap<Wrap<Wrap<Wrap<i64>>>>> w =
        Wrap<Wrap<Wrap<Wrap<Wrap<i64>>>>>::Make(
            Wrap<Wrap<Wrap<Wrap<i64>>>>::Make(
                Wrap<Wrap<Wrap<i64>>>::Make(
                    Wrap<Wrap<i64>>::Make(
                        Wrap<i64>::Make(x)))));
    return w.v.v.v.v.v;
}
MYL
cat > "$MULTI/main.myl" <<'MYL'
import { println_i64 } from "Io.myl";
import { Vec } from "Vec.myl";
import { Map } from "Map.myl";
import { Wrap } from "Wrap.myl";
import { v3, v5, staticWrap, staticNested, staticNested5, mapDeep, wrapBox } from "Multi";
fn main() -> i32 {
    Vec<Vec<Vec<i64>>> a = v3(1);
    println_i64(a.get(0).get(0).get(0));
    println_i64(a.get(0).get(0).len());
    println_i64(a.len());
    Vec<Vec<Vec<i64>>> b = new Wrap<i64>(2).triple();
    println_i64(b.get(0).get(0).get(0));
    Map<string, Vec<Vec<i64>>> m = mapDeep();
    Option<Vec<Vec<i64>>> got = m.get("a");
    match (got) { Some(v) => { println_i64(v.get(0).get(0)); } None => { println_i64(-1); } }
    Vec<Wrap<Wrap<i64>>> wb = wrapBox();
    println_i64(wb.get(0).v.v);
    Vec<Vec<Vec<Vec<Vec<i64>>>>> five = v5(1);
    println_i64(five.get(0).get(0).get(0).get(0).get(0));
    println_i64(five.get(0).get(0).get(0).get(0).len());
    println_i64(staticWrap(2));
    println_i64(staticNested(3));
    println_i64(staticNested5(6));
    Wrap<i64> w4 = Wrap<i64>::Make(4);
    println_i64(w4.triple().get(0).get(0).get(0));
    Wrap<Wrap<i64>> w5 = Wrap<Wrap<i64>>::Make(Wrap<i64>::Make(5));
    println_i64(w5.v.v);
    return 0;
}
MYL
EXPECT_MULTI="$(printf '1\n1\n1\n2\n8\n7\n1\n1\n2\n3\n6\n4\n5')"
multi_emit_ok=1
for mod in Wrap Multi; do
  if ! MYLIS_PATH="$ROOT:$ROOT/std/src:$MULTI/libpkg" "$MYLC" "$MULTI/libpkg/$mod.myl" --emit-mylib "$MULTI/$mod.mylib" >"$OUT/multinest.$mod.emit.log" 2>&1; then
    echo "  FAIL multinest（生成 $mod.mylib 失败）"
    sed -n '1,6p' "$OUT/multinest.$mod.emit.log"; fail=$((fail + 1)); multi_emit_ok=0
    break
  fi
done
if [ "$multi_emit_ok" = 1 ]; then
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$MULTI/libpkg" "$MYLC" "$MULTI/main.myl" --backend mir --link-mylib "$MULTI/Wrap.mylib" --link-mylib "$MULTI/Multi.mylib" -o "$MULTI/app" >"$OUT/multinest.mir.log" 2>&1 \
      && "$MULTI/app" >"$OUT/multinest.actual" 2>"$OUT/multinest.err" \
      && test "$(cat "$OUT/multinest.actual")" = "$EXPECT_MULTI"; then
    echo "  ok   multinest（五层嵌套 + 泛型静态方法 + 兄弟模块重复 API）"
    pass=$((pass + 1))
  else
    echo "  FAIL multinest（mir 运行）"
    sed -n '1,8p' "$OUT/multinest.mir.log"; sed -n '1,4p' "$OUT/multinest.err"; fail=$((fail + 1))
  fi
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$MULTI/libpkg" "$MYLC" "$MULTI/main.myl" --emit-llvm "$MULTI/app.ll" --link-mylib "$MULTI/Wrap.mylib" --link-mylib "$MULTI/Multi.mylib" >"$OUT/multinest.llvm.log" 2>&1 \
      && grep -q 'define ptr @myl_Wrap__Wrap__i64__Make' "$MULTI/app.ll" \
      && test "$(grep -c 'define ptr @myl_Wrap__Wrap__i64__Make' "$MULTI/app.ll")" = 1; then
    echo "  ok   multinest（LLVM 泛型静态方法定义唯一）"
    pass=$((pass + 1))
  else
    echo "  FAIL multinest（LLVM IR）"
    sed -n '1,8p' "$OUT/multinest.llvm.log"; fail=$((fail + 1))
  fi
fi

# 新增的 std 泛型容器也要能跨 .mylib 归档使用。
QMY="$OUT/std_queue"
rm -rf "$QMY"
mkdir -p "$QMY"
cat > "$QMY/QueueLib.myl" <<'MYL'
import { Queue } from "Queue.myl";
import { Stack } from "Stack.myl";
import { Deque } from "Deque.myl";
import { Set } from "Set.myl";

fn orNeg(Option<i64> value) -> i64 {
    match value {
        Some(v) => { return v; }
        None => { return -1; }
    }
}

pub fn queueSum(i64 a, i64 b) -> i64 {
    Queue<i64> q = new Queue<i64>();
    q.enqueue(a);
    q.enqueue(b);
    return orNeg(q.dequeue()) + orNeg(q.dequeue());
}

pub fn stackTop(i64 a, i64 b) -> i64 {
    Stack<i64> s = new Stack<i64>();
    s.push(a);
    s.push(b);
    return orNeg(s.pop());
}

pub fn dequeFront(i64 a, i64 b) -> i64 {
    Deque<i64> d = new Deque<i64>();
    d.pushBack(a);
    d.pushBack(b);
    return orNeg(d.popFront());
}

pub fn dequeBack(i64 a, i64 b) -> i64 {
    Deque<i64> d = new Deque<i64>();
    d.pushBack(a);
    d.pushBack(b);
    return orNeg(d.popBack());
}

pub fn setUnionLen(i64 a, i64 b) -> i64 {
    Set<i64> left = new Set<i64>();
    left.add(a);
    left.add(b);
    Set<i64> right = new Set<i64>();
    right.add(b);
    Set<i64> result = left.union(right);
    return result.len();
}
MYL
cat > "$QMY/main.myl" <<'MYL'
import { println_i64 } from "Io.myl";
import { queueSum, stackTop, dequeFront, dequeBack, setUnionLen } from "QueueLib";
fn main() -> i32 {
    println_i64(queueSum(3, 4));
    println_i64(stackTop(5, 6));
    println_i64(dequeFront(7, 8));
    println_i64(dequeBack(9, 10));
    println_i64(setUnionLen(1, 2));
    return 0;
}
MYL
EXPECT_QUEUE="$(printf '7\n6\n7\n10\n2')"
if ! MYLIS_PATH="$ROOT:$ROOT/std/src:$QMY" "$MYLC" "$QMY/QueueLib.myl" --emit-mylib "$QMY/QueueLib.mylib" >"$OUT/std_queue.emit.log" 2>&1; then
  echo "  FAIL std_queue（生成 .mylib 失败）"
  sed -n '1,6p' "$OUT/std_queue.emit.log"; fail=$((fail + 1))
else
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$QMY" "$MYLC" "$QMY/main.myl" --backend mir --link-mylib "$QMY/QueueLib.mylib" -o "$QMY/app" >"$OUT/std_queue.mir.log" 2>&1 \
      && "$QMY/app" >"$OUT/std_queue.actual" 2>"$OUT/std_queue.err" \
      && test "$(cat "$OUT/std_queue.actual")" = "$EXPECT_QUEUE"; then
    echo "  ok   std_queue（Queue/Stack/Deque/Set 跨归档）"
    pass=$((pass + 1))
  else
    echo "  FAIL std_queue（mir 运行）"
    sed -n '1,8p' "$OUT/std_queue.mir.log"; sed -n '1,4p' "$OUT/std_queue.err"; fail=$((fail + 1))
  fi
fi

# Eq bound 的泛型模板也要跨 .mylib 传播 import。
EQ="$OUT/eq_mylib"
rm -rf "$EQ"
mkdir -p "$EQ/libpkg" "$EQ/app/src"
cat > "$EQ/libpkg/EqLib.myl" <<'MYL'
import { Eq } from "core/lib";
import { Vec } from "Vec.myl";
pub fn has<T: Eq>(Vec<T> values, T value) -> bool {
    return values.get(0).equals(value);
}
MYL
cat > "$EQ/app/src/main.myl" <<'MYL'
import { Vec } from "Vec.myl";
import { println_bool } from "Io.myl";
import { has } from "EqLib";
fn main() -> i32 {
    Vec<i64> xs = new Vec<i64>();
    xs.push(7);
    println_bool(has<i64>(xs, 7));
    return 0;
}
MYL
printf 'true\n' > "$EQ/expected.txt"
if ! "$MYLC" "$EQ/libpkg/EqLib.myl" --emit-mylib "$EQ/lib.mylib" >"$OUT/eq_mylib.emit.log" 2>&1; then
  echo "  FAIL eq_mylib（生成 .mylib 失败）"
  sed -n '1,6p' "$OUT/eq_mylib.emit.log"; fail=$((fail + 1))
else
  if MYLIS_PATH="$ROOT:$ROOT/std/src:$EQ/libpkg" "$MYLC" "$EQ/app/src/main.myl" --backend mir --link-mylib "$EQ/lib.mylib" -o "$EQ/app/app" >"$OUT/eq_mylib.mir.log" 2>&1 \
      && "$EQ/app/app" >"$OUT/eq_mylib.actual" 2>"$OUT/eq_mylib.err" \
      && diff -u "$EQ/expected.txt" "$OUT/eq_mylib.actual" >"$OUT/eq_mylib.diff"; then
    echo "  ok   eq_mylib（Eq bound + MIR）"
    pass=$((pass + 1))
  else
    echo "  FAIL eq_mylib（MIR 运行）"
    sed -n '1,6p' "$OUT/eq_mylib.mir.log"; sed -n '1,4p' "$OUT/eq_mylib.err"; fail=$((fail + 1))
  fi
  if command -v clang >/dev/null 2>&1; then
    if MYLIS_PATH="$ROOT:$ROOT/std/src:$EQ/libpkg" "$MYLC" "$EQ/app/src/main.myl" --backend llvm --cc clang --link-mylib "$EQ/lib.mylib" -o "$EQ/app/app_llvm" >"$OUT/eq_mylib.llvm.log" 2>&1 \
        && "$EQ/app/app_llvm" >"$OUT/eq_mylib.llvm.actual" 2>"$OUT/eq_mylib.llvm.err" \
        && diff -u "$EQ/expected.txt" "$OUT/eq_mylib.llvm.actual" >"$OUT/eq_mylib.llvm.diff"; then
      echo "  ok   eq_mylib（Eq bound + LLVM）"
      pass=$((pass + 1))
    else
      echo "  FAIL eq_mylib（LLVM 运行）"
      sed -n '1,6p' "$OUT/eq_mylib.llvm.log"; sed -n '1,4p' "$OUT/eq_mylib.llvm.err"; fail=$((fail + 1))
    fi
  fi
fi


# 薄归档 + 传递依赖：B 只导出自己的 API/MIR，C 只链接 B 也能解析 B 的依赖 A。
THIN="$OUT/thin_mylib"
rm -rf "$THIN"
mkdir -p "$THIN/A/src" "$THIN/B/src" "$THIN/C/src"
cat > "$THIN/A/mylis.toml" <<'TOML'
[package]
name = "A"
version = "0.1.0"
[lib]
entry = "src/lib.myl"
TOML
cat > "$THIN/A/src/lib.myl" <<'MYL'
pub fn add(i64 a, i64 b) -> i64 { return a + b; }
pub fn twice(i64 a) -> i64 { return add(a, a); }
MYL
cat > "$THIN/B/mylis.toml" <<'TOML'
[package]
name = "B"
version = "0.1.0"
[lib]
entry = "src/lib.myl"
[dependencies]
A = { mylib = "../A.mylib" }
TOML
cat > "$THIN/B/src/lib.myl" <<'MYL'
import { add, twice } from "A/lib";
pub fn calc(i64 x) -> i64 { return add(twice(x), 1); }
MYL
cat > "$THIN/C/src/main.myl" <<'MYL'
import { println_i64 } from "Io.myl";
import { calc } from "B/lib";
fn main() -> i32 {
    println_i64(calc(3));
    return 0;
}
MYL
if ! "$MYLC" "$THIN/A/src/lib.myl" --emit-mylib "$THIN/A.mylib" >"$OUT/thin_mylib.a.log" 2>&1; then
  echo "  FAIL thin_mylib（生成 A 失败）"
  sed -n '1,6p' "$OUT/thin_mylib.a.log"; fail=$((fail + 1))
elif ! "$MYLC" "$THIN/B/src/lib.myl" --emit-mylib "$THIN/B/lib.mylib" --update-lock >"$OUT/thin_mylib.b.log" 2>&1; then
  echo "  FAIL thin_mylib（生成 B 失败）"
  sed -n '1,6p' "$OUT/thin_mylib.b.log"; fail=$((fail + 1))
elif grep -q '^pub fn add' "$THIN/B/lib.mylib"; then
  echo "  FAIL thin_mylib（B 归档不是薄归档，包含依赖 A 的 API）"
  fail=$((fail + 1))
elif ! MYLIS_PATH="$ROOT:$ROOT/std/src" "$MYLC" "$THIN/C/src/main.myl" --backend mir --link-mylib "$THIN/B/lib.mylib" -o "$THIN/C/app" >"$OUT/thin_mylib.c.log" 2>&1; then
  echo "  FAIL thin_mylib（C 只链接 B 编译失败，传递依赖未解析）"
  sed -n '1,8p' "$OUT/thin_mylib.c.log"; fail=$((fail + 1))
elif ! "$THIN/C/app" >"$OUT/thin_mylib.actual" 2>"$OUT/thin_mylib.err"; then
  echo "  FAIL thin_mylib（运行失败）"
  sed -n '1,4p' "$OUT/thin_mylib.err"; fail=$((fail + 1))
elif ! diff -u <(printf '7\n') "$OUT/thin_mylib.actual" >"$OUT/thin_mylib.diff"; then
  echo "  FAIL thin_mylib（输出不符）"
  cat "$OUT/thin_mylib.diff"; fail=$((fail + 1))
else
  echo "  ok   thin_mylib（薄归档 + 传递依赖）"
  pass=$((pass + 1))
fi

echo
echo "包级通过：$pass    失败：$fail"
[ "$fail" -eq 0 ]

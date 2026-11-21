#!/usr/bin/env bash
# 标准库命名约定检查（core/ 与 std/src/）：
#   类名、静态方法、枚举变体：大驼峰
#   非类内函数：小写+下划线
#   实例方法、接口方法、变量/成员变量/静态变量：小驼峰
#   常量：全大写+下划线
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fail=0

check_file() {
  local f="$1"
  local base
  base="$(basename "$f")"
  while IFS= read -r line; do
    local name="${line##*class }"
    name="${name%% *}"
    name="${name%%<*}"
    if [[ ! "$name" =~ ^[A-Z][A-Za-z0-9]*$ ]]; then
      printf '命名违规：%s 类名 %s 必须大驼峰\n' "$base" "$name"
      fail=1
    fi
  done < <(grep -hE '^[[:space:]]*(pub )?class ' "$f" 2>/dev/null || true)

  while IFS= read -r line; do
    local name="${line##*static fn }"
    name="${name%%(*}"
    name="${name%%<*}"
    if [[ ! "$name" =~ ^[A-Z][A-Za-z0-9]*$ ]]; then
      printf '命名违规：%s 静态方法 %s 必须大驼峰\n' "$base" "$name"
      fail=1
    fi
  done < <(grep -hE '^[[:space:]]*(pub )?static fn ' "$f" 2>/dev/null || true)

  # 类/接口实例方法：小驼峰。只匹配类体缩进 4 空格，避免误伤函数体内的局部声明。
  while IFS= read -r line; do
    local name="${line##*fn }"
    name="${name%%(*}"
    name="${name%%<*}"
    if [[ ! "$name" =~ ^[a-z][A-Za-z0-9]*$ ]]; then
      printf '命名违规：%s 实例/接口方法 %s 必须小驼峰\n' "$base" "$name"
      fail=1
    fi
  done < <(grep -hE '^    (pub )?fn ' "$f" 2>/dev/null || true)

  while IFS= read -r line; do
    local name="${line##*fn }"
    name="${name%%(*}"
    name="${name%%<*}"
    if [[ ! "$name" =~ ^[a-z][a-z0-9_]*$ ]] && [[ "$name" != "init" ]]; then
      printf '命名违规：%s 自由函数 %s 必须小写+下划线\n' "$base" "$name"
      fail=1
    fi
  done < <(grep -hE '^(pub )?fn ' "$f" 2>/dev/null || true)

  # 类成员：字段/静态变量小驼峰，常量全大写+下划线。
  while IFS= read -r line; do
    local name="${line%%=*}"
    name="${name%;*}"
    name="${name##* }"
    if [[ "$name" == "" ]]; then
      continue
    fi
    if [[ "$line" == *"const "* ]]; then
      if [[ ! "$name" =~ ^[A-Z][A-Z0-9_]*$ ]]; then
        printf '命名违规：%s 常量 %s 必须全大写+下划线\n' "$base" "$name"
        fail=1
      fi
    elif [[ ! "$name" =~ ^[a-z][A-Za-z0-9]*$ ]]; then
      printf '命名违规：%s 成员/静态变量 %s 必须小驼峰\n' "$base" "$name"
      fail=1
    fi
  done < <(awk '
    /^[[:space:]]*(pub )?class[[:space:]]/ { in_class=1; next }
    in_class && /^}/ { in_class=0; next }
    in_class && /^    [^ ]/ {
      line=$0
      if (line !~ /;/) next
      if (line ~ /^    (pub )?(static )?fn /) next
      if (line ~ /^    \/\/ /) next
      if (line ~ /^    (pub )?(class|interface|enum) /) next
      if (line ~ /^    (pub )?import /) next
      print line
    }
  ' "$f")

  # 顶层常量：全大写+下划线。
  while IFS= read -r line; do
    local name="${line%%=*}"
    name="${name%;*}"
    name="${name##* }"
    if [[ ! "$name" =~ ^[A-Z][A-Z0-9_]*$ ]]; then
      printf '命名违规：%s 顶层常量 %s 必须全大写+下划线\n' "$base" "$name"
      fail=1
    fi
  done < <(grep -hE '^(pub )?const ' "$f" 2>/dev/null || true)
}

for f in "$ROOT"/core/*.myl "$ROOT"/std/src/*.myl; do
  [ -f "$f" ] || continue
  check_file "$f"
done

if [ "$fail" -eq 0 ]; then
  echo "命名约定检查通过"
fi
exit "$fail"

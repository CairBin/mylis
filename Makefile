# ============================================================================
# Mylis 构建入口
#
# 默认策略：
#   * Mylis 代码默认后端：LLVM（mylc 编译 .myl 时）
#   * 运行时默认 GC：自研 GC（own）
#   * Windows 构建 mylc/myls/运行时：MSVC（cl.exe + lib.exe，Rust 目标 msvc）
#   * Linux/macOS 构建：GCC（cc/ar）
#
# 常用：
#   make                      构建 mylc/myls + 运行时 + 自研 GC，收进 bin/
#   make test                 全部回归
#   make test-win             Windows/MSVC 冒烟测试（需在 Windows 上执行）
#   make GC_BACKEND=simple    改用 simple GC
#   make GC_BACKEND=boehm     改用 Boehm GC（需先 make bootstrap）
#   make RT_IMPL=/path/rt     使用目录下的 runtime.rs 作为运行时实现
#   make RT_NO_STD=1          使用 no_std Rust 运行时（POSIX/Win32 平台薄层）
#   make PLATFORM_IMPL=/path/to/platform.rs  第三方平台实现（裸机）
#   make PLATFORM_IMPL=/path/to/dir           读取该目录下的 platform.rs
#   make RT_NO_BACKEND_GC=1                   裸机运行时去掉默认 BackendGc
#   make CC=clang             覆盖 C 编译器（Linux/macOS）
#   make dist                 生成 Windows 发行包（需在 Windows + MSVC 环境执行）
#
# 生成物默认后端可用 `mylc --backend mir|ast` 覆盖；工程也可在 mylis.toml
# 的 [build] backend 指定。工具链在 [toolchain] 指定。
# ============================================================================
ROOT       := $(CURDIR)
# 标准库已与编译器解耦：测试/示例显式把 std 放入模块路径。
MYLIS_PATH ?= $(ROOT):$(ROOT)/std/src
export MYLIS_PATH
GC_BACKEND ?= own
BACKEND    ?= llvm
# 运行时实现：own / boehm / simple，或任意 .rs 实现文件路径
RT_IMPL    ?= own
RT_NO_STD  ?=
RT_NO_BACKEND_GC ?=
RT_IMPL_DIR ?= $(ROOT)/mylrt-rs/src/impls
MYLGC_SRC   := $(wildcard mylgc/src/*.rs) $(wildcard mylgc/src/gc/*.rs)
MYLRT_SRC   := $(wildcard mylrt-rs/src/*.rs) $(wildcard mylrt-rs/src/impls/*.rs) $(wildcard mylrt-rs/src/platforms/*.rs)
PLATFORM_IMPL ?= host
PLATFORM_IMPL_DIR ?= $(ROOT)/mylrt-rs/src/platforms
PLATFORM_IMPL_RS ?= $(if $(wildcard $(PLATFORM_IMPL_DIR)/$(PLATFORM_IMPL).rs),$(PLATFORM_IMPL_DIR)/$(PLATFORM_IMPL).rs,$(if $(wildcard $(PLATFORM_IMPL)/platform.rs),$(PLATFORM_IMPL)/platform.rs,$(PLATFORM_IMPL)))
# Windows cmd.exe 下不要用 abspath 处理已经是盘符绝对路径的值，否则会变成 C:/x/C:/x/...
RT_IMPL_RS_WIN = $(if $(findstring :,$(RT_IMPL_RS)),$(RT_IMPL_RS),$(abspath $(RT_IMPL_RS)))
PLATFORM_IMPL_RS_WIN = $(if $(findstring :,$(PLATFORM_IMPL_RS)),$(PLATFORM_IMPL_RS),$(abspath $(PLATFORM_IMPL_RS)))
# RT_IMPL 可以是名字（在 RT_IMPL_DIR 下找 <name>.rs）、.rs 文件，或包含
# runtime.rs 的实现目录。
RT_IMPL_RS  ?= $(if $(wildcard $(RT_IMPL_DIR)/$(RT_IMPL).rs),$(RT_IMPL_DIR)/$(RT_IMPL).rs,$(if $(wildcard $(RT_IMPL)/runtime.rs),$(RT_IMPL)/runtime.rs,$(RT_IMPL)))
CARGO      ?= cargo
RUST_FEATURES := $(if $(RT_NO_STD),--no-default-features,) $(if $(RT_NO_BACKEND_GC),--features no-backend-gc,)
EXE        :=
OBJ        := .o

ifeq ($(OS),Windows_NT)
  EXE          := .exe
  TOOLCHAIN_FAMILY ?= msvc
  SHELL        := cmd.exe
  .SHELLFLAGS  := /c
  W             = $(subst /,\,$1)
  MKDIR         = if not exist "$(call W,$1)" mkdir "$(call W,$1)"
  RMDIR         = if exist "$(call W,$1)" rmdir /S /Q "$(call W,$1)"
  COPY          = copy /Y "$(call W,$1)" "$(call W,$2)" >NUL
  COPYTREE      = xcopy /E /I /Y /Q "$(call W,$1)\*" "$(call W,$2)" >NUL
  BIN_MYLC      := build/mylc$(EXE)
  BIN_MYLS      := build/myls$(EXE)
  MANIFEST_OS   := windows
  MANIFEST_FAMILY := $(TOOLCHAIN_FAMILY)
  MANIFEST_MYRT = $(MYRT_LIB)
  MANIFEST_MYLGC = $(MYLGC_LIB)
  MANIFEST_SIMPLE = $(SIMPLE_LIB)
  STD_WIN_DIR := build/std-win32/native
  STD_WIN_STAMP := $(STD_WIN_DIR)/.built
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
    OBJ          := .o
    CARGO_TARGET ?= x86_64-pc-windows-gnu
    CC           := gcc
    AR           := ar
    RUSTFLAGS_T  = --target $(CARGO_TARGET)
    RUST_OUT     = target/$(CARGO_TARGET)/release
    MYRT_LIB     := build/libmylrt.a
    SIMPLE_LIB   := build/libgc_simple.a
    BOEHM_LIB    := build/libgc_boehm.a
    MYLGC_LIB    := build/libmylgc.a
    GC_DEPS      := $(MYLGC_LIB) $(SIMPLE_LIB)
    CFLAGS       := -std=c11 -O2 -Wall -Wextra -fno-strict-aliasing -I$(ROOT)/myrt
    LIBFLAGS     :=
    MANIFEST_STYLE := gnu
    MANIFEST_ABI := gnu
    MANIFEST_TARGET := $(CARGO_TARGET)
    MANIFEST_CC  := $(notdir $(CC))
    MANIFEST_AR  := $(notdir $(AR))
    # Rust std（windows-gnu）静态库引用的系统库；链接时加 -l 前缀。
    WIN_RT_LIBS  := ws2_32 ntdll userenv advapi32 bcrypt ole32 oleaut32 shell32
    MANIFEST_SYS_LIBS = $(if $(RT_NO_STD),,$(WIN_RT_LIBS))
    STD_WIN_LIBS := $(addprefix $(STD_WIN_DIR)/,libmylis_file.a libmylis_io.a libmylis_time.a libmylis_net.a libmylis_event.a libmylis_process.a)
  else
    OBJ          := .obj
    CARGO_TARGET ?= x86_64-pc-windows-msvc
    # Windows 分支强制 MSVC 默认；命令行的 CC=clang-cl 仍可覆盖。
    # 不能用 ?=，否则 Git Bash/终端继承的 CC=cc 会覆盖默认值。
    CC           := cl
    AR           := lib
    RUSTFLAGS_T  = --target $(CARGO_TARGET)
    RUST_OUT     = target/$(CARGO_TARGET)/release
    MYRT_LIB     := build/mylrt.lib
    SIMPLE_LIB   := build/gc_simple.lib
    BOEHM_LIB    := build/gc_boehm.lib
    MYLGC_LIB    := build/mylgc.lib
    GC_DEPS      := $(MYLGC_LIB) $(SIMPLE_LIB)
    CFLAGS       := /nologo /std:c11 /O2 /I$(ROOT)/myrt
    LIBFLAGS     := /nologo
    MANIFEST_STYLE := msvc
    MANIFEST_ABI := msvc
    MANIFEST_TARGET := $(CARGO_TARGET)
    MANIFEST_CC  := cl
    MANIFEST_AR  := lib
    # Rust std（windows-msvc）静态库引用的系统库；cl / clang 都需要显式带上。
    WIN_RT_LIBS  := ws2_32.lib ntdll.lib userenv.lib advapi32.lib bcrypt.lib ole32.lib oleaut32.lib shell32.lib
    # RT_NO_STD=1 表示 freestanding 运行时，不额外链 Windows 系统库。
    MANIFEST_SYS_LIBS = $(if $(RT_NO_STD),,$(WIN_RT_LIBS))
    STD_WIN_LIBS := $(addprefix $(STD_WIN_DIR)/,mylis_file.lib mylis_io.lib mylis_time.lib mylis_net.lib mylis_event.lib mylis_process.lib)
  endif
else
  CC           ?= gcc
  AR           ?= ar
  CARGO_TARGET ?=
  RUSTFLAGS_T  =
  RUST_OUT     = target/release
  MKDIR         = mkdir -p "$1"
  RMDIR         = rm -rf "$1"
  COPY          = cp "$1" "$2"
  COPYTREE      = cp -r "$1/." "$2/"
  MYRT_LIB      := build/libmylrt.a
  SIMPLE_LIB    := build/libgc_simple.a
  BOEHM_LIB     := build/libgc_boehm.a
  STD_POSIX_DIR := build/std-posix/native
  STD_POSIX_LIBS := $(addprefix $(STD_POSIX_DIR)/,libmylis_file.a libmylis_io.a libmylis_time.a libmylis_net.a libmylis_event.a libmylis_process.a)
  STD_POSIX_STAMP := $(STD_POSIX_DIR)/.built
  MYLGC_LIB     := build/libmylgc.a
  GC_DEPS       := $(MYLGC_LIB) $(SIMPLE_LIB) $(BOEHM_LIB)
  CFLAGS        := -std=c11 -O2 -Wall -Wextra -fno-strict-aliasing -fPIC -I$(ROOT)/myrt
  LIBFLAGS      :=
  BIN_MYLC      := build/mylc$(EXE)
  BIN_MYLS      := build/myls$(EXE)
  TOOLCHAIN_FAMILY ?= gcc
  MANIFEST_OS   := $(shell uname -s 2>/dev/null | tr '[:upper:]' '[:lower:]' | sed 's/darwin/macos/')
  MANIFEST_FAMILY := $(TOOLCHAIN_FAMILY)
  MANIFEST_STYLE := gnu
  MANIFEST_ABI  := gnu
  MANIFEST_TARGET := $(shell rustc -vV 2>/dev/null | sed -n 's/^host: //p')
  MANIFEST_CC   := $(notdir $(CC))
  MANIFEST_AR   := $(notdir $(AR))
  MANIFEST_MYRT = $(MYRT_LIB)
  MANIFEST_MYLGC = $(MYLGC_LIB)
  MANIFEST_SIMPLE = $(SIMPLE_LIB)
  MANIFEST_SYS_LIBS :=
endif

GC_ROOT ?= $(HOME)/mylis-toolchain
BUILD_MANIFEST := build/mylis-build.json

ifeq ($(OS),Windows_NT)
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
    ifeq ($(findstring -gnu,$(CARGO_TARGET)),)
      $(error MinGW build requires CARGO_TARGET=x86_64-pc-windows-gnu; current: $(CARGO_TARGET))
    endif
    CC_STEM := $(basename $(notdir $(CC)))
    AR_STEM := $(basename $(notdir $(AR)))
    ifeq ($(filter gcc clang,$(CC_STEM)),)
      $(error MinGW build requires CC=gcc or clang; current CC=$(CC))
    endif
    ifeq ($(filter ar llvm-ar,$(AR_STEM)),)
      $(error MinGW build requires AR=ar or llvm-ar; current AR=$(AR))
    endif
  else ifneq ($(TOOLCHAIN_FAMILY),msvc)
    $(error Windows supports TOOLCHAIN_FAMILY=msvc or mingw; current: $(TOOLCHAIN_FAMILY))
  else
    ifeq ($(findstring -msvc,$(CARGO_TARGET)),)
      $(error Windows MSVC build requires CARGO_TARGET=x86_64-pc-windows-msvc; current: $(CARGO_TARGET))
    endif
    CC_STEM := $(basename $(notdir $(CC)))
    AR_STEM := $(basename $(notdir $(AR)))
    ifneq ($(filter cl clang-cl,$(CC_STEM)),$(CC_STEM))
      $(error Windows MSVC build requires CC=cl or clang-cl; current CC=$(CC); run: set CC=cl)
    endif
    ifneq ($(filter lib,$(AR_STEM)),$(AR_STEM))
      $(error Windows MSVC build requires AR=lib; current AR=$(AR); run: set AR=lib)
    endif
  endif
endif

.PHONY: all bin mylc myls myrt test test-win ci release-check bootstrap clean dist check-toolchain check-rt std-native std-native-win FORCE

all: check-toolchain bin

bin: $(BIN_MYLC) $(BIN_MYLS) $(MYRT_LIB) $(GC_DEPS) $(BUILD_MANIFEST) $(if $(filter Windows_NT,$(OS)),$(STD_WIN_STAMP),$(STD_POSIX_STAMP))
	@$(call RMDIR,bin)
	@$(call MKDIR,bin)
	@$(call MKDIR,bin/myrt)
	@$(call MKDIR,bin/core)
	@$(call MKDIR,bin/std)
	@$(call MKDIR,bin/build)
ifeq ($(OS),Windows_NT)
else
	@$(call MKDIR,bin/std-posix)
	@$(call MKDIR,bin/build/std-posix/native)
endif
	@$(call MKDIR,bin/gc)
	@$(call MKDIR,bin/gc/include)
	@$(call MKDIR,bin/gc/lib)
	@$(call MKDIR,bin/toolchains)
	@$(call COPY,$(BIN_MYLC),bin/mylc$(EXE))
	@$(call COPY,$(BIN_MYLS),bin/myls$(EXE))
	@$(call COPYTREE,myrt,bin/myrt)
	@$(call COPYTREE,core,bin/core)
	@$(call COPYTREE,std,bin/std)
ifeq ($(OS),Windows_NT)
	@$(call MKDIR,bin/std-win32)
	@$(call COPYTREE,std-win32,bin/std-win32)
	@$(call MKDIR,bin/build/std-win32/native)
ifeq ($(TOOLCHAIN_FAMILY),mingw)
	@$(call COPY,build/std-win32/native/libmylis_file.a,bin/build/std-win32/native/libmylis_file.a)
	@$(call COPY,build/std-win32/native/libmylis_io.a,bin/build/std-win32/native/libmylis_io.a)
	@$(call COPY,build/std-win32/native/libmylis_time.a,bin/build/std-win32/native/libmylis_time.a)
	@$(call COPY,build/std-win32/native/libmylis_net.a,bin/build/std-win32/native/libmylis_net.a)
	@$(call COPY,build/std-win32/native/libmylis_event.a,bin/build/std-win32/native/libmylis_event.a)
	@$(call COPY,build/std-win32/native/libmylis_process.a,bin/build/std-win32/native/libmylis_process.a)
else
	@$(call COPY,build/std-win32/native/mylis_file.lib,bin/build/std-win32/native/mylis_file.lib)
	@$(call COPY,build/std-win32/native/mylis_io.lib,bin/build/std-win32/native/mylis_io.lib)
	@$(call COPY,build/std-win32/native/mylis_time.lib,bin/build/std-win32/native/mylis_time.lib)
	@$(call COPY,build/std-win32/native/mylis_net.lib,bin/build/std-win32/native/mylis_net.lib)
	@$(call COPY,build/std-win32/native/mylis_event.lib,bin/build/std-win32/native/mylis_event.lib)
	@$(call COPY,build/std-win32/native/mylis_process.lib,bin/build/std-win32/native/mylis_process.lib)
endif
endif
	@$(call COPY,$(MYRT_LIB),bin/build/$(notdir $(MYRT_LIB)))
	@$(call COPY,$(MYLGC_LIB),bin/build/$(notdir $(MYLGC_LIB)))
	@$(call COPY,$(SIMPLE_LIB),bin/build/$(notdir $(SIMPLE_LIB)))
ifeq ($(OS),Windows_NT)
else
	@$(call COPYTREE,std-posix,bin/std-posix)
	@$(call COPY,$(STD_POSIX_DIR)/libmylis_file.a,bin/$(STD_POSIX_DIR)/libmylis_file.a)
	@$(call COPY,$(STD_POSIX_DIR)/libmylis_io.a,bin/$(STD_POSIX_DIR)/libmylis_io.a)
	@$(call COPY,$(STD_POSIX_DIR)/libmylis_time.a,bin/$(STD_POSIX_DIR)/libmylis_time.a)
	@$(call COPY,$(STD_POSIX_DIR)/libmylis_net.a,bin/$(STD_POSIX_DIR)/libmylis_net.a)
	@$(call COPY,$(STD_POSIX_DIR)/libmylis_event.a,bin/$(STD_POSIX_DIR)/libmylis_event.a)
	@$(call COPY,$(STD_POSIX_DIR)/libmylis_process.a,bin/$(STD_POSIX_DIR)/libmylis_process.a)
endif
	@$(call COPY,$(BUILD_MANIFEST),bin/mylis-build.json)
ifeq ($(OS),Windows_NT)
else
	@$(call COPY,$(BOEHM_LIB),bin/build/$(notdir $(BOEHM_LIB)))
endif
ifeq ($(OS),Windows_NT)
	@if exist "$(call W,$(GC_ROOT)/include/gc.h)" ( xcopy /E /I /Y /Q "$(call W,$(GC_ROOT)/include)\*" "$(call W,bin/gc/include)" >NUL & copy /Y "$(call W,$(GC_ROOT)/lib/libgc.lib)" "$(call W,bin/gc/lib)" >NUL ) else ( echo warning: no MSVC Boehm GC found; own GC is default )
else
	@if [ -f "$(GC_ROOT)/include/gc.h" ]; then cp -r "$(GC_ROOT)/include/." bin/gc/include/ && cp "$(GC_ROOT)/lib/libgc.a" bin/gc/lib/; else echo "warning: no Boehm GC found; own GC is default"; fi
endif
	@echo "bin/ ready (backend=$(BACKEND), gc=$(GC_BACKEND), rt=$(RT_IMPL))"

build:
	@$(call MKDIR,build)

# 构建清单：声明本套 build/ 产物的 ABI、target 与库路径。
ifeq ($(OS),Windows_NT)
$(BUILD_MANIFEST): FORCE | build
	@powershell -NoProfile -ExecutionPolicy Bypass -File tools\write_manifest.ps1 -Out "$(call W,$@)" -Os "$(MANIFEST_OS)" -Family "$(MANIFEST_FAMILY)" -Style "$(MANIFEST_STYLE)" -Abi "$(MANIFEST_ABI)" -Target "$(MANIFEST_TARGET)" -Cc "$(MANIFEST_CC)" -Ar "$(MANIFEST_AR)" -Myrt "$(MANIFEST_MYRT)" -Mylgc "$(MANIFEST_MYLGC)" -Simple "$(MANIFEST_SIMPLE)" -SystemLibs "$(MANIFEST_SYS_LIBS)"
else
$(BUILD_MANIFEST): FORCE | build
	@sh tools/write_manifest.sh "$@" "$(MANIFEST_OS)" "$(MANIFEST_FAMILY)" "$(MANIFEST_STYLE)" "$(MANIFEST_ABI)" "$(MANIFEST_TARGET)" "$(MANIFEST_CC)" "$(MANIFEST_AR)" "$(MANIFEST_MYRT)" "$(MANIFEST_MYLGC)" "$(MANIFEST_SIMPLE)" "$(MANIFEST_SYS_LIBS)"
endif

# ---- 运行时（Rust 实现，C ABI 不变）----
ifeq ($(OS),Windows_NT)
$(MYRT_LIB): $(MYLRT_SRC) mylrt-rs/build.rs mylrt-rs/Cargo.toml $(RT_IMPL_RS) $(PLATFORM_IMPL_RS) FORCE | build
	cd mylrt-rs && set "MYLRT_IMPL_RS=$(RT_IMPL_RS_WIN)" && set "MYLRT_PLATFORM_RS=$(PLATFORM_IMPL_RS_WIN)" && $(CARGO) build --release $(RUSTFLAGS_T) $(RUST_FEATURES)
ifeq ($(TOOLCHAIN_FAMILY),mingw)
	@$(call COPY,mylrt-rs/$(RUST_OUT)/libmylrt.a,$@)
else
	@$(call COPY,mylrt-rs/$(RUST_OUT)/mylrt.lib,$@)
endif
else
$(MYRT_LIB): $(MYLRT_SRC) mylrt-rs/build.rs mylrt-rs/Cargo.toml $(RT_IMPL_RS) $(PLATFORM_IMPL_RS) FORCE | build
	cd mylrt-rs && MYLRT_IMPL_RS="$(abspath $(RT_IMPL_RS))" MYLRT_PLATFORM_RS="$(abspath $(PLATFORM_IMPL_RS))" $(CARGO) build --release $(RUSTFLAGS_T) $(RUST_FEATURES)
	@$(call COPY,mylrt-rs/$(RUST_OUT)/libmylrt.a,$@)
endif

# 可选：保留 C 运行时作为对照实现
build/libmylrt_c.a: myrt/mylrt.c myrt/mylrt.h myrt/mylrt_gc.h | build
	$(CC) $(CFLAGS) -c myrt/mylrt.c -o build/mylrt_c.o
	-$(call DEL,$@)
	$(AR) rcs $@ build/mylrt_c.o

# ---- 可选 GC 后端（Boehm / simple）----
ifeq ($(OS),Windows_NT)
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
build/gc_simple.o: myrt/gc/gc_simple.c myrt/mylrt_gc.h | build
	$(CC) $(CFLAGS) -c $< -o $@

build/gc_boehm.o: myrt/gc/gc_boehm.c myrt/mylrt_gc.h | build
	$(CC) $(CFLAGS) -I$(GC_ROOT)/include -c $< -o $@

$(SIMPLE_LIB): build/gc_simple.o
	$(AR) rcs $@ build/gc_simple.o

$(BOEHM_LIB): build/gc_boehm.o
	$(AR) rcs $@ build/gc_boehm.o
  else
build/gc_simple.obj: myrt/gc/gc_simple.c myrt/mylrt_gc.h | build
	$(CC) $(CFLAGS) /c myrt/gc/gc_simple.c /Fo:build/gc_simple.obj

build/gc_boehm.obj: myrt/gc/gc_boehm.c myrt/mylrt_gc.h | build
	$(CC) $(CFLAGS) /I"$(GC_ROOT)/include" /c myrt/gc/gc_boehm.c /Fo:build/gc_boehm.obj

$(SIMPLE_LIB): build/gc_simple.obj
	$(AR) $(LIBFLAGS) /OUT:$@ build/gc_simple.obj

$(BOEHM_LIB): build/gc_boehm.obj
	$(AR) $(LIBFLAGS) /OUT:$@ build/gc_boehm.obj
  endif
else
build/gc_simple.o: myrt/gc/gc_simple.c myrt/mylrt_gc.h | build
	$(CC) $(CFLAGS) -c $< -o $@

build/gc_boehm.o: myrt/gc/gc_boehm.c myrt/mylrt_gc.h | build
	$(CC) $(CFLAGS) -I$(GC_ROOT)/include -c $< -o $@

$(SIMPLE_LIB): build/gc_simple.o
	-rm -f $@
	$(AR) rcs $@ build/gc_simple.o

$(BOEHM_LIB): build/gc_boehm.o
	-rm -f $@
	$(AR) rcs $@ build/gc_boehm.o
endif

# ---- 自研 GC（Rust staticlib）----
ifeq ($(OS),Windows_NT)
$(MYLGC_LIB): $(MYLGC_SRC) mylgc/Cargo.toml | build
	cd mylgc && set "RUSTFLAGS=$(RUSTFLAGS) -C relocation-model=pic" && $(CARGO) build --release $(RUSTFLAGS_T)
ifeq ($(TOOLCHAIN_FAMILY),mingw)
	@$(call COPY,mylgc/$(RUST_OUT)/libmylgc.a,$@)
else
	@$(call COPY,mylgc/$(RUST_OUT)/mylgc.lib,$@)
endif
else
$(MYLGC_LIB): $(MYLGC_SRC) mylgc/Cargo.toml | build
	cd mylgc && RUSTFLAGS="$(RUSTFLAGS) -C relocation-model=pic" $(CARGO) build --release $(RUSTFLAGS_T)
	@$(call COPY,mylgc/$(RUST_OUT)/libmylgc.a,$@)
endif

# ---- 编译器 ----
$(BIN_MYLC): $(wildcard mylc/src/*.rs) $(wildcard mylc/src/*/*.rs) mylc/Cargo.toml | build
	cd mylc && $(CARGO) build --release $(RUSTFLAGS_T)
	@$(call COPY,mylc/$(RUST_OUT)/mylc$(EXE),$@)

$(BIN_MYLS): $(wildcard mylc/src/*.rs) $(wildcard mylc/src/*/*.rs) mylc/Cargo.toml | build
	cd mylc && $(CARGO) build --release $(RUSTFLAGS_T)
	@$(call COPY,mylc/$(RUST_OUT)/myls$(EXE),$@)

# ---- Windows std native 侧车 ----
# PowerShell 5.1 在 rustc 子进程结束后可能不自动退出；用 -Command 显式收尾，并保留失败退出码。
ifeq ($(OS),Windows_NT)
$(STD_WIN_STAMP): $(wildcard std-win32/native/*.rs) $(wildcard std-win32/native/*.h) $(wildcard std-posix/native/*.rs) tools/build_std_native.ps1 | build
	@powershell -NoProfile -ExecutionPolicy Bypass -Command "& { try { & 'tools/build_std_native.ps1' -Target '$(CARGO_TARGET)' -Out '$(subst /,\,$(STD_WIN_DIR))' } catch { Write-Error $$_ ; exit 1 }; exit 0 }"
	@echo built > "$(call W,$@)"

std-native-win: $(STD_WIN_STAMP)
std-native: $(STD_WIN_STAMP)
else
$(STD_POSIX_STAMP): $(wildcard std-posix/native/*.rs) $(wildcard std-posix/native/*.h) tools/build_std_native.sh | build
	@bash tools/build_std_native.sh
	@echo built > "$@"

std-native: $(STD_POSIX_STAMP)
std-native-win:
	@echo "std-native-win 需要在 Windows + MSVC 环境执行"
endif

check-toolchain:
ifeq ($(OS),Windows_NT)
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
	@where $(CC) >NUL 2>NUL || (echo check-toolchain: 找不到 $(CC)，请把 MinGW bin 加入 PATH & exit /b 1)
	@where $(AR) >NUL 2>NUL || (echo check-toolchain: 找不到 $(AR) & exit /b 1)
	@$(CARGO) --version >NUL 2>NUL || (echo check-toolchain: 找不到 cargo & exit /b 1)
  else
	@where cl >NUL 2>NUL || (echo check-toolchain: 找不到 cl.exe，请先进入 VS 开发者环境或运行 vcvars64.bat & exit /b 1)
	@where lib >NUL 2>NUL || (echo check-toolchain: 找不到 lib.exe & exit /b 1)
	@$(CARGO) --version >NUL 2>NUL || (echo check-toolchain: 找不到 cargo & exit /b 1)
  endif
else
	@command -v $(CC) >/dev/null 2>&1 || (echo "check-toolchain: 找不到 $(CC)" && exit 1)
	@command -v $(AR) >/dev/null 2>&1 || (echo "check-toolchain: 找不到 $(AR)" && exit 1)
	@$(CARGO) --version >/dev/null 2>&1 || (echo "check-toolchain: 找不到 cargo" && exit 1)
endif

# 链接探针：用当前 CC 同时链接 myrt + mylgc，提前暴露 ABI/CRT 不一致。
check-rt: $(MYRT_LIB) $(MYLGC_LIB) $(BUILD_MANIFEST)
ifeq ($(OS),Windows_NT)
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
	@$(CC) $(CFLAGS) -o build/rt_probe.exe tools/rt_probe.c $(MYRT_LIB) $(MYLGC_LIB) $(addprefix -l,$(WIN_RT_LIBS))
	@build\rt_probe.exe
  else
	@$(CC) $(CFLAGS) /Fe:build\rt_probe.exe tools\rt_probe.c $(MYRT_LIB) $(MYLGC_LIB) $(MANIFEST_SYS_LIBS)
	@build\rt_probe.exe
  endif
else
	@$(CC) $(CFLAGS) -o build/rt_probe tools/rt_probe.c $(MYRT_LIB) $(MYLGC_LIB) -lm -lpthread -ldl
	@./build/rt_probe
endif

mylc: $(BIN_MYLC)
myls: $(BIN_MYLS)
myrt: $(MYRT_LIB)

test: all
	bash tests/run_tests.sh
	bash tests/test_backend_parity.sh
	bash tests/test_warn.sh
	bash tools/check_core.sh
	bash tools/check_naming.sh
	bash tools/check_std.sh
	bash tests/run_std_native_tests.sh
	bash tests/test_lib.sh
	bash tests/test_emit.sh
	bash tests/test_native_include.sh
	bash tests/test_toolchain.sh
	bash tests/test_ir.sh
	bash tests/test_mir_c.sh
	bash tests/test_llvm.sh
	bash tests/test_runtime_impl.sh
	bash tests/test_runtime_nostd.sh
	bash tests/test_runtime_bare.sh
	bash tests/test_runtime_qemu_arm.sh
	python3 tests/test_lsp.py

bootstrap:
	bash tools/bootstrap_gc.sh

test-win: all
ifeq ($(OS),Windows_NT)
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
	powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows_batch.ps1 -Backend mir -BatchSize 8
	powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows_batch.ps1 -Backend ast -BatchSize 8
	powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows_batch.ps1 -Backend std
  else
	powershell -NoProfile -ExecutionPolicy Bypass -Command "& { try { & 'tools/test_windows.ps1' } catch { Write-Error $$_ ; exit 1 }; exit 0 }"
  endif
else
	@echo "test-win 需要在 Windows 上执行：MSVC 用 tools/test_windows.ps1；MinGW 用 tests/run_tests_windows.ps1"
endif

# 本地 CI 入口：Linux/macOS 跑完整 make test；Windows 按构建族跑 test-win。
ci:
ifeq ($(OS),Windows_NT)
	$(MAKE) test-win
else
	$(MAKE) test
endif

release-check: dist
ifeq ($(OS),Windows_NT)
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
	powershell -NoProfile -ExecutionPolicy Bypass -Command "& { try { & 'tools/check_release_windows.ps1' } catch { Write-Error $$_ ; exit 1 }; exit 0 }"
  else
	@echo "MSVC 发布检查暂未实现；先运行 tools/package_windows.ps1 并人工验证 win64.zip"
  endif
else
	@echo "release-check 需要在 Windows 上执行（MinGW 用 tools/check_release_windows.ps1）"
endif

dist: bin
ifeq ($(OS),Windows_NT)
  ifeq ($(TOOLCHAIN_FAMILY),mingw)
	powershell -NoProfile -ExecutionPolicy Bypass -Command "& { try { & 'tools/package_windows_mingw.ps1' -SkipBuild } catch { Write-Error $$_ ; exit 1 }; exit 0 }"
  else
	powershell -NoProfile -ExecutionPolicy Bypass -Command "& { try { & 'tools/package_windows.ps1' } catch { Write-Error $$_ ; exit 1 }; exit 0 }"
  endif
else
	@echo "dist 目标需要在 Windows 上执行（MSVC 用 tools/package_windows.ps1，MinGW 用 tools/package_windows_mingw.ps1）"
endif

clean:
	@$(call RMDIR,build)
	@$(call RMDIR,bin)
	@$(call RMDIR,mylc/target)
	@$(call RMDIR,mylgc/target)

FORCE:

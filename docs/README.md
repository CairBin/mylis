# Mylis 文档总览

本文是仓库文档的入口，负责说明“谁在什么场景读哪一份文档”，以及文档变更时必须同步什么。单个主题的细节仍放在对应专题文档中，本页不重复。

## 1. 文档分层

文档按读者任务分为四层，避免把教程、操作步骤、API 参考和架构解释混在同一篇里。

| 层级 | 目标 | 主要文档 |
| --- | --- | --- |
| L0 入口 | 第一次接触仓库，知道项目是什么、怎么构建 | 根目录 `README.md` |
| L1 使用者 | 安装、编译、工程配置、包管理、标准库调用 | `docs/使用手册.md`、`docs/std手册/README.md` |
| L2 维护者 | 语言规则、编译器、运行时、平台侧车、测试 | `docs/Mylis语言规范.md`、`docs/开发者指南.md`、`docs/MIR.md`、`docs/后端与GC路线.md`、`docs/std平台与异步约定.md` |
| L3 协作者 | 人和 Agent 如何安全地修改仓库、如何验收 | `docs/agent开发规范.md`、本页 |

## 2. 按任务查文档

| 想做的事 | 先读 | 再查 |
| --- | --- | --- |
| 构建并运行第一个程序 | 根 `README.md` 第 1 节 | `docs/使用手册.md` 第 1–3 节 |
| 写 `mylis.toml`、配置目标与工具链 | `docs/使用手册.md` 第 4 节 | `docs/包获取与目录结构.md` |
| 查某个 std 类/函数/参数 | `docs/std手册/README.md` | 对应领域章节，如 `docs/std手册/04-网络与事件.md` |
| 写语言语法或语义相关代码 | `docs/Mylis语言规范.md` | `docs/泛型设计.md`、`docs/开发者指南.md` |
| 改编译器、MIR 或后端 | `docs/开发者指南.md` | `docs/MIR.md`、`docs/后端与GC路线.md` |
| 改 std native / 平台侧车 | `docs/std平台与异步约定.md` | `docs/std手册/06-native-ABI与构建.md` |
| 用 Agent 修改仓库 | `docs/agent开发规范.md` | 本页第 3 节、测试相关专题 |
| 写库、发布 `.mylib`、从 C 调用 | `docs/Mylis库标准.md` | `docs/包获取与目录结构.md` |

## 3. 推荐阅读路线

### 3.1 新用户（目标：跑起来并写一个小程序）

1. 根 `README.md`：项目定位、`make all`、`bin/` 结构。
2. `docs/使用手册.md` 第 1–4 节：环境、单文件编译、工程与依赖。
3. `docs/std手册/README.md` 第 1–3 节：std 的导入方式、参数传递、错误模型。
4. 按需读 `docs/std手册/01-容器与算法.md` 或 `02-文本与数据.md`。

### 3.2 库作者（目标：写可复用包）

1. `docs/Mylis库标准.md`：三层模型、源码包与 `.mylib`、C FFI 边界。
2. `docs/包获取与目录结构.md`：`path` / Git / `.mylib` / native 载荷。
3. `docs/std手册/06-native-ABI与构建.md`：需要 native 时再读。
4. `docs/使用手册.md` 第 5 节：生成库并从 C 调用。

### 3.3 编译器/运行时维护者

1. `docs/开发者指南.md`：代码地图、扩展语言、后端、运行时 trait、ABI。
2. `docs/MIR.md`：MIR 指令、verifier、MIR-C。
3. `docs/后端与GC路线.md`、`docs/裸机分配器契约.md`：GC 与 freestanding。
4. `docs/agent开发规范.md` 第 5–7 节：改动与测试矩阵。

### 3.4 Agent（目标：可审计地完成一次修改）

1. `docs/agent开发规范.md`：强制阅读顺序、权限边界、变更记录模板。
2. 与任务领域对应的专题文档；改 std 时必须读 `docs/std手册/README.md` 和对应章节。
3. 改动前先跑基线，改动后按第 6 节测试矩阵逐项记录命令与结果。

## 4. 文档类型与写作规则

- **教程**只讲一条可完成的主线，不塞入全部边界。
- **操作指南**按任务组织，每一步给出可复制命令和预期结果。
- **参考**按名称/签名组织，必须覆盖全部公开 API；`std手册`属于这一类。
- **解释**说明为什么这样设计、有哪些不变量和取舍；语言规范、MIR、平台约定属于这一类。

写作约定：

1. 路径、命令、环境变量、库名使用反引号；命令必须能在仓库根目录直接执行。
2. API 参考以源码签名为准；参数名和类型不得改写。行为描述必须能从源码或测试验证。
3. 新增/删除公开 API 时，同一提交内更新 `docs/std手册` 对应表格。
4. 新增 native 库、头文件或 ABI 时，同时更新 `docs/std手册/06-native-ABI与构建.md`、`docs/std平台与异步约定.md` 和相关 README。
5. 新增 CLI 参数、环境变量或 `mylis.toml` 字段时，同时更新 `docs/使用手册.md` 与根 `README.md` 的速查表。
6. 文档中的示例应尽量来自 `tests/` 或可执行的最小片段；不能验证的示例要标注“示意”。
7. 不使用“显然”“大概”“应该可以”等无法验收的表述；未实现的能力明确写“未实现/未验证”。

## 5. 单一事实源与冲突处理

优先级从高到低：

1. **源码与测试**：当前实际行为，尤其是 `tests/` 中已通过的用例。
2. **语言规范**：规定“应该是什么”；与源码不一致时按规范缺陷或实现缺陷处理，不能静默改文档。
3. **使用手册 / std 手册 / 平台约定**：面向调用者的契约，必须与 1、2 一致。
4. **路线图 / 设计文档**：描述计划，不代表已实现；必须显式标注状态。

发现冲突时，不要只改文字掩盖差异：先在变更记录中写明冲突点，再决定是修代码、修规范还是把文档标为待办。

## 6. 文档维护检查

提交前至少执行：

```bash
# 1. 检查相对链接是否存在（Linux）
python3 - <<'PY'
from pathlib import Path
import re
for md in Path('docs').rglob('*.md'):
    for link in re.findall(r'\]\(([^)#]+)', md.read_text()):
        if link.startswith(('http://','https://','mailto:')): continue
        target = (md.parent / link).resolve()
        if not target.exists():
            print(f'{md}: missing {link}')
PY

# 2. 检查 std 手册是否覆盖全部公开声明（由维护脚本/人工复核）
rg -n '^pub (class|interface|enum)|pub (static )?(unsafe )?fn' std/src | wc -l

# 3. 检查旧路径/旧名称残留
rg -n 'Net_bind|net\.rs.*TCP/UDP 后端|std-posix/native/net\.rs.*唯一' docs README.md
```

最后一条命中旧描述时应更新，不应保留过期文档。

## 7. 文档变更记录

文档也是代码的一部分。任何影响使用者或维护者的文档改动，都要在提交说明中写清：

- 改了哪些文档、对应哪次代码/行为变化；
- 哪些链接、命令或 API 表已复核；
- 哪些内容仍是计划或未验证。

完整模板见 [`agent开发规范.md`](agent开发规范.md) 第 8 节。

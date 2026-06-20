# 设计说明

给后续维护者看的短说明。这里记录的是当前取舍，不是永久承诺。

## API 后端：用 `lark-cli`，不直接接 Rust SDK

飞书 / Lark 的 Rust SDK 覆盖不算稳定，而 `lark-cli` 已经处理了认证、scope、导出、下载和删除确认。`fst` 只做一个小的 Rust 编排层：

- CLI 和配置
- 报告文件
- 排序和汇总
- 备份目录结构
- 删除计划安全流程

只有当 `lark-cli` 做不了时，才考虑直接调 API。

## 外部 Lark JSON：用 `serde_json::Value`

解析 Lark 返回值时继续用 `Value` 是有意的。不同 `lark-cli` 命令字段形态不完全一致，例如 `token` / `file_token`、`name` / `title`、`page_token` / `next_page_token`。

如果强行写 struct，大概率会变成一堆 `Option` 和别名字段，代码更多，弹性更差。这里保持动态解析，并用 `json_string()`、`json_string_any()`、`metadata_bytes()` 这类小 helper 收住脏活。

我们自己拥有的数据继续用强类型 `serde` struct：配置、报告、删除计划和结果。

## CSV：用 `csv` crate，不手写解析

CSV 有引号、转义、值里带逗号、表格软件编辑后的兼容问题。标准库没有 CSV 解析器。`csv` crate 小、常用、边界情况已经处理好。

读取时用 `HashMap<String, String>`，因为输入可能来自不同命令或旧报告，表头会有轻微差异。字段别名和优先级放在 `csvutil` helper 里显式维护。

## size 字段：不要乱合并优先级

不要把所有 size 字段查找列表强行合成一个。字段顺序会影响行为。

例如：

- 删除计划读取已确认计划时，优先 `known_bytes`
- 报告汇总时，优先 `total_bytes`

如果一个共享 helper 会改变字段优先级，就不要用它。

## 删除安全

真正删除必须同时满足：

- `delete_candidate = true`
- `human_confirmed = true`

执行时还必须带 `--yes`。这里保持无聊，别在数据删除上耍聪明。

## 测试

测试保持小而直接。解析器或安全规则容易悄悄坏掉时，加一个最小测试。没有真实 bug 前，不加 fixture 和大测试框架。

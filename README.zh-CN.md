# fst / 飞书存储工具集

`fst` 是一个简洁的飞书 / Lark 存储工具集，用于获取云空间资源清单、诊断大文件/大文档、导出备份，并只删除经过人工确认的清理目标。

`fst` 使用 `lark-cli` 作为飞书 API 后端。Rust 负责配置、编排、报告、排序、备份目录结构和删除计划安全流程。

English documentation: [README.md](README.md)

设计说明：[docs/design.zh-CN.md](docs/design.zh-CN.md) / [English](docs/design.md)

## 工作流

```text
配置
  -> 环境和权限检查
  -> 获取清单
  -> 诊断大小
  -> 导出备份
  -> 生成删除计划
  -> 人工确认后删除
```

工具把诊断、备份、删除拆开。所有报告同时输出 CSV/JSON，方便人工审核、编辑和续跑。

## 运行

开发阶段：

```bash
cargo run -- <command>
```

构建或安装后：

```bash
fst <command>
```

示例：

```bash
fst doctor
fst --verbose doctor
fst list --limit 20
fst audit size --mode full --limit 20
```

## 工具链

`fst` 通过 [`pixi`](https://pixi.sh) 统一管理工具链，让本地和 CI 使用完全
一致的环境（`just` / `git-cliff` / `lefthook` / `rtk-cli`）。Rust 本身由
`rust-toolchain.toml` 锁定在 v1.96（edition 2024）。

首次配置：

```bash
pixi install
just hooks-install   # 安装 lefthook git 钩子
```

常用 recipe（都通过 `pixi run` 运行）：

| Recipe | 用途 |
|---|---|
| `just dev` | 快速本地回路：fmt + check + clippy |
| `just ci` | 完整 CI：fmt-check + check + clippy-deny + test + machete |
| `just fmt` / `just fmt-check` | 格式化 / 校验格式 |
| `just clippy` / `just clippy-deny` | Lint / 警告视为错误 |
| `just test` | 跑测试 |
| `just run <args>` | 运行二进制 |

请优先用 `just <recipe>` 而不是裸 `cargo <cmd>` —— `Justfile` 会把所有
命令包装成 `pixi run -- rtk ...`，保证环境可复现。

## 配置

默认配置路径：`~/.fst/config.toml`。

示例配置：[src/config.example.toml](src/config.example.toml)。

创建配置：

```bash
fst config init
```

查看有效配置。`fst config` 和 `fst config show` 等价：

```bash
fst config
```

指定其他配置文件：

```bash
fst --config path/to/config.toml list
```

默认配置内容：

```toml
[storage]
# fst 所有本地输出的工作区根目录（备份/导出、媒体、报告、清单）。
# 默认 ~/.fst/storage，配置和数据同住在 ~/.fst/ 下。可改指到别的磁盘：
# root = "D:/backup/feishu/fst-storage"
# root = "/mnt/nas/feishu/fst-storage"

[list]
# lark-cli 搜索分页大小。
page_size = 20
# 默认搜索关键词。空字符串表示不按关键词过滤。
query = ""
# 默认只列出自己拥有的资源。别人共享给你的文件通常不占你的空间，
# 所以这里默认 true。
mine = true

[audit]
# top-* CSV 报告保留的最大行数。
top_limit = 100
# 默认下载/统计文档内嵌媒体，让大小估算更接近备份和删除判断。
include_media = true
# 导出/下载时允许的最大并发 lark-cli 子进程数。
# 设为 1 可恢复以前的串行行为。
concurrency = 4

[delete]
# 已知大小大于等于该值时，标记为删除候选。
min_mb = 100.0
# 只保留最大的 N 个删除候选。0 表示不限制。
top = 0
# 大小未知的行也保留在人工 review 计划里，不静默隐藏。
include_unknown = true
```

输出目录是代码约定，不进入配置：

```text
~/.fst/storage/
  backups/
    exports/   # backup/audit 导出的飞书原生文档
    files/     # backup download-files 下载的普通 Drive 文件
    media/     # full audit/backup 下载的文档内嵌媒体
  reports/     # CSV/JSON 报告和删除计划
```

备份和报告放在同一个工作区，方便直接根据生成的 CSV 复查、编辑和续跑。

## Doctor / 环境和权限检查

```bash
fst doctor
```

`doctor` 默认执行尽可能完整、但不做危险操作的实用检查。

它会检查：

- 本地配置路径
- `lark-cli --version`
- `lark-cli auth status`
- 常用 scope 文本提示
- `drive +search` 云空间搜索探测
- Drive 根目录 `files list` 探测
- 自动选择一个可导出样本并探测
  - 对 doc/docx 样本执行 `docs +fetch`
  - 执行 `drive +export`

可选指定文档探测：

```bash
fst doctor --doc <doc_url_or_token>
fst doctor --doc-type docx --doc-token <token>
```

`doctor` 能发现系统性问题，但不能证明每个文件都一定可访问。飞书权限是逐资源模型，还会受到文件夹权限、归属人、Wiki 路由、租户策略和密级等影响。

## 获取清单

### 搜索式清单

```bash
fst list
fst list --limit 100
fst list --query "关键词"
fst list --doc-types docx,sheet,bitable,file,folder,wiki,slides
fst list --mine=false
```

默认使用 `drive +search --as user`。

输出：

```text
list-<timestamp>.json
list-<timestamp>.csv
```

### 文件夹树清单

列出 Drive 根目录：

```bash
fst list --folder-token ""
```

列出指定文件夹：

```bash
fst list --folder-token <folder_token>
```

递归列出：

```bash
fst list --folder-token <folder_token> --recursive
fst list --folder-token <folder_token> --recursive --max-items 1000
```

文件夹清单使用 `drive files list`，不是搜索接口。输出包含 `path` 字段，用于区分同名文件。

## 大小诊断

```bash
fst audit size --mode metadata
fst audit size --mode export
fst audit size --mode full
```

| 模式 | 是否下载 | 用途 |
|---|---:|---|
| `metadata` | 否 | 快速扫描，主要适合 metadata 里有 size 的普通文件。 |
| `export` | 是，导出原生文档 | 按导出文件大小排序飞书原生文档。 |
| `full` | 是，导出 + 媒体 | 最接近真实备份体积。 |

使用已有清单作为输入：

```bash
fst audit size --mode metadata --input list-xxx.csv
fst audit size --mode export --input list-xxx.csv
fst audit size --mode full --input list-xxx.csv
```

输出：

```text
size-results-metadata-<timestamp>.json
size-results-metadata-<timestamp>.csv
top-large-metadata-<timestamp>.csv

audit-results-<timestamp>.json
audit-results-<timestamp>.csv
top-large-docs-<timestamp>.csv
failed-<timestamp>.json
```

`full` 模式会尝试从文档 Markdown 中识别并下载以下内嵌资源：

```text
source
img
image
file
whiteboard
```

## 备份

### 导出飞书原生资源

```bash
fst backup export
fst backup export --limit 100
fst backup export --input list-xxx.csv
fst backup export --include-media
fst backup export --include-media=false
```

支持导出类型：

```text
doc
docx
sheet
bitable
slides
```

`backup export` 默认会包含文档内嵌媒体，因为备份的目标是尽量保留资源。

#### 并发

导出与下载循环最多同时运行 `[audit].concurrency` 个 lark-cli 子进程（默认 4）。
设为 `concurrency = 1` 可恢复串行行为。

#### 断点续跑

导出与下载在中断后可安全重跑。未加 `--force` 时，本地已存在的目标文件会被跳过。
如果输入 CSV 带有已知大小（`metadata_bytes` / `total_bytes` / `export_bytes`），
fst 还会要求本地文件大小与之一致；Ctrl-C 留下的截断/写一半文件会自动重下。
没有大小提示的文件退化为“存在即跳过”。

### 下载普通 Drive 文件

```bash
fst backup download-files --input list-xxx.csv
fst backup download-files --input list-xxx.csv --limit 100
fst backup download-files --input list-xxx.csv --force
```

该命令下载 `doc_type=file` 的普通云盘文件。

输出目录：

```text
~/.fst/storage/files/
```

报告：

```text
file-download-results-<timestamp>.json
file-download-results-<timestamp>.csv
```

## 删除计划

从审计或清单 CSV 生成可人工审核的删除计划。

```bash
fst delete plan --input top-large-docs-xxx.csv
fst delete plan --input audit-results-xxx.csv --min-mb 500
fst delete plan --input audit-results-xxx.csv --min-mb 500 --opened-before 180d
fst delete plan --input audit-results-xxx.csv --updated-before 2024-01-01
fst delete plan --input list-xxx.csv --include-unknown
fst delete plan --input list-xxx.csv --inspect-wiki
```

输出：

```text
delete-plan-<timestamp>.json
delete-plan-<timestamp>.csv
```

删除前，需要人工编辑 CSV，把确认删除的行设置为：

```text
delete_candidate = true
human_confirmed = true
```

### Wiki 处理

`drive +delete` 不能直接删除 wiki 资源。请使用：

```bash
fst delete plan --input list-xxx.csv --inspect-wiki
```

它会调用 `drive +inspect`，尝试把 wiki 行替换成底层 Drive 对象的类型和 token。

## 报告汇总

汇总任意 `fst` CSV 报告：

```bash
fst report summary --input audit-results-xxx.csv
fst report summary --input delete-plan-xxx.csv --top 20
```

汇总内容包括总行数、已知/未知大小、删除候选、已确认行、类型分布、状态分布和 Top 大文件。

## 执行删除

默认预演：

```bash
fst delete apply --input delete-plan-xxx.csv
```

真正删除：

```bash
fst delete apply --input delete-plan-xxx.csv --yes
```

安全规则：

- 默认只预演。
- 真删除必须传 `--yes`。
- 只有同时满足两个标志的行才会删除。

```text
delete_candidate = true
human_confirmed = true
```

支持删除类型：

```text
file
docx
bitable
doc
sheet
mindnote
folder
shortcut
slides
```

删除结果：

```text
delete-results-<timestamp>.json
delete-results-<timestamp>.csv
```

删除后的资源会进入飞书回收站。根据 `lark-cli drive +delete` 行为，文件夹删除可能是异步任务。

## 推荐流程

### 快速 metadata 扫描

```bash
fst doctor
fst list --limit 1000
fst audit size --mode metadata --input ~/.fst/storage/reports/list-xxx.csv
```

### 准确诊断原生文档

```bash
fst list --doc-types docx,doc,sheet,bitable,slides
fst audit size --mode full --input ~/.fst/storage/reports/list-xxx.csv
```

### 删除前备份

```bash
fst backup export --input ~/.fst/storage/reports/list-xxx.csv
fst backup download-files --input ~/.fst/storage/reports/list-xxx.csv
```

### 人工确认清理

```bash
fst audit size --mode full --input ~/.fst/storage/reports/list-xxx.csv
fst report summary --input ~/.fst/storage/reports/audit-results-xxx.csv
fst delete plan --input ~/.fst/storage/reports/top-large-docs-xxx.csv --min-mb 500
```

然后人工编辑 `delete-plan-xxx.csv`。

```bash
fst delete apply --input ~/.fst/storage/reports/delete-plan-xxx.csv
fst delete apply --input ~/.fst/storage/reports/delete-plan-xxx.csv --yes
```

## 输出目录结构

默认配置下：

```text
~/.fst/storage/
  backups/
    exports/
    files/
    media/
  reports/
    list-<timestamp>.json
    list-<timestamp>.csv
    manifest-<timestamp>.json
    size-results-metadata-<timestamp>.json
    size-results-metadata-<timestamp>.csv
    top-large-metadata-<timestamp>.csv
    audit-results-<timestamp>.json
    audit-results-<timestamp>.csv
    top-large-docs-<timestamp>.csv
    failed-<timestamp>.json
    file-download-results-<timestamp>.json
    file-download-results-<timestamp>.csv
    delete-plan-<timestamp>.json
    delete-plan-<timestamp>.csv
    delete-results-<timestamp>.json
    delete-results-<timestamp>.csv
```

## 认证和权限

`fst` 直接调用 `lark-cli`，访问用户云空间资源时使用 `--as user`。

常用授权命令：

```bash
lark-cli auth login --domain docs
lark-cli auth login --scope "search:docs:read"
lark-cli auth login --scope "docs:document:export"
lark-cli auth login --scope "docx:document:readonly"
lark-cli auth login --scope "docs:document.media:download"
```

常用权限：

```text
search:docs:read
docs:document:export
docx:document:readonly
docs:document.media:download
drive:drive:readonly
drive:drive
```

如果 `lark-cli` 返回认证或权限错误，`fst` 会保留原始输出，并追加简短的下一步建议。

## 限制

- `doctor` 不能证明每个文件都一定可访问，因为飞书是逐资源权限模型。
- 原生飞书文档通常没有可靠 metadata size，需要 export/full 模式才能更准确排序。
- 当前内嵌资源识别主要覆盖 Markdown 媒体标签：`source`、`img`、`image`、`file`、`whiteboard`。
- `delete apply` 不直接删除 wiki 行，除非已通过 `delete plan --inspect-wiki` 解包成支持的 Drive 类型。
- 删除流程刻意保守且串行执行。

`--verbose` 可以开启 debug 日志，也支持 `RUST_LOG` 环境变量。

## 退出码

fst 通过进程退出码区分失败模式，以便 shell 脚本能针对性处理：

| 码 | 含义 |
|---:|---|
| 0 | 成功 |
| 1 | 未分类 / 内部错误（anyhow 兼底） |
| 2 | 用法错误（参数错误、缺少配置、输入非法） |
| 3 | 认证失败（需重新登录） |
| 4 | 缺少权限（需申请权限） |
| 5 | 网络/IO 错误（暂时性） |
| 6 | lark-cli 调用失败（非暂时性子进程错误） |

clap 参数错误（退出 2）和 panic（stderr 显示为 `fatal: internal panic at <file>:<line>:<col>`）不经过此映射。

示例 shell 包装：

```bash
fst audit size --mode export --input list-xxx.csv
rc=$?
if [ $rc -eq 3 ]; then lark-cli auth login --domain docs
elif [ $rc -eq 4 ]; then lark-cli auth login --scope "$missing"
elif [ $rc -eq 5 ]; then sleep 30; fst audit size --mode export --input list-xxx.csv
fi
```

## 命令汇总

```bash
fst completions powershell
fst completions bash
fst completions zsh

fst config init
fst config show

fst doctor
fst doctor --doc <doc_url_or_token>
fst doctor --doc-type docx --doc-token <token>

fst list
fst list --folder-token ""
fst list --folder-token <folder_token> --recursive

fst audit size --mode metadata
fst audit size --mode export
fst audit size --mode full
fst audit export

fst backup export
fst backup download-files --input <list.csv>

fst report summary --input <report.csv>

fst delete plan --input <audit-or-list.csv>
fst delete plan --input <audit-or-list.csv> --inspect-wiki
fst delete apply --input <delete-plan.csv>
fst delete apply --input <delete-plan.csv> --yes
```

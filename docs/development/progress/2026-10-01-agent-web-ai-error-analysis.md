# Agent 翻译与网页 AI 错误分析

> 分析日期：2026-10-02
>
> 本文只整理安装日志、Pi 会话记录和源码证据，不修改 Agentero 源代码。

## 1. 取证范围

- 安装程序：`D:\Agentero\agentero.exe`
- 安装版运行日志：`C:\Users\xuan\AppData\Local\com.poco-ai.agentero\logs\agentero.log`
- Agentero 源码：`E:\Desktop\workspace\Project\Agentero`
- Pi 会话：`C:\Users\xuan\.pi\agent\sessions\--E--Desktop-workspace-论文--`
- Pi 当前设置：`C:\Users\xuan\.pi\agent\settings.json`
- Agentero 当前设置：`C:\Users\xuan\AppData\Roaming\agentero\settings.json`

日志中没有发现需要在本文输出的 Cookie、Authorization 或完整认证信息；本文只保留 provider、模型、错误类型、时间和 session ID 等定位字段。

## 2. 全文翻译失败的结论

### 2.1 直接根因已经确认

2026-10-01 22:49:31 首次提交全文翻译任务：

```text
[22:49:31] agent_run_once agent_id=catalog-pi ... accepted=true
[22:49:54] agent_run_session session_id=dbb8716d-... agent_id=catalog-pi
[22:49:54] layout translate glossary generation failed
           error=Agent returned an empty translation
```

同一批次的 Pi 会话记录实际使用：

```text
provider=CKFF-Grok
model=grok-4.7
stopReason=error
errorMessage=503: {"code":"model_not_found",
  "message":"No available channel for model grok-4.7 under group default ..."}
```

因此链路是：

1. Agentero 通过 `catalog-pi` 发起 ACP 请求。
2. Pi 会话选用了 `CKFF-Grok / grok-4.7`。
3. 上游返回 HTTP 503，原因是该模型没有可用 channel。
4. Pi 没有产生 assistant 翻译文本。
5. Agentero 收到空结果，在前端 `layout-translate.ts` 中触发 `Agent returned an empty translation`。
6. glossary 生成失败，后续批量请求虽然显示 `accepted=true`，但这只代表请求已登记并进入后台执行，不代表模型已经成功生成译文。

22:49:54 至 22:51:26 之间，多条 `agent_run_once accepted=true` 后均只出现 `agent_run_session` 结束；Pi 会话中的 503 才是模型层失败证据。故“全文翻译失败”的直接原因不是 PDF 文本为空，而是 Pi 上游模型不可达。

### 2.2 为什么设置中的模型没有成为实际模型

源码链路如下：

- `src/lib/translate/resolve-agent.ts:23-53` 从翻译设置解析 `agentId` 和 `modelId`；未填写固定模型时会读取该 Agent 的模型偏好。
- `src/lib/pdf/layout/layout-translate.ts` 以 `runOnce({ agentId, modelId, workflow: "pdf-layout-translate" })` 发起布局翻译。
- `src-tauri/src/features/agent/service.rs:262-275` 将 `request.model_id` 传给 `RunOnceParams.preferred_model_id`。
- `src-tauri/src/features/agent/session/run.rs:709-767` 建立 ACP session 后，通过 `set_config_option` 尝试设置偏好模型。

`apply_session_preferences` 在 736 行取得偏好模型，在 738 行判断是否已是当前模型，在 739-750 行发送 ACP `set_config_option`。如果请求失败，756-765 行只写 debug：

```text
agent=<id> set model failed (listed=<bool>): pref=<model> err=<error>
```

然后继续执行，不会因为模型设置失败立即终止本次 run。当前安装日志没有出现这条 debug，也没有记录 ACP 返回的 `config_id`、请求模型、是否在模型列表中、切换后的实际模型。因此可以确定：

- Agentero 代码具备传递配置模型的路径；
- 该失败 session 实际运行的是 Pi 默认 `CKFF-Grok / grok-4.7`；
- 配置模型没有成为实际会话模型，程序保留或回退到了 Pi 当前默认模型；
- 仅凭现有日志不能进一步区分：`modelId` 当时为空、模型 ID 不在 ACP 列表、`set_config_option` 被拒绝/忽略，还是复用的 Pi session 保留旧模型。

Pi 设置文件当时显示的默认值为 `defaultProvider=CKFF-Grok`、`defaultModel=grok-4.7`。Agentero 当前设置文件是在失败后读取的，`translate.modelId` 为空，不能直接作为 22:49 失败时的历史配置快照。

## 3. Agent 报错应该怎样体现在日志中

现在的日志把“请求被接受”和“模型执行完成”分开记录，但 `agent_run_once ok=true accepted=true` 很容易被误读为成功。对于一次翻译，至少应形成以下关联事件：

```text
agent.run.accepted session_id=... agent_id=catalog-pi workflow=pdf-layout-translate
agent.model.requested session_id=... provider=... model=... config_id=... listed=...
agent.model.applied session_id=... provider=... model=... actual_model=...
agent.failed session_id=... phase=model_request provider=... model=...
  stop_reason=error upstream_status=503 error_code=model_not_found
agent.completed session_id=... ok=false output_chars=0
translation.failed session_id=... reason=empty_output
```

建议字段：`session_id`、`message_id`、`agent_id`、`workflow`、请求模型、实际 provider/model、ACP config ID、模型是否在 advertised list、`stopReason`、上游 HTTP 状态和错误码、输出字符数、最终业务错误。不要记录完整提示词、论文全文、Cookie 或 Authorization。

## 4. 网页 AI 测试错误清单

### 4.1 Gemini 子 WebView label 冲突（当前网页 AI 主要错误）

安装日志唯一一条同类错误：

```text
[2026-10-01][23:45:30][WARN][agentero::web_ai]
child WebView failed for gemini:
a webview with label `agentero-web-ai-gemini` already exists
```

源码位置：

- `src-tauri/src/features/web_ai/controller.rs:125-155`：打开时只检查内存 `state.views` 是否有 provider 实例。
- `controller.rs:158-204`：固定使用 `agentero-web-ai-{provider}` label 创建子 WebView。
- `controller.rs:204-247`：`add_child` 失败后才记录告警并尝试 `web-ai-window-{provider}` 回退窗口。
- `controller.rs:275-284`：关闭时从内存 map 移除后调用 `close()`。
- `src/components/web-ai/web-ai-panel.tsx:43-83`：provider 切换、面板卸载时异步发送 `visible:false`；`98-114` 又执行 open 和 `visible:true`。

这说明创建时 Tauri/WebView2 仍认为该 label 已存在，而 controller 的内存状态没有相应 entry。可能路径包括旧实例未真正销毁、关闭/重新打开并发、或状态 map 与原生 WebView 生命周期不同步。现有日志没有记录 `state.views` 是否已有 entry、调用来源、close 完成时间和 fallback label 是否已存在，所以不能从日志单独判定是哪一种竞态。

### 4.2 Gemini/ChatGPT 登录跳转被导航白名单拒绝

日志中有 2 条：

```text
[2026-09-28][22:55:21] navigation denied provider=chatgpt host=accounts.youtube.com path=/accounts/SetSID
[2026-09-29][00:35:40] navigation denied provider=gemini host=accounts.google.de path=/accounts/SetSID
```

对应代码：

- `src-tauri/src/features/web_ai/controller.rs:429-449` 的 `handle_navigation` 做白名单判断并记录告警。
- `src-tauri/src/features/web_ai/providers.rs:52-55` 中 Gemini 的 `auth_origins` 只有 `accounts.google.com`、`accounts.youtube.com`；实际错误 host 为 `accounts.google.de`，不在列表内。

这类错误会阻止认证跳转，可能导致网页 AI 显示未登录或登录流程中断。它与 10 月 1 日的 label 冲突是两类独立问题。

### 4.3 历史 glossary WebView 类型错误

9 月 28 日共有 11 条：

```text
layout translate glossary generation failed
error=current webview is not a WebviewWindow
```

这不是网页 AI provider 页面错误，而是全文翻译 glossary 流程调用 WebView/窗口 API 时的宿主类型不匹配。定位区域是 `src/lib/pdf/layout/layout-translate.ts` 的 glossary 生成调用，以及 Tauri 窗口/当前 WebView 获取相关代码。它与 Pi 503 同属翻译流程，但根因不同，不能合并为“模型不可达”。

### 4.4 翻译 provider 探测失败（测试噪声）

10 月 1 日 23:44 的 provider 探测中：

```text
provider=google     HTTP request failed ... translate.google.com ... 5012ms
provider=googleapi  HTTP request failed ... translate.googleapis.com ... 5012ms
```

同一轮 `huoshanweb`、`tencenttransmart`、`deeplx` 和 `openaiCompatible` 成功。因此这两条是 Google 网络/接口探测失败，不是 Web AI WebView 创建问题。

### 4.5 PDF 路径被当作论文目录

日志多次出现：

```text
enqueue paper layout analysis failed ... error=paper folder not found
pdf: list paper dir failed dir=...<file>.pdf error=目录名称无效 (os error 267)
```

错误路径的末尾是 `.pdf` 文件，而调用方在尝试读取论文目录。对应区域是前端论文布局分析入队和 Rust PDF/paper 目录枚举边界。这会阻止布局分析，属于论文路径/任务编排问题，不是 Web AI provider 或 Pi 模型问题。

## 5. 按优先级整理的问题区域

| 优先级 | 问题 | 证据 | 主要源码区域 |
| --- | --- | --- | --- |
| P0 | Pi 模型不可达导致全文翻译空结果 | Pi 503 `model_not_found`，随后 Agentero 空翻译 | Pi ACP 会话、`layout-translate.ts`、agent session runner |
| P1 | 设置模型未成为实际 session 模型且缺少可观测性 | 实际 provider/model 是 Pi 默认；缺少 set-model 结果日志 | `resolve-agent.ts`、`service.rs`、`session/run.rs` |
| P1 | Gemini 子 WebView label 已存在 | 23:45:30 `child WebView failed` | `web_ai/controller.rs`、`web-ai-panel.tsx` |
| P1 | Google 登录域名被白名单拒绝 | `accounts.google.de` 不在 Gemini auth origins | `web_ai/providers.rs`、`controller.rs` |
| P2 | glossary 宿主类型不匹配 | 11 条 `current webview is not a WebviewWindow` | glossary 调用、Tauri WebView/window 边界 |
| P2 | PDF 文件路径当成目录 | `os error 267`、`paper folder not found` | 论文布局入队、PDF 目录枚举 |
| P2 | Google 翻译探测超时/网络失败 | 两条 5012ms 请求失败 | translate provider probe |

## 6. 需要补充的日志关联

后续测试若要精确回答“为什么配置模型没生效”和“WebView 为什么重复创建”，日志至少应补充：

1. ACP：请求进入、session 建立、`set_config_option` 请求模型/config ID/listed、响应中的实际模型、失败错误及 `stopReason`。
2. Agent：`accepted`、`completed`、`failed` 使用同一个 session/message/workflow 关联 ID，并记录输出字符数；`accepted=true` 不再作为成功结论。
3. WebView：provider、固定 label、调用来源、controller map 是否已有 entry、原生实例是否存在、close 开始/完成、create/fallback 结果。
4. 导航：provider、scheme、host、path、匹配到的 allowlist 类别（provider/auth/denied）和拒绝原因。
5. 翻译业务层：输入块 ID、provider/model、返回文本长度、空结果原因、sidecar 写入结果；不记录论文全文。

## 7. 当前结论边界

- 可以确认：本次全文翻译失败的直接根因是 Pi 使用的 `grok-4.7` 不可达，返回 503 `model_not_found`，Agentero 随后得到空翻译。
- 可以确认：网页 AI 当前最明确的错误是 Gemini WebView label 冲突；历史上还存在登录域名白名单拒绝。
- 可以确认：设置模型的代码传递链存在，但实际 session 没有使用配置期望模型。
- 不能仅凭现有日志确认：模型切换是因为设置值为空、模型未列出、ACP 拒绝，还是 session 复用保留旧模型。
- 本次工作没有修改任何源代码；现有工作区中其他未提交改动保持原样。

## 8. 截图错误 `No local PDF for layout analysis`

### 8.1 错误产生路径

错误文案在 `src/lib/pdf/layout/run-analysis.ts` 的远程布局分析函数中抛出：

```ts
const pdfPath = await findLocalPdfPath(paperAbsPath);
if (!pdfPath) throw new Error("No local PDF for layout analysis");
```

进入这个分支需同时满足：布局设置选择了远程 provider、`paperAbsPath` 非空、页数大于 0。布局分析随后用 `paperAbsPath` 调 `findLocalPdfPath`；该函数把参数当论文目录传给 `readDir`，并不会识别参数是否为 `.pdf` 文件再转到父目录。若目录扫描失败或目录内找不到 PDF，就会显示截图中的错误。

截图里的“后台任务”更符合 viewer 的本地 activity 流程：`use-pdf-layout-run.ts` 的自动分析和用户触发分析都能以 `asBackgroundTask: true` 启动 `runLocalActivity`，其 `runCore` 直接调用 `runDocumentLayoutAnalysis`，异常会被后台任务面板记为失败。不要据此认定它就是 Rust `layoutAnalyze` JobCenter 任务。

### 8.2 两种后台任务不可混为一谈

同名任务还有 Rust JobCenter 路径：`enqueue-paper-layout.ts` 注册 `layoutAnalyze` executor，调用 `analyzePaperLayoutHeadless`。headless 函数先查一次 PDF；第一次查不到时返回 `skipped: no local PDF`。若第一次查到，它读取 PDF 后再调用 `runDocumentLayoutAnalysis`；远程 provider 分支会对同一 `paperAbsPath` 再查一次。因此在文件/目录状态发生变化、或两次查找的输入不一致时，headless JobCenter 路径也可能在第二次查找处抛出截图错误，但普通的“任务启动得比下载早”会在第一次查找处软跳过。

Rust `job_layout_analyze_enqueue` 通过 `resolve_paper_dir` 校验论文目录；JobCenter offer 把 vault-relative `paper_path` 交给 renderer，executor 再拼成 `vaultPath/paperPath`。代码契约预期这里传论文目录。

### 8.3 与安装日志的对应关系及根因置信度

日志在 `2026-10-01 23:04:35`、`23:48:25–23:48:42` 和 `2026-10-02 00:23:13–00:24:13` 记录了 `layoutRun` activity 入队，同时记录 `pdf: list paper dir failed`，目录参数末尾明确是 `Adam - A Method for Stochastic Optimization.pdf` 或 `Batch Normalization ... .pdf`，并返回 Windows `os error 267`（目录名称无效）。这直接证实：当时至少有 viewer 布局调用把 PDF 文件路径当成论文目录传入。

但安装日志没有 `No local PDF for layout analysis` 原文，也没有为上述 `layoutRun` 记录绑定 `paperAbsPath`、任务终态或失败详情，故无法证明截图那条任务就是这些路径错误中的某一个，也无法仅凭日志判断是稳定的路径映射错误还是 PDF 落盘/移除竞态。应将结论拆开表述：

- **直接触发条件（源码已证实）：**远程布局分支的第二次 PDF 搜索得到 `null`。
- **已证实的同类路径缺陷（日志已证实）：**部分布局调用把以 `.pdf` 结尾的文件路径交给目录扫描；`findLocalPdfPath` 会扫描失败并返回 `null`，这足以触发该错误。
- **对截图的最可能解释：**该条任务沿用了上述错误路径，导致 remote provider 找不到 PDF。日志无法把具体路径与截图 task ID 关联起来，故不能声称已确认是同一条调用。
- **次要可能：**文件在 headless 首次查找并读取后、远程 provider 再次查找前消失或不可读；当前日志没有文件系统事件证明这一竞态。

### 8.4 代码定位区域

| 区域 | 作用 |
| --- | --- |
| `src/lib/pdf/layout/run-analysis.ts`（远程分支约 423–438 行、抛错约 821–836 行） | 选择远程 provider，并在二次查找为空时抛出截图错误 |
| `src/lib/paper/media.ts`（`findLocalPdfPath`） | 将参数当作目录枚举；`.pdf` 文件路径会导致目录读取失败并转成 `null` |
| `src/components/viewer/pdf/hooks/use-pdf-layout-run.ts`（约 183–190、228–230、377–383 行） | viewer 路径及本地后台 activity 包装；任务失败进入任务面板 |
| `src/lib/pdf/layout/headless-analyze.ts`（约 140–149、217 行起） | JobCenter/headless 路径；首次无 PDF 软跳过，但之后会再次进入共享远程分析逻辑 |
| `src/lib/pdf/layout/enqueue-paper-layout.ts`（约 36–83 行） | JobCenter renderer executor，以 vault 根目录和论文相对目录构造传入路径 |
| `src-tauri/src/features/jobs/commands.rs`（约 368–391 行）、`src-tauri/src/features/jobs/mod.rs`（`validate_job_paper`） | 入队时将目标解析/校验为论文目录 |

### 8.5 需要补齐的诊断日志

目前 `runLocalActivity` 只记了 task enqueue；失败时只更新前端任务状态，未记录 task ID、paper 路径、所选 backend、`findLocalPdfPath` 失败原因或首次/二次查找结果。JobCenter 执行器也只向任务状态报告 error，现有安装日志不足以将面板错误和某次论文路径关联。若后续允许修复，日志应在脱敏前提下记录 `task_id/job_id`、入口类型（viewer activity 或 JobCenter）、paper path、backend、PDF 搜索失败阶段、路径是否存在/是否为目录，以及最终错误；同时记录 JobCenter `job_report` 终态，才能区分参数错位、PDF 未下载和文件系统竞态。

本次只补充了问题分析文档，没有修改源代码。

## 9. `.src/metadata.json` 迁移与给定论文单元的复核

### 9.1 实际论文单元状态

本次复核的目录为：

```text
E:\Desktop\workspace\论文\papers\01-视觉地点识别\paper-0ba0b1910d\
├─ .src\metadata.json
├─ source\
├─ 2021 Autoplace Robust place recognition with single-chi 2109.08652.pdf
├─ NOTES.md
└─ PAPER.md
```

`.src/metadata.json` 中的 `path` 是论文目录相对路径：

```json
"path": "papers/01-视觉地点识别/paper-0ba0b1910d"
```

它没有 `pdf_path` 字段，这是当前设计允许的：PDF 位置由论文目录扫描得到，而不是要求写入元数据。该论文的 PDF 实际位于论文单元根目录，目录本身存在 `NOTES.md` 和 `PAPER.md`，因此即使不依赖隐藏目录，前端文件树也能识别它为论文单元。

### 9.2 `.src` 迁移在代码中的支持情况

目前代码已经在多个边界明确支持 `.src`：

- `src/lib/paper/detect.ts:18-21,120-133,248-251` 把 `.src` 作为论文目录标记，并将其纳入 `paperFolders`；
- `crates/agentero-core/src/features/paper/catalog/papers.rs:846-872` 从 `papers/` 扫描 `NOTES.md` 或 `.src/metadata.json`，重建 catalog；
- `src-tauri/src/features/paper/catalog/commands.rs:100-123` 先把请求解析为论文目录，再用 capabilities 扫描该目录下的 PDF；
- `crates/agentero-core/src/features/paper/capabilities.rs:108-157` 从论文目录根部及浅层子目录寻找 PDF，不读取元数据中的 PDF 字段。

因此，`.src/metadata.json` 本身缺少 `pdf_path` 不是当前全文翻译失败的直接原因。迁移后只要 catalog/file-tree 仍把 `papers/01-视觉地点识别/paper-0ba0b1910d` 作为论文目录，PDF 就能被正常发现。

### 9.3 已确认的直接路径错误

文件树加载流程已经有正确的归一化：`src/lib/workspace/tabs/resources.ts:240` 调用 `paperDirFromPath(path, paperFolders)`，对根目录 PDF 理论上得到 PDF 的父目录；随后 `:287-317` 以该目录读取 catalog、NOTES 和本地 PDF。

但 PDF viewer 传给布局/翻译模块的路径在另一处重新计算。`src/components/workspace/doc-view.tsx:23-30` 的 `paperAbsPathForTab()` 在 `tab.notesPath` 为空、且 tab 直接打开 `.pdf` 时直接返回 `tab.path`，也就是 PDF 文件路径。这个返回值随后在 `:376` 和 `:409` 作为 `paperAbsPath` 传给 `PdfViewer`。

布局/翻译模块把 `paperAbsPath` 当论文目录使用：`src/lib/paper/media.ts:118-164` 对它执行 `readDir(root)`，`src/lib/pdf/layout/run-analysis.ts:821-836` 再用 `findLocalPdfPath(paperAbsPath)` 查找 PDF。于是会出现以下链路：

```text
tab.path = ...\\paper.pdf
  -> paperAbsPathForTab() 返回 ...\\paper.pdf
  -> findLocalPdfPath() 对 ...\\paper.pdf 执行 readDir
  -> Windows os error 267 / 返回 null
  -> No local PDF for layout analysis
```

安装日志中的路径正好满足这条链路：`2026-10-01 23:04:35`、`23:48:25-23:48:42`、`2026-10-02 00:23:13-00:24:13` 多次记录 `.pdf` 结尾的 `dir=`，并报 `目录名称无效 (os error 267)`；同时出现 `paper folder not found`。这比“`.src` 元数据中没有 PDF 路径”更直接地解释了截图中的全文布局/翻译失败。

### 9.4 与迁移的因果边界

当前证据支持以下判断：

1. **直接根因已确认：** viewer 某些入口把 PDF 文件路径传给只接受论文目录的布局/翻译 API。
2. **`.src` 支持已确认：** catalog、文件树和 PDF capabilities 都已按论文目录处理 `.src`，给定论文目录中的 PDF 也确实位于预期位置。
3. **迁移的可能作用：** 论文整理/迁移可能改变了 tab 的打开入口或使 `tab.notesPath` 为空，从而暴露了上述旧的路径契约错误；但现有日志没有记录迁移前后 tab 状态，不能证明迁移代码本身把路径改错。
4. **不能下的结论：** 不能把 `metadata.json` 缺少 `pdf_path`、`.src` 目录位置或 catalog 重建直接认定为全文翻译失败原因。

### 9.5 迁移后仍存在的独立兼容风险（网页 AI）

`src-tauri/src/features/jev/service.rs:20-27` 的 `read_title_from_sidecar()` 仍然只读取：

```text
<paper_dir>/metadata.json
```

而当前论文元数据位于：

```text
<paper_dir>/.src/metadata.json
```

因此网页 AI 的 jEV 标题读取在迁移后的论文上可能返回 `None`，使标题退化为调用方的其他来源或空值。这是一个真实的 `.src` 迁移残留点，应归入网页 AI/智能高亮问题；它不解释日志中已经出现的 `.pdf` 被当目录的 `os error 267`，也不改变 PDF 布局/全文翻译失败的直接根因。

此外，仓库中的 `crates/agentero-core/src/features/paper/import/auto_ingest.rs` 仍有面向旧布局的根目录 `metadata.json` 兼容逻辑。它影响自动导入/迁移兼容，不是本次已记录的 viewer 布局调用路径。

### 9.6 最终问题归类

| 现象 | 结论 | 代码区域 |
| --- | --- | --- |
| 截图中的 `No local PDF for layout analysis` | 高概率由 PDF 文件路径被当作论文目录触发；日志已确认同类调用 | `doc-view.tsx`、`media.ts`、`run-analysis.ts` |
| `.src/metadata.json` 没有 `pdf_path` | 不是直接缺陷；PDF 由目录 capabilities 扫描 | catalog `commands.rs`、`capabilities.rs` |
| 迁移后网页 AI 标题/高亮异常 | 存在旧根目录 `metadata.json` 读取残留，需独立修复评估 | `src-tauri/src/features/jev/service.rs` |
| Pi 全文翻译空结果 | 仍由日志中的 Pi `grok-4.7` 503 `model_not_found` 触发，与 PDF 路径错误是两条并行故障链 | Pi ACP/session、`layout-translate.ts` |

本轮仍未修改源代码，仅补充了上述分析记录。

# Rust 端架构重构计划（2026-09）

状态：**实施中：R1 WAL 一致快照、R2 字段原子更新、R3 sync 占用 RAII、A1 共享移动用例已完成，其余按下文状态推进**。最后更新：2026-09-25。

来源：2026-09-06 三轮架构审计；2026-09-08 由 3 个 sub-agent 分别复核应用边界、存储一致性、运行时与集成，主评审补充核实论文入库及解析链路。本次为静态代码评审，未运行行为测试。历史 V 编号保留，旧 P0–P6 执行顺序由本文新计划替代。

## 核心判断与目标

现有 `agentero-core` / desktop / CLI 三分结构和 feature-first 方向值得保留。主要缺口是共享了底层函数，却没有共享完整业务流程：相同操作可能因桌面、CLI、远端入口不同而执行不同的数据维护规则。

本轮围绕六条主线：**共享应用用例、提交与恢复、Vault 派生状态、论文准备与提取、任务生命周期、Agent 运行上下文**。验收优先看业务一致性、失败恢复和资源归属；删行数、文件长度、trait 数量与 crate 数量仅作辅助指标。

- 保持 local-first：Catalog SQLite 是结构化 metadata 的事实来源；笔记和源文件仍以盘上文件为准；sidecar 是需要可靠传播的投影。
- 复用现有 rename 事务、JobCenter runner/取消清理、HostHooks 的依赖倒置方向，以及 settings 的装配与订阅能力。
- 按用例逐片迁移，保留 wire 兼容，不先做全仓库目录搬迁或万能框架。
- 每项完成后勾选并记录 commit hash、实际验证；实施时更新关联 backend/frontend 文档。
- 下文类型名是建议职责名，不是必须照搬的最终 API；A–F 是工作流编号，依赖与批次见进度总览。

## 本次对旧计划的修正

| 旧方向或表述 | 2026-09-08 决策 |
|---|---|
| 先完成 IPC 包络与宏，再做核心重构 | IPC 整理不是共享用例的前置，后移到收尾 |
| 先把本地操作全面改为 async `VaultFs` | 先统一业务规划与提交契约，按需适配 IO；保留本地路径能力 |
| trash 统一是纯机械、最安全的起点 | 存在 manifest、回滚与远端发布差异；先复用现有移动事务，再处理 trash |
| CLI 可保留移动后提示修复双链的降级 | CLI 与桌面共用移动不变量；不以提示代替正确性 |
| 扩大 HostHooks 成完整宿主面 | 收窄事件出口；后续工作由明确的用例结果与执行策略表达 |
| 所有缓存失效都依赖 UI | CapsCache 已由 Host watcher 失效；需要迁移的是 Wiki 等剩余对账责任 |
| MinerU 完全没有复用 | 已复用请求编排，缺少在途执行与提取产物的共享 |
| JobKind 必须变字符串、进度和取消按类型数量归一 | 保留类型约束；先分离业务策略，共享资源所有权，不强并运行模型 |
| 行数下降、取消类型归一是核心验收 | 改为跨入口契约、故障恢复、取消边界和重复执行测试 |

## A · 共享完整业务用例

**问题与证据：** 原先桌面/Connector 的完整移动服务位于 commands，CLI 只移动文件与 Catalog；A1 已将这三条本地路径统一到 core 用例。跨 Vault migrate、远端移动、trash 与入库仍按后续切片处理。关联 V2–V9、V38。

- [x] **A1 共享移动用例（首个架构切片，2026-09-25）**
  - 在现有 core 的对应业务域建立 application/service 入口，复用已有 Wiki 规划、执行和回滚，不再造 rename 引擎。
  - Desktop 传入当前索引与 dirty paths；CLI 构造所需索引并明确无本进程编辑状态。Connector 调用服务而非 `commands`。
  - 验收：同一 fixture 经桌面服务和 CLI 后，文件、Catalog、`[[...]]` 双链结果一致；失败不遗留半移动状态；桌面脏文档保护保留。
  - 实施：core `catalog/move_paper.rs::move_with_index` 复用 rename 引擎；Host `catalog/service.rs` 仅适配 blocking/索引锁，Connector 不再引用 commands；CLI 的兼容入口构建新索引。`papers` 与 `pdf_page_counts` 路径更新补为同一事务。重复移动统一成功 no-op（桌面原报错语义调整，wire 形状不变）。核心测试覆盖冷/热索引结果、dirty 阻止、第二条 SQL 故障后的文件/链接/Catalog 补偿；Host 测试比较两入口，CLI 真实命令测试覆盖改链/no-op。验证：core Catalog 29 通过；CLI `paper_move` 3 通过；Host service 1 通过；`export_typescript_bindings` 通过且 wire 文件无变化；三 crate `clippy --all-targets -- -D warnings` 通过。提交：`267e22caa`。
  - 后续正确性修复（2026-09-25）：Catalog 的子树快照/删除/移动、页数表和 Usage 筛选/改名统一使用 `sqlite::descendant_path_pattern` + `LIKE … ESCAPE '!'`，避免路径 `%` / `_` 被当作通配符；`!` 同时转义。保留路径边界、Windows 分隔符归一与既有 LIKE 大小写规则。实际 SQL 回归覆盖精确/嵌套路径、相似旁支、转义字符、非 ASCII、页数及 Usage 聚合/其他 Vault 隔离。验证：core Catalog 30 通过、Usage 13 通过；三 crate `clippy --all-targets -- -D warnings` 通过。提交记录待主审提交后补充。
- [ ] **A2 统一 trash/restore 操作计划**（依赖 A1 的用例边界；恢复策略与 B 协作）
  - 收敛本地与远端的校验、恢复 manifest、执行顺序和失败结果；IO 执行器保留能力差异。
  - 验收：移动后 manifest 写失败、Catalog 更新失败、远端发布失败均有明确可恢复结果；同表测试覆盖两种后端。
- [ ] **A3 收敛论文身份与 Vault 能力解析**
  - `PaperUnit` / `PaperLayout` 统一论文 marker、主 PDF、正文源、支撑材料分类，供 rescan、caps、Doctor、树与同步消费；`attachments/` 不是身份 marker。
  - 在适配边界解析本地/远端目标，共享服务只依赖所需能力；不让 core 持有 integration 的具体远端 session 类型。
  - 验收：本地与远端同目录结构得到相同分类；保留有依据的旧 Vault 兼容行为，Windows 路径归一和越界校验一致。
- [ ] **A4 统一入库用例与入口适配**（依赖 B 的提交契约）
  - 本地导入、remote commit、Connector 两条远端路径、Zotero migrate 共用去重、ID 分配、提交结果及失败语义；来源 metadata/linkage、Connector 时限留在入口策略。
  - Catalog 与目录共同参与路径碰撞检查；远端发布及附件延迟明确表达。
  - 验收：不同入口去重、ID 冲突和部分失败的结果一致；保留 Connector 协议时限。能力覆盖后再退役 RemoteImportOps/RemoteTrashOps 和合并孪生命令，不先删除倒置接缝。

## B · 数据提交、投影与恢复契约

**问题与证据：** core `catalog/papers.rs::upsert_paper` 在 DB 提交后独立写 sidecar；字段操作原先读整行再 upsert 的并发窗口已由 R2 的事务内 patch 修复，可靠投影仍待实施；Host `integration/remote/catalog_mirror.rs` 发布原先裸读 SQLite 主文件的问题已由 R1 的一致快照修复。`integration/sync/engine.rs:466` 从拉取的文件重建 Catalog，说明投影失败会影响跨设备传播。关联 V1、V10、V11、V33、V35。

- [ ] **B1 字段级事务与可重试投影**（以前置 R1/R2 为基础；R2 字段修改事务已完成，投影部分未开始）
  - 以 `PaperMutation` 或等价用例保存字段 patch 语义，读改写在一个 DB 事务内完成；不把所有更新统一成全行覆盖。
  - 同事务记录待投影版本，由有序 projector 写 sidecar；失败可重试，旧版本不得覆盖新版本。同步扫描前排空相关投影或显式报告未就绪。
  - 验收：并发改不同字段不丢值；投影失败、乱序及重启后可恢复；Catalog 始终是结构化 metadata 的事实来源。
- [ ] **B2 明确远端 snapshot/publish 边界**（R1 一致快照已完成，其余未开始）
  - 以一致快照发布 Catalog，保持冲突检查；将连接、work-root 和待发布投影纳入远端会话生命周期。
  - 修正 remote commit 先上传目录、后在 staging 生成 sidecar 的顺序缺口（`integration/remote/paper_commit.rs:84`）。
  - 验收：活跃连接写入后 push/pull 数据完整；sidecar 到达正确目标；失败不误报已发布；断开清退连接后才能清理临时目录。
- [ ] **B3 文件操作恢复与数据库策略复用**
  - 移动、trash、入库沿现有补偿能力补齐小型操作记录，明确提交点、可取消点及幂等恢复；仅在需要时引入 journal。
  - 复用 SQLite 连接配置与迁移执行工具，但保留 Catalog、Usage、Feeds、WikiCache 各自生命周期和合理 journal mode。
  - 验收：关键步骤故障注入和恢复重试通过；旧库迁移行为保持。无需全局数据库注册表或跨 SQLite/文件/SFTP 的万能 ACID 事务。

## C · Vault 会话拥有派生状态维护

**问题与证据：** Host watcher 按 window label 注册；`features/vault/watcher/mod.rs:106` 已直接失效 caps，但 `src/App.tsx:288` 仍把 Wiki 变化转成后端重建请求。sync、rename 另有对账入口。关联 V34、V35。

- [ ] **C1 按 Vault 维护变更协调器**
  - 合并同 Vault 的 watcher 所有权；接收用例 changeset 与外部文件事件，按需驱动 caps、Wiki 和 Catalog 对账，再广播给窗口。
  - 明确订阅、连接和任务在 Vault session 关闭时的清退；CLI 未运行 Host 时不假定常驻 watcher，必要不变量由共享用例直接维护，Host 再次打开时对账。
  - 验收：多个窗口共享一次维护结果；Host 无对应窗口但会话活跃时仍能处理外部写入；重新打开后可恢复派生状态。
- [ ] **C2 明确对账与编辑冲突边界**（依赖 C1，接入 A/B 的变更结果）
  - 内部写入和 watcher 回声通过合并、幂等、版本检查去重，不依赖固定静默时间。
  - UI 继续处理 dirty editor 冲突；普通磁盘事件不能无条件用文件投影覆盖权威 Catalog，也不能擅自重写手写笔记。
  - 验收：内部移动不会重复改链；外部编辑、同步拉取、多窗口与关闭重开均有契约测试。无需全局持久化事件总线。

## D · 论文准备与共享提取

**问题与证据：** core `paper_import/mod.rs:224` 从 `progress.app` 推导解析调度；Host `paper/import/job_runners.rs:235` 分别安排正文与版面。MinerU 的 `body_engines/mineru.rs:22` 与 `layout/hosted/mineru.rs:409` 共用请求函数却独立启动提取。关联 V12、V31、V39。

- [ ] **D1 PaperPreparation 统一后续工作计划**（接入 A4 的明确结果；与 E1 协作）
  - 依据最终论文身份、已有资产、缺失产物及配置生成任务依赖；commit/download/recognize 返回结果，不通过进度观察者隐式安排工作。
  - Desktop 策略入 JobCenter；CLI 明确等待或跳过可选派生任务，不依赖 `None` 同时表达无 UI 与无调度。
  - 收窄 HostHooks，事件通知与业务 spawn 分离；配置在用例/任务边界注入，保留 settings 装配与订阅模式。
  - 验收：识别后的稳定路径、Connector 附件就绪、下载后的解析顺序有同一计划来源；无进度监听也不改变必需业务步骤。
- [ ] **D2 DocumentExtraction 共享执行与产物**
  - 先为相同 MinerU 请求共享在途执行，正文与版面分别转换结果；再按收益添加可重建缓存。
  - 复用键覆盖 PDF 内容摘要、provider/服务地址、模型及相关选项、必要的凭据隔离身份和产物版本，不写入明文密钥；明确 force 重算与消费者取消。
  - 验收：兼容配置下同一 PDF 双消费者只启动一次提取；配置不同不误复用；取消一个消费者不破坏其他消费者。Paddle 不同模型保持独立。

## E · 调度内核与任务资源生命周期

**问题与证据：** `features/jobs/mod.rs:417` 集中匹配业务并发，`:1549` 起包含论文后续调度；已存在 runner 注册和取消 RAII。sync 原先手工释放占用且 scheduler 直接 abort 的问题已由 R3 的 `SyncRunLease` 修复。关联 V13、V14、V28–V30。

- [ ] **E1 JobCenter 退出论文业务策略**（与 D1 协作）
  - 论文域构建 JobSpec / PipelinePlan，拥有参数、后续工作和配额策略；调度内核只管理作用域、去重键、资源配额、依赖、执行与终态。
  - 保留已有 runner 注册、backfill probe、panic settle 与取消登记，不重写调度器；不预设必须把 JobKind enum 改为裸字符串。
  - 验收：新增业务任务不修改中央业务 match；既有 fingerprint、Renderer offer/report、timeout 和终态行为兼容。
- [ ] **E2 薄运行作用域与释放契约**（R3 sync lease 已完成，子任务/服务作用域仍未开始）
  - `RunLease` 管理占用/取消登记，任务作用域管理子任务、协作取消及有界等待；区分停止周期触发与取消在途工作。
  - 逐步覆盖同步、Agent 运行及服务句柄；同步退出 flush 与正常同步共享必要的占用规则。
  - 验收：取消、错误、任务 abort 后资源可再次获取；应用退出与配置重启可预测。`spawn_blocking` 和远端写入明确可取消边界，不能宣称 abort 可终止所有操作。
- [ ] **E3 按职责复用服务生命周期**
  - MCP/Connector 可共享 listener、generation、shutdown、join；保留各自 Router、鉴权和协议状态。Bridge 重连、MCP tunnel 子进程监督保留专用模型。
  - 进度共享通用观察字段与适配器，保留领域载荷；Agent、周期同步和长期服务不强塞 JobCenter。
  - 验收：stop/restart 能等待旧 listener 释放，旧任务不能覆盖新状态；协议兼容测试通过。

## F · Agent 运行上下文

**问题与证据：** `accept_run_once` 已改为 `(app, webview_label)`，不再把调用方收成 `WebviewWindow`（子 WebView 存在时该类型注入会失败）。Bridge 仍寻找桌面 webview 并监听手写事件列表。远端主进程可以经 SSH 启动，但 `agent/acp/terminal.rs:124` 仍创建本机进程。关联 V15–V19、V21、V22、V40。

建议职责：`AgentRunContext { execution_target, event_sink, interaction_policy, cancellation }`，避免聚合成所有 managed state 的万能上下文。

- [ ] **F1 显式事件与交互目标**
  - 窗口与 Bridge 各自提供事件出口，服务不再查找任意窗口或依赖 `listen_any` 转发；权限与 ask-user/elicitation 明确接收方。
  - 验收：服务可在不创建 Webview 的情况下测试；隔离不同会话的输出，保持 stream flush 顺序、完成事件与交互回答关联。无交互目标时明确策略，不默许自动审批。
- [ ] **F2 完整 ExecutionTarget**（依赖 F1 上下文；可独立于 A–D 推进）
  - 统一主进程启动、cwd、terminal executor 和 ACP capabilities；远端终端执行在远端，未支持时明确拒绝。
  - 验收：本地/远端主进程与终端位置一致；Windows 子进程选项保留；不静默回退到本机执行。
  - 当前边界（#570）：Unix 已统一启动 cwd，Dsh 自管 launcher；Windows 仅保留旧 Pi / Custom 包装，探针直启。本项仍需补齐原生 cwd、UNC 与进程树终止，不能把本次防回归当成统一执行上下文已完成。
- [ ] **F3 收回命令编排并统一连接装配**（依赖 F1/F2）
  - warm/probe/lifecycle 进入现有 Agent service；probe/warm/run/history 共用连接装配，保留各模式权限和恢复策略。
  - 验收：commands 仅转换参数与结果；provider session ID、恢复回放抑制和取消的 wire 行为不回归。runtime 可留在 desktop crate，不立即拆独立 ACP crate。

## R · 前置正确性修复

小范围修复可单独提交，不等待架构重构；R1/R2/R3 已完成，其余状态见各项。安全与正确性修复不因其规模小而推迟。

验证保护（2026-09-25）：CI 的既有 `cli-tests` 任务改为显式运行 `cargo test -p agentero-core -p agentero-cli`，使 R2 等共享层回归测试进入 PR 检查；复用该任务的 PDFium provisioning 与构建产物，不增加矩阵任务或桌面 adapter staging。本地 `cargo test -p agentero-core`：506 通过、4 个既有手动/性能测试 ignored、0 失败；详见[测试入口](../test/index.md)。提交：`4abb65180`。

- [x] **R1 WAL 一致快照**（旧 P0-1，2026-09-25）：CatalogMirror 初始化与 push 使用 `VACUUM INTO` 导出自包含快照，保留 size/mtime 冲突检查；临时文件由 RAII 清理，不依赖关闭连接或 checkpoint。验证：`cargo test -p agentero catalog_mirror::tests --lib -- --nocapture`（1 通过），LocalFs 回归测试保持 WAL 写连接存活，验证未提交行不可见、提交后再次 push/checkout 的内容及完整性；`cargo clippy -p agentero --lib --tests -- -D warnings` 通过。B2 的投影就绪与会话清退尚未完成。提交：`cf9b4730f`。
- [x] **R2 字段原子更新**（旧 P0-7，2026-09-25）：`papers.rs::mutate_paper` 用 `BEGIN IMMEDIATE` 统一 `update_meta`、`set_is_read`、`set_tags`、`add_tags`、`remove_tags` 的事务内读改写与回读。标签集合修改受同一写预约保护，覆盖独立 SQLite 连接。验证：`cargo test -p agentero-core features::paper::catalog::papers::tests -- --nocapture`（22 通过），新增独立连接写预约/错误释放及并发字段/标签集合测试。sidecar 与 NOTES 仍在提交后 best-effort 写入；B1 的投影重试和顺序保护未完成。提交：`90f26ca99`。
- [x] **R3 sync 占用 RAII**（旧 P0-2，2026-09-25）：`SyncRunLease` 替代手工 begin/end，正常/错误/超时/abort 均释放占用；退出 flush 使用相同 lease，忙碌 Vault 跳过。验证：`cargo test -p agentero integration::sync::tests --lib -- --nocapture`（3 通过），覆盖真实 tokio abort、虚拟时钟 timeout、错误返回与多 Vault 独立性。保留既有 wire 终态；abort 后的 UI 事件对账及子任务协调取消留在 E2，不宣称 abort 可撤销远端 IO 或 `spawn_blocking`。提交：`ef2ca0a75`。
- [ ] **R4 Agent 交互清理与转发**（旧 P0-3/P0-4）：超时/取消移除 pending，补齐 Bridge ask-user/elicitation 请求转发；晚到回答保持 `resolved:false`，完整交互链路验证后再由 F1 替换临时转发。
- [ ] **R5 论文附件分类**（旧 P0-5）：附件 PDF/TeX 不成为主资产；测试锁定 AGENTS.md 约定，不未经确认搬动历史用户文件。
- [ ] **R6 文件授权与会话清退**（旧 P0-6/P0-8 连接项）：规范化路径并校验已授权 Vault 范围；远端断开前释放 work-root 连接和任务。验证正常打开流程及越界拒绝。

## S · 后置整理与条件性工作

这些条目保留为 backlog，不是 A–F 的统一前置。历史细节可查本文件 Git 历史和附录 V 编号。

- [ ] **S1 IPC 契约来源**（旧 P1-1/P1-2/P4-6）：单一命令注册表、iOS 命令契约、明确返回包络与公开事件载荷；随服务迁移逐步收敛，避免一次性全量 breaking change。
- [ ] **S2 错误和路径边界**（旧 P1-4/P1-6/P4-7）：优先替换影响控制流的字符串嗅探，补 NotFound/Conflict/Cancelled 语义与路径类型；边界校验不能等待全仓库 newtype 迁移。
- [ ] **S3 机械收尾**（旧 P1-3/P1-5/P6-1/P6-2/P6-3 及 P0-8 清理项）：按实际收益整理包装宏、计时、别名、glob、重复 helper、死 JobKind 与过期注释；边界稳定后搬 tauri-free 模块。命令宏、platform 目录和新的 crate 均非必选。
- [ ] **S4 资产与进程工具**（旧 P4-3/P4-4/P6-4）：按需求复用下载校验、解包、多源回退、进度/取消与 SSH 选项；保持 `.partial` 原子替换和 Windows 行为。现有 terminal 单任务拥有 Child、MCP tunnel generation 机制作为模板。
- [ ] **S5 Agent 探测与取消细节**（旧 P5-10）：复核当前 PATH/登录 shell 解析后考虑缓存和阻塞隔离；内部 typed cancellation 不直接改变既有客户端 stop_reason 契约。
- [ ] **S6 队列恢复评估**（旧 P3-5）：明确哪些任务允许重试、输入如何持久化后，再决定是否存储 pending/interrupted jobs；不持久化不可序列化执行器，不混入 usage 事实日志，不把所有在途任务自动重跑。

## 进度与依赖总览

| 批次 | 工作 | 主要依赖 | 状态 |
|---|---|---|---|
| 前置修复 | R1–R6 | 各项独立；按影响优先处理 R1/R2/R3 | R1/R2/R3 已完成，其余未开始 |
| 第一批 | A1 移动用例，随后 A2/A3 | 复用已有 rename；A2 与 B3 明确恢复契约 | A1 已完成，A2/A3 未开始 |
| 第二批 | B1–B3、A4、C1/C2 | B 以前置数据修复为基础；A4/C 接入提交结果 | 未开始 |
| 第三批 | D1/D2、E1 | D1 与 E1 先对齐计划接口；D2 可独立试点 | 未开始 |
| 可独立推进 | E2/E3、F1–F3 | E2 从 R3 起步；F 内部按事件→执行→装配 | E2 的 sync lease 已完成，其余未开始 |
| 后置 | S1–S6 | 主线边界稳定、或具体需求证明收益 | 未开始 |

首个架构切片 **A1** 已完成：复用现有 rename，用 CLI/桌面对比证明本地移动语义收敛；后续继续 A2/A3。R 中的数据与安全问题先行或穿插处理，不要求所有 R 完成后才能开始独立主线。

## 旧任务归并索引

| 旧任务 | 新归属 |
|---|---|
| P0-1…P0-8 | R1–R6；死变体和注释清理归 S3 |
| P1-1/P1-2/P1-3/P1-4/P1-5/P1-6 | S1/S1/S3/S2/S3/S2；解析调度 helper 随 D1 |
| P2-1/P2-2/P2-3/P2-4/P2-5/P2-6 | A3/A2/A3/A4+B/A4/A4 |
| P3-1/P3-2/P3-3/P3-4/P3-5 | E1+D1/E1/E3/E2+F/S6；取消强制搬目录和字符串 ID 要求 |
| P4-1/P4-2/P4-3/P4-4/P4-5/P4-6/P4-7 | B3/E3/S4/S4/C/S1/S2；不保留 Bridge 长期监听窗口的建议 |
| P5-1/P5-2/P5-3/P5-4/P5-5 | A3/A1+A2/D1/D2/D1 |
| P5-6/P5-7/P5-8/P5-9/P5-10 | F3/F3/F1/F2/S5+F |
| P6-1/P6-2/P6-3/P6-4 | S3/S3/S3/S4 |

## 明确保留的边界

不扩大 HostHooks 为万能宿主对象，不引入覆盖所有 feature 的 VaultService，不强并 ACP/Bridge/Connector wire 协议。不为了搬模块引入 ConfigProvider，不要求所有本地 IO 走 async trait，不把 UI 未保存内容的决策交给后台对账。继续保留 `core::http`、`run_blocking`、settings 订阅、JobCenter 既有 runner/清理能力与 feature-first 语义目录。

相关路线图：[crate 拆分记录](crate-split-roadmap.md)、[开发索引](index.md)。本文为进行中的架构计划；已实现行为仍以 `docs/backend/`、`docs/frontend/` 和代码为准。

## 附录：历史证据与裁决（V 编号保持稳定）

以下保留 2026-09-06 的审计快照，并在本次核实涉及的条目中修正表述。未重新统计的数量、行号与绝对性论断不代表 09-08 全仓库验证结果；实施前须按符号定位复核。简写 `catalog/`、`paper_import/` 等数据域路径位于 `crates/agentero-core/src/features/paper/`；Host 的 `remote/`、`sync/`、`bridge/` 位于 `src-tauri/src/integration/`。

### 问题证据快照

| # | 论断 | 关键证据 |
|---|---|---|
| V1 | **远端 catalog 发布绕开 WAL 一致快照**：`push` 裸读主文件，而写入连接启用 WAL 且长期缓存，可能遗漏尚未 checkpoint 的已提交修改。09-08 静态复核确认路径；不沿用“首次修改必丢”的绝对结论 | `crates/agentero-core/src/sqlite.rs:16`、`catalog/schema.rs` 的连接缓存、`src-tauri/src/integration/remote/catalog_mirror.rs:105`；需用活跃连接 roundtrip 测试验证 |
| V2 | VaultFs 双轨制：`dyn VaultFs` 13 处全在 `integration/remote/**`，features 0 使用；`LocalFs` 生产实例仅 1 处（local-sim） | `core/fs/mod.rs:27`、`remote/session.rs:120` |
| V3 | remote 分支共 **28 处 / 10 文件**（23 处 `parse_remote_handle` 调用 + 5 处裸 `"remote:"` 比较） | 分布：trash×5、import×4、connector×11、jobs×1、zotero×1、launch×1、mcp×1、sync×1 |
| V4 | `remote_*` 命令 21 个（remote/commands.rs 18 + agent 3），其中**真孪生 5 个**：`remote_paper_{get,list,set_tags,set_is_read,rescan}` ↔ `paper_*` | `app/handlers.rs:155-175` |
| V5 | 3 个 `Remote*Ops` trait 无本地实现，调用点 `if remote { ops } else { 直调 }` | `RemoteImportOps`（features/paper/import/remote_ops.rs:22）、`RemoteTrashOps`（features/vault/trash/remote_ops.rs:17）、`RemoteAgentHosts`（features/agent/remote_host.rs:57） |
| V6 | trash 整体复制分叉：结构体、校验函数、5 个操作全部重写，仅共享结果类型与 catalog 行助手 | core `vault/trash/mod.rs` vs `remote/trash_bridge.rs`（自带头注释"Semantics match local"） |
| V7 | CLI move 只动文件与 SQL；桌面和 Connector 已共用包含双链更新的移动事务，但服务仍放在 `commands.rs`。09-08 确认：需要统一完整用例，不能把 Connector 本地移动报告为另一分叉 | `catalog/mod.rs:66`、`src-tauri/src/features/paper/catalog/commands.rs:550`、`integration/connector/state.rs:766`、`cli/src/commands/paper.rs:689` |
| V8 | caps 误判 attachments：附件 PDF 可成主 PDF（read_dir 顺序不定，可能抢在 source/ 前）；附件 `.tex` 置 `has_tex` 压制 ParseBody。与 AGENTS.md attachments 约定冲突 | `core/paper/capabilities.rs:146,148` |
| V9 | 本地/远端 rescan marker 不同：本地 `NOTES.md∨metadata.json`；远端另加 `highlights.md`/`PAPER.md`/`source\|assets\|marks` 目录 + 从 NOTES 刮标题 + 恒刷 `updated_at` | `catalog/papers.rs:811` vs `remote/commands.rs:581-613` |
| V10 | `set_tags`/`set_is_read`/`add_tags`/`remove_tags` 均为读全行→改→全行 upsert，两次独立拿锁，并发覆盖窗口 | `catalog/papers.rs:1001-1056` |
| V11 | Catalog 提交后独立写 sidecar，投影失败只 log，写入顺序也未由同一事务保护；sidecar 又参与同步传播，需要有序、可重试的投影契约 | `catalog/papers.rs:391`、`catalog/sidecar.rs:17`、`src-tauri/src/integration/sync/engine.rs:466` |
| V12 | MinerU 正文与版面已共用 HTTP 编排函数，但分别调用并启动 provider 提取，未共享执行结果；两类任务可分别入队 | `body_engines/mineru.rs:22`、`layout/hosted/mineru.rs:409`、`paper/import/job_runners.rs:235`；不是“完全没有代码复用” |
| V13 | sync 注销泄漏：`try_begin`/`end` 无 abort 防护（全仓库无 Drop/scopeguard 兜底），scheduler abort 跳过 `end()` → 直到重启永远 "sync already running" | `sync/commands.rs:211,229`、`sync/scheduler.rs:53` |
| V14 | gate 超时泄漏：三个 gate 的 pending map 只在 `resolve()` 删除，300s 超时路径不清理（晚到回答优雅返回 `resolved:false`，不崩） | `runtime/gates.rs:24,71,115`、`acp/interaction.rs:345,388,448` |
| V15 | bridge 事件缺口：`agent:ask-user-request` / `agent:elicitation-request` **不在**转发列表，但回答端 RPC 存在 → 移动端这两个 RPC 是死代码，300s 自动取消。`agent:permission-request` 已转发（无缺口） | `bridge/host.rs:41-49`（转发表）、`host.rs:964-985`（回答端） |
| V16 | 远端 agent 终端本机执行：SSH run 的 `terminal/create` 无条件走本机 `tokio::process::Command`，无远端 executor | `session/run.rs:244,356`、`acp/terminal.rs:124` |
| V17 | runtime 事件目标已从 `WebviewWindow` 改为 webview label（`accept_run_once(app, webview_label)`）；bridge 仍狩猎 webview + `listen_any` 窃听 | `agent/service.rs` `accept_run_once`、`bridge/host.rs` `agent_run_once`、`bridge/host.rs:79-104` |
| V18 | Agent 命令绕过 service：`agent_warm` 内联整套编排；`agent_probe` 近乎逐字复制 `service::probe_catalog`；lifecycle 无 service 包装 | `commands/session.rs:122-183`、`commands/registry.rs:156-178` |
| V19 | 连接装配重复 **5 处**（非 4）：probe/warm/run/history×2，builder+terminal handler+deny-permission+connect_with 样板复制，无共享装配助手 | `acp/probe.rs:37`、`session/warm.rs:78`、`session/run.rs:353`、`session/history.rs:70,411` |
| V20 | PATH 无缓存全量重扫：每次 `agent_run_once` 对**每个**注册 agent 跑 `probe_command`（遍历 nvm/brew/scoop），阻塞 tokio 线程；`upsert`/`discover` 还持锁扫描 | `registry/store.rs:160,341`、`core/process/discover.rs`（零 memoization） |
| V21 | 取消=Ok+字符串：run.rs 恰 9 处 `select!`，取消返回 `Ok` + `stop_reason:"cancelled"` 硬编码 | `session/run.rs:502-768`、`acp/client.rs:266` |
| V22 | `is_ssh()`/`is_local_sim()` flag 泄漏 4 处（比原报告多 2 处） | `remote_host.rs:23,25`、`acp/client.rs:169`、`registry/remote.rs:121,158,211`、`commands/remote.rs:137`、`remote/launch.rs:75` |
| V23 | IPC 包络三态 95 / 94 / **7**（`set_locale` 曾被漏计）；成因是 State-borrow 变通，文档自认 | `integration/remote/commands.rs:1-4`；前端 3 个 unwrap helper（`src/lib/core/ipc.ts`） |
| V24 | fs scope 由渲染层授予：`vault_allow_fs_scope` 命令对任意非空路径 `allow_directory(&p, true)` 递归授予，无 canonicalize/白名单/活跃 vault 校验（host 侧 `handle_open_dir` 反而会先 canonicalize） | `app/vault_session/fs_scope.rs:30`、`vault_session/mod.rs:6-14` |
| V25 | 注册表双份：handlers.rs 191 vs bindings_test.rs 191 手工对齐；`bridge_status` 桌面/iOS 同名不同签名 → iOS 5 个命令整体排除在 bindings.ts 外（cfg 互斥故无运行时遮蔽） | `app/bindings_test.rs:11-13` |
| V26 | `events_contract.rs` 1248 行 + 手写 Rust 源码扫描器；42 事件（`:360` 的"43"注释过期）；57 个 emit 点 | `app/events_contract.rs:905-915` |
| V27 | OpTimer 覆盖 50/196 命令；jobs 全部 20 个命令、agent、feeds、catalog 大部无计时 | `core/log_util.rs` |
| V28 | jobs/mod.rs 2845 行；JobKind 14 变体（严格 paper 11 + LibraryIo 边缘）；并发上限 9/14 硬编码字面量；**发现 3 个死变体**：`LayoutTranslate`/`PageCount`/`WikiReindex` 无 runner 无 enqueue | `features/jobs/mod.rs:35-50,417-442` |
| V29 | 历史审计记录多套进度与取消机制；应统一资源释放和观察契约，保留 Agent、周期同步、队列任务各自语义，不以类型数量归一为验收 | 本次主线 E；旧 P3-3/P3-4 的迁移对象包括 sync、Zotero sync、安装器、Connector、Bridge 和下载进度 |
| V30 | JobCenter 无持久化，崩溃丢队列 | `jobs/mod.rs:247-266`（无 Serialize） |
| V31 | `probe_command` 薄重复包装（函数体逐字节相同，均委托同一 core `resolve_command`；测试不同）；`spawn_parse_after_import` 5 处定义（1 trait 声明+1 impl+1 固有方法+2 自由函数） | `core/process/discover.rs:150` vs `agent/registry/discovery.rs:9`；`core/app_handle.rs:29,69`、`host_hooks.rs:27`、refs 两处 |
| V32 | 错误分类学空壳：`AppError::message` 937 处 vs `domain` 13 处；3 处靠嗅探文本决策（CLI 子串退出码、`VaultFs::exists` grep "not found"、connector `contains("SESSION_EXISTS")`） | `cli/src/error.rs:107-133`、`core/fs/mod.rs:44-62`、`connector/server.rs:329` |
| V33 | 存储管道四套写法：4 种 SQLite 连接策略 / 4 套迁移梯子；mirror 连接无 busy_timeout；`CatalogMirror::open` 生产零调用 | `usage/schema.rs:116`、`feeds/mod.rs:524`、`wiki/cache.rs:302`（DELETE 模式）、`catalog_mirror.rs:96-102` |
| V34 | Wiki 更新仍经 React 回调 Host，watcher 按窗口管理；但 CapsCache 已由 watcher 直接失效。09-08 修正“全部失效依赖 UI”的表述 | `src-tauri/src/features/vault/watcher/mod.rs:104`（106 行失效 caps）、`src/App.tsx:288`、watcher 的 window-label 注册表 |
| V35 | 远端 work-root 连接永不清退：`vault_release` 按本地路径 canonical 匹配，`remote:<id>` 永不命中；disconnect 也不清；远端 session 的 sidecar 写进临时 work 目录而非远端 vault | `app/vault_session/lifecycle.rs:34`、`remote/session.rs:88` |
| V36 | core/host 边界：src-tauri 14 处 glob 重导出；core 12 个扁平别名且**自用 ~150 处**（src-tauri 侧反而语义路径 83 处/33 文件、扁平 0 处）；`#[path]` 重挂载恰 2 处（cli_install、open_request） | `core/features/mod.rs:16-31`、`src-tauri/features/mod.rs:12-13,21-22` |
| V37 | 两个下载器互补残缺（**非**重复实现）：install/download.rs 有 sha256+解包+chmod 无多源回退无进度无取消；model_assets 有多源回退+节流+取消无校验和。两者都已有 `.partial`+rename（不会留损坏产物） | `agent/install/download.rs`、`layout/model_assets/mod.rs:21,193-243` |
| V38 | Zotero 两个家：core codec+io 1189 行（tauri-free）vs host db.rs 1651 + sync 1352；db.rs 是事实上的 paper source 却无 trait（手工拼 Zotero API JSON 喂 `map_zotero_item_to_record`） | `features/paper/zotero/db.rs:1-6,390` |
| V39 | 历史审计记录 settings 直读 6 模块/10 处；按用例或任务边界显式传配置，保留装配层读取及已有订阅模式 | paper/import、body_engines、layout/hosted、recommend、translate、mcp；本次主线 D |
| V40 | acp/ 层不纯：import `AgentEventEmitter` 并直接发 UI 事件（一跳之隔，无直接 tauri:: import） | `acp/updates.rs:7,407-422`、`acp/interaction.rs:5-6,334-437` |

### 已推翻 / 修正的论断（不要再按原说法执行）

| 原论断 | 裁决 |
|---|---|
| `parse_remote_handle` 38 处/11 文件 | 真实 **28 处/10 文件**（38 是含 import/测试的总文本行数） |
| `layout_backend_source` 直读 settings | **REFUTED**——注入闭包模式，settings 读取在装配层（`app/mod.rs:187`），这是**好设计，应推广而非移除** |
| "core::cancel 只有 JobCenter 用" | **方向反了**：JobCenter 是 probe 提供方，消费者是 core 深处的 import/pdf-parse/scholar/citing |
| enqueue 112 处 | 文本 112，其中 78 处测试、11 处内部委托，**真实外部调用 23 处** |
| "recommend/coolpapers/search/recognize 因 settings 滞留 host（等 ConfigProvider）" | **大部分不成立**：四者主体（807+860+373+1416 行）今天就是 tauri-free 且 settings-free，settings 读取全在 commands 壳；recognize 的接缝早已以函数参数存在。直接搬即可，**ConfigProvider 不做** |
| import-tmp 永不清理 | REFUTED：前端 `cleanupImportTempPaths` finally 清理；仅崩溃孤儿无 host 清扫（降级为可选任务） |
| glob 17 处 / 别名 11 个 / 事件 43 个 | 精确值 **14 / 12 / 42** |
| OpTimer "20/39 文件" | 50/196 命令；文件口径 20/**40** |

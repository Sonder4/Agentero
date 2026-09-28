# PaperReader 网页 AI 能力纳入 Agentero：需求与设计

> 状态：需求与技术设计，尚未实施。  
> 范围：只整理 PaperReader 已有的网页 AI 集成，以及将其纳入 Agentero 的需求；不涵盖本地 ACP Agent 的重构。

## 1. 目标与背景

PaperReader 已经提供嵌入式网页 AI 工作流。用户可以在阅读论文时打开 ChatGPT、Gemini、DeepSeek、Kimi、GLM 等网页端服务，复用网页登录状态，把论文材料、选中文本或截图交给网页 AI，并把网页会话绑定到论文。部分操作还包括 ChatGPT MCP Connector、长文上下文准备、对话改名和项目管理。

目标是在 Agentero 中以 Rust/Tauri 为宿主承接这些能力，同时复用 Agentero 已有的 WebView、Vault、MCP/Tunnel 和 Agent 基础设施。迁移要保留 PaperReader 已经验证的用户流程与数据语义，不能以 ACP 对话面板取代网页 AI。

### 参考实现

- PaperReader：`electron/web-ai-providers.cjs`、`electron/chatgpt.cjs`、`electron/ipc.cjs`、`electron/preload.cjs`、`electron/paper-conversations.cjs`、`src/components/ReaderAssistant.tsx`。
- Agentero：`src-tauri/src/features/web/`（现有普通网页/论文代理）、`src-tauri/src/features/agent/`（ACP）、`src/lib/mcp/tunnel.ts` 及后端 MCP Tunnel 实现。
- PaperReader 的交互原型：`E:\Desktop\workspace\Project\PaperReader\docs\plans\inline_ui_recreation_v2.html`。
- PaperReader 的视觉规范：`E:\Desktop\workspace\Project\PaperReader\docs\plans\InLine_UI组件与视觉设计规范.md`。

## 2. 必须遵守的边界

1. **保留网页 AI 底层能力。**迁移期间不改写或替换 PaperReader 中已经成功的 provider 流程、Electron `chatgpt` bridge、BrowserView 行为、IPC action 名称及已验证的数据协议。确需适配时，在 Agentero 新增薄适配层，并保持原有动作语义。
2. **网页 AI 与本地 ACP Agent 是两类独立能力。**网页 AI 使用用户登录的服务网页；本地 Agent 仍走 Agentero 的 ACP runtime。不得把网页 AI 请求暗中改送 ACP，也不得用 ACP 绕过网页服务的登录/确认流程。
3. **不自动发送。**材料转交后应形成网页输入框中的草稿/附件并明确显示准备状态；最终发送仍由用户在网页服务中确认。任何 provider 能力不能确认时，显示失败/待用户操作，不声称材料已成功提交。
4. **保留用户会话与登录状态。**provider 的浏览器存储彼此隔离；Paper 与 provider 的会话绑定应能持久化并在重启后恢复。
5. **普通网页代理与登录态 Web AI 分离。**Agentero 当前 `agentero-web` 是面向普通网页/论文内容的代理，不具备承载已登录 ChatGPT/Gemini 等站点的 Cookie 与账号会话的职责。不要把敏感登录态塞进该代理。
6. **最小权限与本地优先。**网页服务凭证、Cookie、页面 DOM、论文内容和用户对话不写入日志，不上传到 Agentero 服务。Agentero 后端只持有执行必要操作的短生命周期状态。

## 3. 用户与产品需求

### 3.1 工作区入口

- 在论文阅读/工作区的上下文区域提供网页 AI provider 入口，并与本地 ACP Agent 清楚区分。
- 支持切换 provider、打开或重新显示网页视图、隐藏/关闭视图及设置视图边界；不应切换 provider 就丢失各自登录状态。
- 网页 AI 仍在阅读工作区中辅助论文任务；切换 PDF、笔记或侧栏时工作区不应意外重置 provider 会话。
- 沿用 Agentero 当前主题和组件体系；面板展示连接状态、当前 provider、当前论文绑定会话及材料准备状态，不另造一套全局视觉系统。

### 3.2 Provider 与登录

- 首批覆盖 PaperReader 已支持的 ChatGPT、Gemini、DeepSeek、Kimi、GLM；provider 列表、主页、输入框能力、允许导航的主机名与 DOM 能力应由统一注册表描述。
- 复用 provider 自己的网页登录和现有账户权限，不要求用户把网页 AI API Key 填进 Agentero。
- 每个 provider 使用独立、持久化的 WebView 数据目录/partition。用户关闭面板或重启 Agentero 后，仍可保持服务网站已有登录状态。
- 明确展示加载中、未登录/登录失效、页面不可用、材料准备中、准备成功及失败状态。不得把站点网页的普通加载误报为应用崩溃。

### 3.3 论文和会话绑定

- 对每个 Vault 中的论文，按 provider 保存当前网页 AI 会话标识或 URL，并在再次打开论文时恢复对应 provider 会话。
- 绑定键需包含稳定的 Vault/论文身份和 provider ID；路径规范化和重命名时应避免串到另一篇论文。不得只用标题匹配。
- 用户可以显式新建/解除绑定；不同 provider 的会话不得互相覆盖。
- 会话改名、项目准备/创建仅在 provider 明确支持时启用，并对页面状态变化和失败给出反馈。

### 3.4 材料转交

材料转交不得触发网页消息自动发送。界面须让用户辨认材料来源、页码/选区以及当前准备结果。

| 材料 | 需求 | 成功条件 |
|---|---|---|
| 纯文本/论文片段 | 写入 provider 的网页输入框；支持标题、页码、引用文本等现有上下文 | 网页输入框可见预填文本，显示 `draftReady`；用户自行发送 |
| 选中文本 | 从 PDF/笔记选择后，可转交给网页 AI；保留来源论文和页码上下文 | 选区内容完整进入草稿；失败时保留用户选择供重试 |
| 图片/截图 | 按现有 provider 能力通过剪贴板/粘贴等方式添加 | 只有页面确认收到附件后才报 `attachmentReady` |
| PDF | 允许将当前 PDF 作为网页附件，识别站点附件控件和上传结果 | 必须验证附件确已显示/上传；不支持的平台提供用户可完成的文件选择回退 |
| 长文上下文 | 超过网页输入能力时，准备可读的长文上下文导出/附件 | 明示实际准备的文档及范围，不静默截断或误报完整 |

- 保留 PaperReader 的核心结果语义：`providerId`、`draftReady`、`attachmentReady`、`requiresSend`、`message`。Agentero 内部可以采用 Rust 类型，但 UI/迁移适配器需表达等价信息。
- 论文标题、页码、选区和上下文的拼接规则以 PaperReader 的既有体验为兼容基线；如要更改，应单独作为产品变更，不混入迁移实现。
- 复制网页回答到笔记等反向数据流必须由用户主动触发，显示将写入哪篇论文/哪个笔记位置，并遵循 Agentero 的笔记保存冲突保护。

### 3.5 连接器与项目操作

- 保留 PaperReader 中 ChatGPT 专属的 Codex-with-ChatGPT MCP 连接流程：prepare、connect、pair、status、disconnect；提供授权/连接状态和失败恢复。
- 在 Agentero 中优先复用现有 MCP Server 与 Secure MCP Tunnel，不重复实现一套隧道。网页 AI connector 状态应明确区别于本地 ACP session。
- 用户可将当前论文上下文准备为 ChatGPT 可读取的 MCP 资源/工具输入。授权范围只覆盖用户配置的当前 Vault/MCP 功能。
- 对话重命名、项目准备和项目创建是网页自动化能力，必须保留用户主动触发、支持性探测、结果校验和失败反馈；provider 不支持时隐藏或禁用对应动作。

## 4. Agentero 技术设计

### 4.1 模块边界

建议新增 `src-tauri/src/features/web_ai/`，按现有 feature-first 风格拆分：

```text
features/web_ai/
  mod.rs             feature 装配与状态
  models.rs          Provider、视图状态、转交结果、事件类型
  providers.rs       provider 注册表、URL/主机策略、能力声明
  controller.rs      WebView 生命周期、导航和活动 provider 管理
  view.rs            Tauri WebView 创建、显示、隐藏、定位
  bridge.rs          provider 页面注入脚本与受限事件桥
  conversations.rs  Vault/论文/provider 会话映射
  exports.rs         长文上下文和临时附件导出
  connector.rs       MCP Tunnel/Connector 薄适配
  commands.rs        Tauri commands
```

具体文件可以结合 Tauri 版本与现有窗口架构调整，但职责边界必须保持：Rust Host 掌控资源和策略；provider 页面脚本只负责站点 DOM 交互；React 负责交互呈现，不直接访问浏览器 Cookie 或任意文件路径。

### 4.2 WebView 承载

- 使用 Rust/Tauri WebView 生命周期管理器承载 provider 网站，可采用 Tauri 子 WebView；如果目标平台/API 不适用，可使用独立 WebviewWindow 作为平台回退。UI 通过统一 controller 抽象，不依赖具体窗口类型。
- 子 WebView API 的 `unstable` feature、跨平台能力及目标 Tauri 版本需在实现前验证；不得为追求单窗口而绕过平台 API 或造成启动/关闭死锁。
- Provider WebView 使用独立且持久的数据目录，确保登录隔离；提供版本化清理/迁移机制，删除应用数据前由用户明确操作。
- 阅读工作区布局变化通过 Rust `set_bounds`/显示状态同步；在 PDF 缩放、侧栏开关、窗口 resize、DPI 变化时保持几何同步。
- 页面外部链接默认交由系统浏览器打开；仅允许注册表中定义的 provider 主机和必要的登录/静态资源域名继续留在嵌入式视图。

### 4.3 Rust 与页面脚本的职责

**Rust Host 负责：**

- provider 注册表、导航白名单、活动视图身份和权限校验；
- WebView 创建/销毁、bounds、显示隐藏、生命周期和平台差异；
- 会话绑定、Vault 作用域、临时文件生成与清理；
- MCP Tunnel / connector 生命周期调用；
- 所有 command 的输入校验、错误类型、状态事件和审计级脱敏日志。

**页面初始化脚本负责：**

- provider 特有的输入框查找与文本插入；
- 粘贴/上传动作触发、附件存在确认、回答复制监听；
- 受支持的对话重命名和项目操作；
- 页面标题、当前 URL、加载/登录线索等必要状态探测。

页面脚本应按 provider 和版本集中维护，selector/capability 不散落在 React 组件中。DOM 变更导致操作无法确认时必须返回明确失败，禁止静默点错控件或发送消息。

### 4.4 安全桥

- 页面桥只开放窄范围事件/动作，例如 `web_ai_page_event`；所有请求都校验当前 WebView 实例、provider ID、当前 URL、动作类型和参数大小。
- 远程网页不得获得 Vault 任意读写、Agent ACP 执行、命令执行、设置读写、通用 `invoke` 或 MCP 管理权限。
- 不将 Rust command 全面授权给 provider 远程 origin。若 Tauri capability 不能做到精确的远程页面限制，应通过 Host 侧隔离通道和单一受审查入口实现；不能假设本地页面的权限配置会自动覆盖远程 WebView。
- 对导航、弹窗、新窗口、文件下载、剪贴板、粘贴、文件选择器分别设置策略。导航离开当前 provider 时应阻止或交由外部浏览器，并通知 Host。
- 不记录 Cookie、Authorization、整页 DOM、论文全文、完整提示词或用户回答。日志只记 provider、动作、耗时、结果类别与可操作错误码。
- 导出/暂存文件限制在 Agentero 管理目录；校验规范化路径、拒绝路径穿越/符号链接逃逸、限制大小和生命周期，任务结束后清理。

## 5. Agentero 命令和事件草案

名称可按代码风格调整，语义须覆盖以下能力。Tauri 命令全部由 Rust 校验，不由 React 自行模拟浏览器操作。

| Command | 作用 |
|---|---|
| `web_ai_providers` | 返回已配置 provider 与能力，不含 Cookie/凭证 |
| `web_ai_status` | 返回视图、导航、登录线索及当前会话绑定状态 |
| `web_ai_open` / `web_ai_close` | 打开/关闭指定 provider 视图 |
| `web_ai_view` / `web_ai_set_bounds` | 切换可见性、调整嵌入视图范围 |
| `web_ai_bind_conversation` / `web_ai_unbind_conversation` | 绑定/解除 Vault 论文会话 |
| `web_ai_transfer_text` | 准备文本草稿，不发送 |
| `web_ai_transfer_image` | 准备图片/截图附件并确认状态 |
| `web_ai_transfer_pdf` | 准备 PDF 附件或返回需用户选择文件的回退状态 |
| `web_ai_prepare_context` | 准备较长论文上下文/可引用资源 |
| `web_ai_copy_to_notes` | 用户确认后把选定回答写入指定笔记 |
| `web_ai_conversation_rename` | 请求 provider 支持的对话改名并校验结果 |
| `web_ai_project_prepare` / `web_ai_project_create` | 项目流程能力探测及用户触发的项目操作 |
| `web_ai_connector_status` / `start` / `pair` / `disconnect` | 复用 Agentero MCP Tunnel/Connector 生命周期 |

建议的统一材料结果：

```ts
type WebAiTransferResult = {
  providerId: string;
  draftReady: boolean;
  attachmentReady: boolean;
  requiresSend: true;
  message?: string;
  paperId?: string;
  page?: number;
};
```

事件建议：

| Event | 内容 |
|---|---|
| `web-ai:state` | provider、视图状态、当前论文/会话绑定及能力状态 |
| `web-ai:navigation` | 经校验的 URL/主机变化、被阻止的导航原因 |
| `web-ai:selection` | 页面中用户主动复制/选择的内容摘要或明确授权的文本 |
| `web-ai:transfer` | 草稿/附件准备开始、成功、失败及结果字段 |
| `web-ai:copy` | 用户触发复制网页回答后的待确认回传事件 |

事件 payload 不应含 Cookie、认证头或未授权的完整页面内容。

## 6. 数据与生命周期

- 会话绑定按 Vault、稳定 paper ID、provider ID 存储；与其他会话数据一样遵循 Agentero 本地优先原则。绑定更新在 WebView 确认打开目标会话后提交，避免错误 URL 污染映射。
- provider 的 WebView 数据目录按 provider 隔离、持久保存；UI 清理浏览数据前说明会退出登录。
- 临时上下文/PDF/图片材料放在应用管理的暂存目录，生成内容可追踪、可清理、不可被 provider 页面任意枚举。
- WebView 关闭、provider 切换、Vault 关闭和应用退出时释放页面桥、监听器、临时资源；Vault 关闭不得意外销毁跨 Vault 登录态。
- paper 重命名/移动时，使用 catalog paper ID 和 Vault 相对标识维护绑定，不以旧绝对路径为唯一事实源。

## 7. PaperReader 到 Agentero 的能力映射

| PaperReader 能力 | Agentero 承接方式 | 兼容要求 |
|---|---|---|
| Electron BrowserView `open/view/bounds/hide` | Rust WebView controller 与 React 布局同步 | 对应动作含义、当前 provider 行为不变 |
| provider 登录与持久 partition | 按 provider 隔离的 Tauri WebView 数据目录 | 切换 provider 不共享或丢失登录态 |
| 文本转交 `insertText` | provider 页面桥定位输入框并插入文本 | 仅准备草稿，不发送 |
| 图片/截图通过剪贴板粘贴 | 平台剪贴板/页面粘贴适配器 | 以页面附件可见为成功条件 |
| PDF 通过 CDP 文件 input 设置 | 按平台的 WebView 文件选择/上传适配器 | Tauri 无 Electron CDP 等价 API；逐平台验证，必要时用用户文件选择器回退 |
| 长文上下文导出 | Agentero scratch/context 文件或现有 Vault/MCP 能力 | 保留文档范围和用户可见的准备结果 |
| 论文-会话 JSON 映射 | Agentero Vault/catalog 作用域持久化 | provider 隔离、paper 稳定 ID、重启恢复 |
| ChatGPT `codex-with-chatgpt` MCP Tunnel | 复用 Agentero MCP server + Secure MCP Tunnel | 不复制第二套隧道，不改变现有 Connector 权限流程 |
| 对话重命名/项目操作 DOM 自动化 | provider bridge 中按能力启用 | 触发前做能力判断，动作后验证，不支持即明确反馈 |
| 复制网页回答到笔记 | 用户主动确认 + Agentero 笔记写入入口 | 遵循现有冲突检测和保存状态 |

适配器必须保留 PaperReader IPC 的外部语义与成功后的会话行为。Agentero 不要求照搬 Electron API 命名；但迁移层测试需覆盖旧动作到新命令的映射，确保 UI 与现有调用者不会因命名/状态字段改动产生行为差异。

## 8. 验收与回归测试

### 功能验收

- 每个支持的 provider 可打开、显示、隐藏、恢复其独立网页登录状态；窗口 resize、面板折叠和 DPI 变化后视图 bounds 正确。
- 为论文打开 provider 后能够绑定当前会话；关闭并重启应用后可以恢复该论文/provider 的会话；不同论文和 provider 不串线。
- 转交文本、选区、截图/PDF 后均明确区分准备中、成功、失败。所有材料操作都不会自动点击发送。
- 网页输入框选择器失效、用户未登录、页面导航被拦截、网络失败时均可理解并重试；失败不能丢掉待转交选区。
- ChatGPT MCP Connector 可查看状态、启动、配对、断开；复用 Agentero Tunnel，不影响既有 MCP 客户端。
- 回传网页回答到笔记需显式确认目标论文/笔记，写入遵守磁盘冲突保护。
- 对话改名/项目功能只在支持 provider 上操作，并验证最终结果；失败时返回站点错误/操作未确认状态。

### 安全与兼容验收

- provider 登录 Cookie 不跨 provider；远程页面不能调用 Agent 执行、Vault 任意文件或设置权限。
- 非允许主机导航、新窗口、文件下载、异常大事件 payload 均被阻止或显式降级。
- 日志中不出现 Cookie、Authorization、完整提示词、论文全文或网页回答。
- 验证 Agentero 普通 `agentero-web` 论文页面代理、ACP Agent、现有 MCP Tunnel/Server 行为无回归。
- 对照 PaperReader 回归：provider 打开与定位、材料转交、截图、PDF、选段、长文上下文、connector 授权、会话改名、项目操作和论文会话映射。
- 不以 mock 测试替代浏览器行为验收。provider DOM 操作使用 fixture 测试稳态逻辑，再对真实服务做受控 smoke test；真实测试不得自动发送真实消息。

### 工程验证

- Rust 单元/集成测试：注册表、origin 策略、会话绑定、Host bridge 身份校验、临时文件路径与清理。
- 前端：provider 状态、转交结果、取消/失败/重试、Vault/paper 切换。
- Windows、macOS、Linux 分别验证 WebView 创建、持久目录、剪贴板、bounds、文件选择器/PDF 上传；若 PDF 上传存在平台缺口，验收结果明确记录并提供手动回退。
- 运行 `cargo test` 相关 feature、`pnpm lint`、TypeScript 检查、`pnpm build` 和 Tauri 构建检查。

## 9. 分阶段实施建议

1. **P0：契约与能力盘点**——把 PaperReader provider/actions/payloads/selectors 做成清单；验证目标平台的 Tauri 子 WebView、持久目录、远程页面桥及窗口 bounds API；建立兼容测试夹具。
2. **P1：Rust 宿主和 provider 工作区**——新增 provider registry、WebView controller、open/view/bounds/close、导航限制、持久登录与状态事件。
3. **P2：阅读上下文与草稿转交**——接入 PDF 选区、文本/图片/截图准备，完善 Pending Context、成功确认、失败重试；验证不自动发送。
4. **P3：会话绑定与上下文导出**——按 Vault/paper/provider 恢复会话，实现长文导出、回答复制到笔记及确认流程。
5. **P4：MCP Connector、项目与对话操作**——复用 Agentero MCP Tunnel；接入 provider 特定的配对、重命名、项目准备/创建。
6. **P5：PDF 附件与平台收尾**——实现或评估各平台上传桥；逐平台做真实 WebView 回归、可访问性、错误态和性能收尾。

P0 应先产出平台能力实测表。尤其不能将 Electron CDP 的 `DOM.setFileInputFiles` 假定为 Tauri WebView 的跨平台能力；若无可靠支持，PDF 功能应先使用用户主动文件选择流程，后续再逐平台增强。

## 10. 未决技术问题与风险

1. **远程页面与 Rust Host 的安全事件通道：**需选定在 Tauri 2 下能按活动 WebView/URL/provider 身份强校验的桥接方式。Tauri 的远程 origin capability 不可未经验证地授予通用 invoke。
2. **Tauri 子 WebView 稳定性：**子 WebView API 可能受 `unstable` feature 和目标平台限制；需要确认 Windows/macOS/Linux 的创建、销毁、定位及焦点行为。
3. **PDF 上传：**Electron CDP 能直接设置文件 input，Tauri 没有天然等价的跨平台公共 API。不同 WebView 引擎需要独立 spike，或者采用用户选择文件的 MVP 回退。
4. **站点 DOM 漂移：**登录页、输入框、附件上传、项目/改名入口经常变更。选择器需带能力版本、fixture 和失败可观测性；不得把站点结构当稳定 API。
5. **剪贴板权限：**截图和图片转交需要平台权限与用户预期；需检查是否会覆盖用户剪贴板，并在实现中选择恢复原内容或改用安全临时附件路径。
6. **MCP Connector 的边界：**Agentero 已有 ChatGPT MCP Tunnel；需要确认当前 API 可否完整承接 PaperReader 的 `prepare/connect/pair/status/disconnect` 用户步骤，缺口仅以适配层补齐。
7. **provider 条款和用户确认：**网页 DOM 自动化需限定为用户主动操作和站点公开 UI，不做后台发送、验证码绕过或未授权账户自动化。

## 11. 完成定义

- 需求映射表中的 PaperReader 成功能力都有 Agentero 实现、明确的 MVP 降级或经产品确认的排除说明。
- 网页 AI 与 ACP Agent 的职责、存储、权限和状态清楚分开；Rust Host 是 WebView 生命周期与安全策略的权威入口。
- PaperReader 的动作/结果兼容测试通过，核心流程真实 smoke test 通过且未自动发送消息。
- Agentero 的现有论文代理、ACP、MCP Server/Tunnel、Vault 和笔记保存流程回归通过。
- 跨平台差异、PDF 上传能力与 provider DOM 限制已在对应功能文档中记录，用户可理解失败、重试或手动完成操作。

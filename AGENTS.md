# AGENTS.md

## 项目概览

Agentero 是一个基于 Tauri 2 + React 19 的本地优先科研工作台。Vault 中笔记与源文件以 Markdown/文件为准，论文结构化 metadata 以 `.agentero/catalog.sqlite` 为准。离开应用后数据仍可被外部工具读取。

架构总览：[docs/architecture.md](docs/architecture.md)

## 开发规则

- 优先做小而聚焦的改动，能用少的修改解决问题就不要增加复杂度。高内聚、低耦合。如果系统有额外冗余可以提示用户进行重构
- 只加必要的影响性能和功能的测试代码，不加没有实际意义的测试用例。
- 尽可能能复用能力，避免重复造轮子。
- 在 UI 修改的过程中尽量少加入无关的、非必须的i18n 文案，如果能用icon表达，就不要加额外的描述。操作失败用 `notifyError` Toast，不在侧栏 header 挂常驻错误条。
- 国际化（i18n）：所有面向用户文案必须经 `t()` 走 `react-i18next`。en 源语言 → 同步 `zh-CN`（`src/i18n/locales/`）。详见 [docs/frontend/shell.md](docs/frontend/shell.md)。
- 如果修改了 Template 下的 Skill 和 AGENTS.md ，需要对应更新版本号。
- 修改后需同步更新相关文档，并检查 Roadmap 和 Todo。
- 修改完成后，把当次相关的改动按照 commit 部分的要求提交。
- 每次完成代码修改后，必须根据本次修改内容补充工作记录（写明原因、改动、验证结果和已知限制），完成对应的 Conventional Commit，并完成项目应用编译；桌面应用修改还必须安装本次构建产物，核对安装注册版本、运行 exe 版本和启动日志后才能结束任务。
- 全文翻译修改必须同时检查 Agent/ACP 事件结果、chain 分段状态和 sidecar 写入边界；运行状态文本（如 `Retrying...` / `Retry finished, resuming.`）不得进入译文或 PDF 覆盖层。
- 论文翻译相关改动至少运行 `pnpm typecheck` 与 `pnpm vitest run test/pdf-layout-translate.test.ts test/pdf-layout-translate-agent-lifecycle.test.ts`；桌面发布前还要运行 `pnpm build`，并在安装后的应用日志中核对启动与翻译状态。
- `pnpm tauri build` 若因配置 updater 公钥但缺少 `TAURI_SIGNING_PRIVATE_KEY` 返回失败，需区分签名产物失败与 MSI/NSIS 生成结果；必须检查 `target/release/bundle/{msi,nsis}`、安装注册版本、运行 exe 版本和启动日志后再下结论。

## Windows 注意事项

- **路径**：禁止手拼 `${vault}/${rel}`，统一走 `joinVaultPath` 并归一分隔符；混用 `/` `\` 在 `\\?\` 前缀下报 ERROR_INVALID_NAME（#181），拖入文件路径也要归一后再去重。
- **新窗口**：同步 `#[tauri::command]` 里 build WebviewWindow 会在 WebView2 嵌套消息循环死锁（白屏、关不掉），必须用 async command。
- **窗口装饰**：统一用原生 decorations；React 自绘标题栏按钮在 Settings 等子窗口拿不到 IPC 权限。
- **子进程/安装**：Windows 只走 npm/PowerShell 安装路径，Unix 脚本用 `cfg(not(windows))` gate；注意 `CREATE_NO_WINDOW` 与 `.exe` 命令名。
- **cfg 门控**：Cargo target cfg（如 `cfg(all(unix, ...))`）必须与代码侧 gate 对齐，否则 Windows release 编译失败。
- **UI 缩放**：Windows 125% 等非整数 DPR 会放大 px/rem 混用误差，对齐类样式用 rem。

## Commit

- 提交信息必须符合 [Conventional Commits](https://www.conventionalcommits.org/) 规范。
- 一次提交只做一件事，避免混合多个 unrelated changes。
- 如果有多个 unrelated changes，`git commit --only -m "msg" -- <path1> <path2>` 使用只提交指定路径的改动
- commit message 不可以有emoji
- 如果库当中有对应的 issue，则在提交信息中引用（如 `Fix #123`）；解决了该 issue 则关闭；部分解决则在 issue 区评论。
- 解决 Issue 之后，把解决的方法评论在 issue 中

## GitHub

- 上游仓库是 `poco-ai/Agentero`，保留为 `origin`，不要直接推送。
- 本机开发提交推送到 fork：`https://github.com/Sonder4/Agentero`。远程名使用 `fork`。
- 推送命令是 `git push fork main`。没有 `fork` 远程时，先执行 `git remote add fork https://github.com/Sonder4/Agentero.git`。
- 不要把 `.cargo/`、`cmake/` 等本地工具目录提交或推送。

## 常用命令

```bash
pnpm install
pnpm dev
pnpm tauri dev
pnpm build
pnpm lint
pnpm format
pnpm tauri build
```

```bash
# Headless CLI 与共享基础 crate（仓库根 workspace）
cargo build -p agentero-cli
cargo run -p agentero-cli -- vault list --json
cargo test -p agentero-cli
cargo test -p agentero-core   # tauri 无关基础层（crates/agentero-core）
```

完成实现前运行最小必要验证。UI 改动优先启动应用检查对应流程。

## 文档地图

| 层 | 技术栈 | 关键能力 | 入口 |
|---|---|---|---|
| 前端 | React 19、TypeScript、Tailwind CSS 4、shadcn/ui、AI Elements | zustand 按域状态管理、Plate Markdown 编辑器、EmbedPDF 渲染、Dockview 多面板工作区 | [docs/frontend/](docs/frontend/index.md) |
| Host | Rust、Tauri 2、feature-first 布局 | 文件系统与 Vault 树、Catalog SQLite、Wiki 双链索引、ACP Client、本地文件监听、可选 loopback MCP | [docs/backend/](docs/backend/index.md) |
| CLI | Rust、`agentero` bin | headless Vault/Catalog 操作、论文标签管理、BibTeX 导入导出 | [docs/backend/cli.md](docs/backend/cli.md) |

| 分类 | 内容 | 路径 |
|---|---|---|
| **架构** | 整体架构、工作台布局、核心工作流、数据流 | [docs/architecture.md](docs/architecture.md) |
| **前端** | 壳、工作区、文件树、论文库、入库、Markdown、PDF、Agent、双链、翻译、设置 | [docs/frontend/](docs/frontend/index.md) |
| **后端** | 数据模型、Catalog、Vault、入库、Connector、Wiki、Agent、远程、搜索、日志、CLI、API | [docs/backend/](docs/backend/index.md) |
| **教程** | 快速上手、导入论文、阅读整理、接入 Agent、Zotero | [docs/usage/](docs/usage/index.md) |
| **开发** | 路线图、TODO、设计记录 | [docs/development/](docs/development/index.md) |
| **测试** | 测试策略、发布 Checklist | [docs/test/](docs/test/index.md) |
| **Bug 复盘** | 历史 Bug 分析（不删除） | [docs/bug_fix/](docs/bug_fix/) |

约定：已实现按功能写在 `docs/frontend/` / `docs/backend/`；未实现草稿在 `docs/development/`。

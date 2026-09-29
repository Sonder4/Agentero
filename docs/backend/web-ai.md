# Web AI Host

Agentero 的 Web AI 集成位于 `features/web_ai`，与普通论文网页代理 `features/web` 分离。Host 使用 Tauri 2 子 WebView 承载 ChatGPT、Gemini、DeepSeek、Kimi 和 GLM；子 WebView 创建失败时降级到独立 `WebviewWindow`。移动端只返回能力不可用提示。

## 数据与登录态

- Provider 登录数据由各自的 WebView profile 保存：`<Agentero data>/web-ai/profiles/<provider>/`。
- 论文与会话绑定保存在独立 `web_ai.sqlite`，只保存 provider、Vault key、稳定 paper id、会话 URL/id、项目 id、标题和更新时间。
- Cookie、token、聊天记录和论文正文不进入 Agentero 数据库、事件或日志。
- 附件先经过路径、符号链接、大小、文件头和 SHA-256 校验，再复制到按请求隔离的 scratch 目录；过期 scratch 目录由 TTL 清理。

## 页面桥

每个 WebView 使用随机 nonce 和 provider origin 白名单。Host 只注入有限的 `appendText`、打开文件入口和附件确认接口；页面事件必须同时通过 WebView provider、nonce、事件类型和 64 KiB 大小校验。页面不能调用 `__TAURI_INTERNALS__`，也不能继承主窗口的文件系统、Shell、Dialog、Vault 或 MCP 权限。

文本追加使用 provider registry 中的 composer 选择器，只追加草稿，并且要在输入框里读回这段文字才算 `draftReady`。ChatGPT 和 Gemini 的 PDF 由 Windows WebView2 的 `DOM.setFileInputFiles` 写入真实路径；脚本伪造的 `DataTransfer` 不再使用。`attachmentReady` 只在页面正文出现该文件名时为真。写入失败或 WebView 未打开时返回 `manualFile`，scratch 文件保留给用户手动选择；确认后才清理。DeepSeek、Kimi、GLM 不接收 PDF。结果里的 `requiresSend` 永远为 `true`，注入脚本不点击发送按钮。

PDF 选区菜单把当前选中文本和当前论文 PDF 一起交给已打开的 ChatGPT 或 Gemini；都没打开时默认 ChatGPT。它不自动绑定论文，也不自动发送。

`web_ai_copy_to_notes` 只在调用方已经确认目标论文后，把回答追加到 `{paper}/NOTES.md`，并保留已有 frontmatter。

## 尚未验收

Windows、macOS、Linux 上的真实登录态和五个 provider DOM 仍需人工 smoke。Windows 上 ChatGPT 与 Gemini 的 PDF 由 WebView2 写入，并以页面是否出现文件名确认；macOS 与 Linux 还没有这条文件通道。

## MCP Connector

`web_ai_connector_*` 复用现有 `McpTunnelController` 和 `mcp:tunnel-status` 生命周期，不保存 API key，也不创建第二套 tunnel。Connector 当前负责启动、状态投影、配对提示和显式断开；ChatGPT 页面中的最终 Connector 确认仍由用户完成。

## IPC

Rust commands 和 events 使用现有 Specta bindings。绑定、WebView 生命周期、文本/图片/PDF 转交、长上下文准备、项目操作和 Connector 都通过 `web_ai_*` 前缀命名。前端右侧栏只渲染 host 容器和操作栏，通过 `ResizeObserver` 更新物理像素 bounds。

## 验收范围

核心单元测试覆盖 provider URL、会话 URL 规范化、SQLite binding、PDF magic bytes、大小限制、符号链接拒绝、scratch 清理、桥 nonce/消息大小和 `requiresSend`。发布前仍需在 Windows WebView2、macOS WKWebView、Linux X11/Wayland 分别执行真实登录态、附件和回退 smoke test；远程 provider DOM 选择器会随页面更新，发布验收必须记录页面版本和失败回退路径。

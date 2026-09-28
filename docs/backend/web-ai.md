# Web AI Host

Agentero 的 Web AI 集成位于 `features/web_ai`，与普通论文网页代理 `features/web` 分离。Host 使用 Tauri 2 子 WebView 承载 ChatGPT、Gemini、DeepSeek、Kimi 和 GLM；子 WebView 创建失败时降级到独立 `WebviewWindow`。移动端只返回能力不可用提示。

## 数据与登录态

- Provider 登录数据由各自的 WebView profile 保存：`<Agentero data>/web-ai/profiles/<provider>/`。
- 论文与会话绑定保存在独立 `web_ai.sqlite`，只保存 provider、Vault key、稳定 paper id、会话 URL/id、项目 id、标题和更新时间。
- Cookie、token、聊天记录和论文正文不进入 Agentero 数据库、事件或日志。
- 附件先经过路径、符号链接、大小、文件头和 SHA-256 校验，再复制到按请求隔离的 scratch 目录；过期 scratch 目录由 TTL 清理。

## 页面桥

每个 WebView 使用随机 nonce 和 provider origin 白名单。Host 只注入有限的 `appendText` 与分块附件接口；页面事件必须同时通过 WebView provider、nonce、事件类型和 64 KiB 大小校验。页面不能调用 `__TAURI_INTERNALS__`，也不能继承主窗口的文件系统、Shell、Dialog、Vault 或 MCP 权限。

文本追加和附件准备都只修改 provider 的 composer，返回结果中的 `requiresSend` 永远为 `true`。Agentero 不调用远程页面的发送按钮。若 provider 拒绝脚本生成的文件事件，后续桥实现应发出手动选择文件回退状态。

## MCP Connector

`web_ai_connector_*` 复用现有 `McpTunnelController` 和 `mcp:tunnel-status` 生命周期，不保存 API key，也不创建第二套 tunnel。Connector 当前负责启动、状态投影、配对提示和显式断开；ChatGPT 页面中的最终 Connector 确认仍由用户完成。

## IPC

Rust commands 和 events 使用现有 Specta bindings。绑定、WebView 生命周期、文本/图片/PDF 转交、长上下文准备、项目操作和 Connector 都通过 `web_ai_*` 前缀命名。前端右侧栏只渲染 host 容器和操作栏，通过 `ResizeObserver` 更新物理像素 bounds。

## 验收范围

核心单元测试覆盖 provider URL、会话 URL 规范化、SQLite binding、PDF magic bytes、大小限制、符号链接拒绝、scratch 清理、桥 nonce/消息大小和 `requiresSend`。发布前仍需在 Windows WebView2、macOS WKWebView、Linux X11/Wayland 分别执行真实登录态、附件和回退 smoke test；远程 provider DOM 选择器会随页面更新，发布验收必须记录页面版本和失败回退路径。

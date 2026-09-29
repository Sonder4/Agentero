# 网页 AI 打开后，布局翻译术语表失败

## 范围与现象

主窗口打开 ChatGPT 或 Gemini 网页 AI 后，PDF 布局翻译在生成术语表时失败。日志为：

```text
layout translate glossary generation failed error=current webview is not a WebviewWindow
```

术语表这一步被跳过，后续翻译仍可能继续，但专有名词不再由 Agent 统一。普通 Agent 对话在同一时刻也会无法发起。

## 原因

网页 AI 是主窗口里的子 WebView。布局翻译的术语表通过 `agent_run_once` 调用 Agent，而该命令和 `agent_warm` 把调用方声明成 `WebviewWindow`。主窗口一旦挂了子 WebView，Tauri 就拒绝注入这个类型，并返回 `current webview is not a WebviewWindow`。

Agent 事件原先也按 `EventTarget::webview_window` 定向。前端监听的是 `getCurrentWebview()`，两端目标不一致时，子 WebView 存在还会让流式事件丢失。

## 修复

`agent_run_once` 与 `agent_warm` 改为接收 `Webview`，只保留 app handle 和 webview label。事件改为 `EventTarget::webview(label)`，与前端监听一致。Bridge 的 Agent RPC 同样按 webview label 找目标，不再要求独立窗口。

网页 AI 子页面因此不再阻断主窗口里的 Agent 调用，包括布局翻译术语表。

## 回归验证

- `cargo check -p agentero --offline` 通过。
- 发布构建已安装到本机。打开网页 AI 后再次生成布局翻译术语表，日志里不应再出现 `current webview is not a WebviewWindow`。

# 网页 AI 验收记录（2026-09-28）

## 范围

验收对象是提交 `6d977bdf` 上的桌面转交闭环，不是五个真实网站的全量登录验收。

## 已验证

- `cargo check -p agentero --offline` 通过。
- `pnpm exec tsc --noEmit --pretty false` 通过。
- `cargo test -p agentero --lib features::web_ai::bridge --offline`：3 passed。
- 桥测试确认 provider 选择器进入注入脚本，脚本不引用 `__TAURI_INTERNALS__`，也不包含 click 或 submit。
- `.src` 迁移的前置验收见 `2026-09-28-src-layout.md`。

## 未在本机执行

- 没有启动 Agentero 做 Windows WebView2 smoke，因此没有人工确认右侧栏打开、provider 切换、隐藏后恢复和选区草稿。
- 没有登录 ChatGPT、Gemini、DeepSeek、Kimi 或 GLM，也没有验证真实 composer 和 file input。
- 没有 macOS WKWebView、Linux X11 或 Linux Wayland 测试。
- 没有验证附件被站点拒绝后的手动选择对话框；当前只返回 `manualFile` 路径。

## 剩余产品缺口

- PDF 选区固定发给 ChatGPT，不带 vault id 和稳定 paper id。
- NOTES 工具栏和 PDF 工具栏没有独立转交入口。
- 打开 provider 不会自动恢复已绑定会话。
- 会话重命名和项目创建仍只改本地状态。
- 页面没有把 composer 或附件确认回传给 Host，所以脚本未报错不等于站点已接受内容。

## 结论

代码级转交闭环可以进入后续人工 smoke。发布前仍必须按 `docs/backend/web-ai.md` 的平台矩阵记录真实页面版本和失败回退路径。

# 网页 AI 转交进度（2026-09-28）

## 目标

把网页 AI 从通用骨架推进到可验证的桌面转交流程：provider 选择器进入页面桥，附件失败保留手动选择，确认后的回答可以写入 NOTES，PDF 选区可以准备草稿。

## 已完成

- `src-tauri/src/features/web_ai/providers.rs` 为 ChatGPT、Gemini、DeepSeek、Kimi、GLM 保存 composer 和附件选择器。
- `bridge.rs` 按 provider 注入这些选择器。注入脚本只追加文本和准备 file input，不包含 click 或 submit。
- 图片和 PDF 转交不再把脚本执行成功同时当成文本草稿和附件成功。失败或 WebView 未打开时返回 `manualFile`，并保留 scratch 文件。
- `web_ai_copy_to_notes` 通过现有 `write_notes(Append)` 写入 `{paper}/NOTES.md`，保留 frontmatter。空参数返回 false，不写文件。
- PDF 选区菜单增加 Web AI 动作。它准备选中文本，成功后打开右侧栏网页 AI 页；失败使用 `notifyError`。
- `docs/development/index.md` 不再把网页 AI 标成“实施未开始”。

## 验证

- `cargo check -p agentero --offline` 通过。
- `pnpm exec tsc --noEmit --pretty false` 通过。
- 桥单元测试断言选择器注入，以及脚本不包含 `__TAURI_INTERNALS__`、click 和 submit。

## 未完成

- 没有在真实 ChatGPT、Gemini、DeepSeek、Kimi、GLM 页面验证 composer 和 file input。
- 没有 Windows WebView smoke，也没有 macOS 或 Linux/Wayland 验收。
- 选区转交目前固定使用 ChatGPT，不携带 vault id 或稳定 paper id，因此不会自动绑定会话。
- NOTES 工具栏和 PDF 工具栏还没有独立入口。
- 会话重命名和项目创建仍只更新本地状态。
- 页面桥没有从远程页面回传 composer 或附件确认事件，所以 `attachmentReady` 仍表示脚本执行没有报错，不表示站点已接受文件。

## 下一步

在 Windows 上打开应用，验证面板打开、选区准备和失败提示。真实站点 DOM 变化时只更新 provider registry，不把选择器放进 React。

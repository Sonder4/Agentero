# Codex 交接进度（2026-09-28）

## 目标

接续两段因用量上限中断的 Codex 会话，先把工作区中已经落地但未提交的实现保存为可回溯基线，再继续完成验收和网页 AI 缺口。

来源会话：

- `01a0d1c5-a7e1-7671-b120-5ccb73540b84`：PDF 全文翻译缓存、并行、术语表、断点，以及论文单元 `.src` / `source` 分层。
- `01a0e378-b7a2-7250-b8e0-da0d316ec4e1`：ChatGPT、Gemini、DeepSeek、Kimi、GLM 的网页 AI Host。

两条会话都在 2026-09-28 03:30 左右结束，最后一条记录是用量上限错误，没有最终验收结论。

## 已落地

### 全文翻译

- 译文面板等待 `layoutTranslateCacheReady` 后才自动翻译。
- 原文/译文切换只改变显示状态，不删除已完成译文。
- 并发可选 `1/2/3/4/6/8`，默认 `2`；Rust 对 `pdf-layout-translate` 使用最多 8 路 semaphore。
- 并发大于 1 时不复用同一个 ACP session。
- 失败、空响应和结构损坏进入 `error`，不永久停留在 `running`。
- 可恢复错误最多自动重试 2 次；认证和配置错误不盲目重试。

### 论文单元

运行时结构：

```text
papers/<paper>/
├── .src/
│   ├── metadata.json
│   ├── layout-index.json
│   ├── layout-translate.json
│   ├── glossary.json
│   ├── state.json
│   └── citations.json
└── source/
    └── layout.json
```

Catalog 打开或重扫时把旧的根目录 `metadata.json`、`source/layout-index.json`、`source/layout-translate*.json` 和 `source/agentero-cite.json` 迁入 `.src`。目标冲突不覆盖，原始 `source/layout.json` 保留。

### 网页 AI 骨架

已有：

- `crates/agentero-core/src/features/web_ai/`：五个 provider、SQLite 绑定、附件校验。
- `src-tauri/src/features/web_ai/`：子 WebView、独立窗口回退、commands/events、页面桥。
- `src/components/web-ai/web-ai-panel.tsx`：右侧栏 provider 选择、打开和文本准备。
- `docs/backend/web-ai.md`：已实现行为说明。

仍未完成：

- 页面桥仍使用通用 `textarea` / `input[type=file]`，没有消费 provider registry 的选择器。
- 附件失败没有手动选择文件回退。
- 复制到笔记、项目创建和会话重命名只更新本地状态或返回布尔值。
- PDF 选区、当前页和 NOTES 还没有统一转交入口。
- 没有 fixture provider 集成测试。
- Windows/macOS/Linux 的真实登录态和附件 smoke test 未做。
- `docs/development/index.md` 仍写着“实施未开始”。

## 本次提交边界

工作区改动互相引用 `bindings.ts`、i18n、capabilities 和 feature 注册，拆成多个 commit 会留下不能编译的中间状态，因此本次作为一个基线提交。

明确排除：

- `paper-vault/`：本地论文库、PDF、笔记和 catalog，已加入 `.gitignore`。
- 密钥、Cookie、updater 私钥和构建产物。

## 验证

本阶段只做 git 基线，不把历史会话里的测试结果当作本次新验证。下一阶段运行 `.src` 迁移相关测试。

## 下一步

1. 运行 sidecar、layout、citation 和全文翻译测试，修复失败。
2. 补齐网页 AI 的 provider 选择器、附件回退和阅读器转交入口。
3. 在 Windows 上做面板 smoke test，并把无法在本机执行的 macOS/Linux 验收写入进度文档。

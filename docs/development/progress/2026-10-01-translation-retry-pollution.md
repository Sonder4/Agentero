# 2026-10-01 Agent 全文翻译污染修复

## 现象

PDF 双栏译文面板把以下 Agent 运行状态显示成正文：

- `Retrying (attempt 1/3, waiting 2s)...`
- `Retrying (attempt 2/3, waiting 4s)...`
- `Retrying (attempt 3/3, waiting 8s)...`
- `Retry finished, resuming.`

这些文本会重复出现在多个 region，并写入论文的 `source/layout-translate.json`。

## 根因

Agent ACP 完成事件的文本内容经过翻译链后，旧逻辑只在入口做清理；清理为空后，chain 分段仍使用清理前的 segment 设置为 `done`。随后流式 sidecar 写入逻辑只检查 `status === done` 和非空，没有再次检查内容是否为重试状态，因此污染文本被持久化并被 PDF overlay 渲染。

## 修复

涉及文件：

- `src/lib/pdf/layout/layout-translate.ts`
  - 在 `applyChain` 中对每个分段再次执行 `sanitizeAgentTranslationText`。
  - 清理后为空的分段改为 `error`，不再进入可见覆盖层。
  - `writeLayoutTranslateSidecar` 写入前再次清理翻译文本，并丢弃空结果。
  - 保留已有的旧污染 sidecar 拒绝逻辑。
- `docs/frontend/translate.md`
  - 补充前端全文翻译的防污染约束。
- `docs/backend/translate.md`
  - 补充 Agent 事件监听、超时、取消和验证边界。
- `AGENTS.md`
  - 增加本类问题的维护规则、验证命令和安装验证记录。

## 数据修复

当前 ViT 论文：

`E:\Desktop\workspace\论文\papers\ViT\paper-b5b6282779`

已将污染的 `source/layout-translate.json` 替换为已验证的完整 `.src/layout-translate.json` 缓存，并复核没有 `Retrying` 或 `Retry finished` 字样。

## 验证

```text
pnpm typecheck                                      PASS
pnpm vitest run test/pdf-layout-translate.test.ts \
  test/pdf-layout-translate-agent-lifecycle.test.ts PASS (27/27)
pnpm build                                         PASS
```

构建过程只产生已有 CSS pseudo-element、动态导入和大 chunk 警告，没有 Rust 或前端构建错误。

补充发布验证：

- `pnpm tauri build` 已完成 release 编译，并生成 MSI 与 NSIS 安装包；命令最后因配置了 updater 公钥但未提供 `TAURI_SIGNING_PRIVATE_KEY` 返回 exit code 1。该签名错误只影响 updater 签名文件，不影响安装包生成。
- 安装包：`target/release/bundle/msi/Agentero_0.11.4_x64_en-US.msi`、`target/release/bundle/nsis/Agentero_0.11.4_x64-setup.exe`。
- 已使用 NSIS 包更新 `D:\Agentero`，安装注册版本为 `0.11.4`，运行进程响应正常。安装后的 exe 为 `79,183,872` bytes，ProductVersion/FileVersion 均为 `0.11.4`。
- 安装后日志确认 `frontend_boot ok=true`、Vault `ensureVault ok=true`、PDFium worker 正常启动，bundle 为 `index-cJHuRU41.js`。
- 当前 ViT 污染 sidecar 扫描不到 `Retrying` / `Retry finished`；安装后主窗口截图也正常显示 PDF、译文面板和 Agent/Web AI 面板。
- 日志中的非本次修复问题：GitHub updater endpoint 网络错误；部分 `source` 与 `.src` 生成文件迁移冲突被保留；当前 OpenAI-compatible `grok-4.7` provider 返回 HTTP 503 `model_not_found`。这些不影响安装和主界面启动，但 provider 不可用时实时翻译请求会失败。

## 发布与安装记录

- 目标版本：`0.11.4`
- Windows 安装目录：`D:\Agentero`
- 安装前注册版本：`0.11.3`
- 安装后注册版本：`0.11.4`
- CLI 自报版本：`agentero 0.11.4`
- 应用日志：`%LOCALAPPDATA%\com.poco-ai.agentero\logs\agentero.log`

在重新安装包含本修复的桌面包后，应重新打开该论文并确认译文面板不出现重试日志；若旧任务仍在运行，先停止任务并清除当前页面译文，再重新翻译。

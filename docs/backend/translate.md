# 翻译（Host）

| Command | 说明 |
|---|---|
| `translate_text` | 免费 MT + 商用 BYOK + 内置 provider 路径（非文献 Translator） |
| `builtin_provider_status` | 内置 provider 的非秘密快照（`available` / `baseUrl` / 三个 model id）；见 [builtin-provider.md](builtin-provider.md) |

| 项 | 值 |
|---|---|
| 通用 `timeout_ms` | 可选；钳制 1s–30s；默认 30s |
| 商用 BYOK | DeepL / Azure / Google Cloud / OpenAI-compatible；`apiKey` 可由调用方传入，或由 Host 从 `settings.translate.providerConfigs` 解析（前端仅持有同长度 `*` 掩码） |
| 内置 provider | id `agentero`；凭证由**构建期**环境变量注入，用户在设置里选它即可，无需填 key / baseUrl / model。走 Hunyuan-MT，见下方 |
| OpenAI-compatible endpoint | 要求 Chat Completions 兼容：`POST {baseUrl}/chat/completions`，`Authorization: Bearer <key>`，请求体包含 `model`、两条 `messages` 与 `temperature`；解析 `choices[0].message.content`。设置里的 `baseUrl` 应是根地址（如 `https://api.openai.com/v1`），Host 会自动追加 `/chat/completions` |
| 配置边界 | `settings.translate.provider` 选择普通翻译和 PDF 全文翻译的翻译服务；选 `openai` 后，Host 从 `settings.translate.providerConfigs.openai` 读取 key / base URL / model。`settings.layout.parserBackend` 与 `settings.layout.providerConfigs` 只选择 PDF 版面解析/OCR 服务，不会覆盖翻译 provider。选择 `agent` 时则走 Agent/ACP 的 `translate.agentId`、`translate.modelId`，不会读取 OpenAI-compatible 模型配置 |
| OpenAI-compatible prompt | `openai_translate_messages`：学术译者 system prompt + 规则块（按意思重组语序、公式/符号/引用/`⟦n⟧` 占位符原样、术语一致、只输出译文、批量保留 `[[n]]`）；`temperature` 0.2。与前端 `buildTranslatePrompt` 保持同步。设置 `translate.customPrompt` 非空时（Host 在 `translate_text` 命令内注入 `custom_prompt`，WebView 调用方无感）替换 system message（`{{targetLang}}`/`{{sourceLang}}` 插值，映射与前端 `targetLangDisplayName` 一致）；`[[n]]` 批量规则与 `Text:` 原文仍由 Host 组装 |
| 密钥存储 | BYOK：明文写在用户本机 `settings.json`（Unix `0600`）；`settings_get` / 广播按字符 redact 为 `*`；`settings_set` 对纯 `*` 串 merge 保留原值。内置 provider 的 key **不落 `settings.json`**，编译期编入二进制、只在 Host 进程内使用（见 [builtin-provider.md](builtin-provider.md) §密钥边界） |
| 导入摘要 `free_mt_to_zh` | **并行竞速** 腾讯 / 火山 / DeepLX，取最先成功；单引擎 5s（`FREE_MT_ZH_TIMEOUT_MS`）；全失败则不写翻译。**内置 provider 不参与这条竞速**：`ZH_RACE_PROVIDERS` 只含免费引擎，导入摘要仍走非官方免费接口 |
| 设置页探测 | 前端 5s / 引擎；内置 provider 不参与探测，可用性直接来自 `builtin_provider_status` |

## 内置 provider（Hunyuan-MT）

`tencent/Hunyuan-MT-7B` 是专用 MT 模型，不是 instruct 模型，因此这条路径**不复用**上面的长规则 prompt，也**不给模型看 `[[n]]` 批量标记**（对齐协议依赖指令遵循）：

| 项 | 值 |
|---|---|
| 模板 | `Translate the following segment into <target_language>, without additional explanation.<source_text>`（逐字；指令与原文之间无空格无换行） |
| 消息 | 单条 user message，**无 system message**；`sourceLang` 不参与（模板没有它的位置，模型自动检测） |
| `[[n]]` | Host 侧按行首标记拆分（从 1 递增；行中或乱序即停止扫描）→ 每段一个请求 → `buffered(3)` 保序并发 → 按 `"{marker} {text}"` + `"\n\n"` 重组，与前端 `buildNumberedPayload` 字节一致。空段丢弃，段数变少时前端回退逐段翻译 |
| `⟦n⟧` | `mask.ts` 插入的行内占位符原样透传，不剥离（**未经真实 key 验证**） |
| 目标语言 | 只映射可达值：`zh-CN` → Chinese，`en` → English，防御性 `ui` → English，未知/`auto`/空 → English。上限由 `TR_TARGETS` 决定，模型侧的 37 语言见 [builtin-provider.md](builtin-provider.md) §支持语言 |
| 无 key | `commands.rs` 在任何 `.await` 前返回 `AppError::domain(ERR_NO_BUILTIN_KEY)`（`translate.no_builtin_key`），不放一个无法认证的请求出去 |
| 列表归属 | `"agentero"` 既不在 Rust `FREE_PROVIDERS`（CLI 用它门控 `--provider` 且以 `api_key: None` 调用）也不在 `COMMERCIAL_PROVIDERS`（驱动 WebView 凭证卡片）。**但前端 `FreeTranslateProviderId` / `FREE_MT_PROVIDER_IDS` 含它**——借此复用无 key 管线且不渲染凭证卡片；两份清单刻意相反，改一份要想到另一份 |
| 探测 | `probeFreeMtProviders` 显式过滤掉 `agentero`（探测它会真发一次翻译请求）；可用性只来自 `builtin_provider_status` |

实现：`crates/agentero-core/src/features/translate/sources/hunyuan_mt.rs`；凭证解析 `src-tauri/src/features/translate/commands.rs` + `src-tauri/src/features/system/builtin/`。限制与未决项见 [builtin-provider.md](builtin-provider.md) §限制与后续。

## Agent 全文翻译故障边界

Agent 全文翻译由前端 `runOnce` + `agent:completed` / `agent:failed` 事件驱动，不经 `translate_text`。监听器必须在 `runOnce` 前注册，并缓存尚未获得 `sessionId` 的完成/失败事件，避免事件竞态；单次请求有 180 秒超时，取消时必须终止运行并清理监听器。

Catalog 论文使用 `NOTES.md` 所属目录作为布局与 sidecar 根路径。松散 PDF 使用精确 PDF 路径查找源文件，sidecar 则统一放在文件名去掉 `.pdf` 后的目录；不能把组织目录当成论文目录扫描，否则可能选中同目录另一篇 PDF。`.src/metadata.json` 是论文元数据 sidecar，不能把 `.src` 当成论文目录。

排查 Agent 翻译失败时，日志需要关联记录 `workflow`、`session_id`、`agent_id`、请求配置的 `model_id`、ACP 实际 `provider/model`（若上游提供）、`stop_reason`、上游 HTTP 状态与错误码、最终 `output_chars`、耗时和业务结果（成功/空输出/超时/取消）。`accepted=true` 只表示请求被接收，不代表翻译成功。不得记录 API key、完整论文正文或其他凭证。特别是 Pi/ACP 上游返回不可达或空输出时，应将上游故障与应用层 `Agent returned an empty translation` 分别记录并通过 session id 关联。

ACP/provider 的重试提示不是译文。前端在事件结果、chain 分段和 sidecar 写入三个边界执行清理：纯重试提示被视为空结果并标记失败，sidecar 只保存非空清理结果。修改 Agent 事件协议或翻译渲染时，必须同时运行：

```bash
pnpm typecheck
pnpm vitest run test/pdf-layout-translate.test.ts test/pdf-layout-translate-agent-lifecycle.test.ts
```

## CLI 与桌面缓存契约

两端核对 provider、sourceLang、targetLang、serviceKey；CLI 不跨服务复用后改写来源标签。免费 MT 不使用自定义 prompt，其 serviceKey 不含 prompt 指纹；Agent / OpenAI / 内置模型仍按实际提示词区分。桌面把 CLI 缓存的原文经相同的断词、ligature 与页眉清理后比较，并核对 region ID 和 pageIndex，改变正文或语言仍然不会命中。

误识别为 header 的单字符数学重音碎片（如 `b ¯`）保留 PDF 原文，不调用翻译服务。CLI 在单篇 `skipped` 与 `skippedRegions` 中报告，并在 sidecar 的 `skippedRegions` 保存区域 ID、页号、原文和 `math-accent-fragment` 原因；成功译文统计不包含这些区域。顶层 CLI `skipped` 仍指跳过的论文数，不能与单篇区域数混淆。

MinerU 轮询保留 600 秒总预算，同时连续 180 秒状态/页数无进展即失败，单次 poll 不超过 30 秒及剩余预算。错误沿已有等待任务路径显示 notifyError，用户可以取消等待、重试或改用其他版面 provider。已有布局与翻译缓存不要求再次请求 MinerU。

# 开发草稿

本目录只放：

1. **工程规划**（版本切片、发布）
2. **尚未实现**的功能设计稿

已实现功能的说明在 [`../frontend/`](../frontend/index.md) 与 [`../backend/`](../backend/index.md)，按功能分篇。

## 未实现草稿

| 文档 | 主题 |
|---|---|
| [lifecycle-events.md](lifecycle-events.md) | 生命周期事件系统：语义事件 + 注册式 handler，统一事件命名规范。**已落地**，保留为设计记录（scoped 事件与 teardown 语义、为何不需要 `vault:closed`、为何 `app:will-quit` 延后） |
| [plaza.md](plaza.md) | 广场（Cool Papers / ModelScope 论文 / 推荐 / 播客）。**壳 + 两个站点来源浏览与入库已落地**（`agentero-coolpapers` / `agentero-modelscope` 站点代理）；推荐 / 播客仍为草稿 |
| [plaza-feeds.md](plaza-feeds.md) | 广场订阅 MVP：本地 RSS/Atom 时间线 + 论文入库。**已落地** |
| [usage-analytics.md](usage-analytics.md) | \#239 Activity 总线。P0 存储已落地：XDG `usage.sqlite`（见 [../backend/usage.md](../backend/usage.md)） |
| [zotero-word-integration.md](zotero-word-integration.md) | 官方 Zotero Word 插件 provider 兼容、文档迁移与平台实现评估 |
| [cloud-sync-s3.md](cloud-sync-s3.md) | 云同步（S3 兼容）：catalog sidecar 化前置 + 内容寻址 blob/manifest/CAS 同步引擎。**Phase 0–1 已落地**（见 [../backend/sync.md](../backend/sync.md)）；自动同步 / GC / E2EE / 官方托管仍为草稿 |
| [mark-cli-roadmap.md](mark-cli-roadmap.md) | \#170 阅读标注**内置进 CLI**（方案/命令面/边界）+ 基础→上层→Skill；与 [CLI 文档](../backend/cli.md) 分发衔接 |
| [mark-locate-lazy.md](mark-locate-lazy.md) | 文字定位：打开 PDF 再算（惰性，默认主路径） |
| [import-api-abstraction.md](import-api-abstraction.md) | Import 学术 API 抽象层：统一论文元数据、期刊指标、PDF URL 与题录批处理的 trait 与数据结构。**已落地**（`crates/agentero-core` 的 `features/paper/scholar_api/`），保留为设计记录 + 论文元数据重构的剩余缺口清单（`citation_count` 入库断点、`status`/`body_source` enum 化代价、`PaperTag` 生成契约） |
| [mark-locate-eager.md](mark-locate-eager.md) | 文字定位：标注时算（即时 B1 viewer / 可选 B2 headless） |
| [crate-split-roadmap.md](crate-split-roadmap.md) | agentero-core crate 拆分路线：Phase 1/2 已落地（基座 + CLI 数据域迁入、`HostHooks` 事件抽象、CLI 脱离 `agentero_lib`）；留守域耦合清单与 Phase 3 建议 |
| [agent-dx.md](agent-dx.md) | Agent DX：ops 自省 / MCP 对齐 / Skill 削薄。Phase 2 + 精简 3 + 4 已落地；dry-run 与更多 MCP tools 仍为后续 |
| [rust-arch-refactor.md](rust-arch-refactor.md) | Rust 架构重构计划（2026-09-08 更新）：共享业务用例、提交与恢复、Vault 派生状态、论文准备/共享提取、任务生命周期、Agent 上下文六条主线；含前置正确性修复、依赖与验收、旧任务归并及历史裁决。实施未开始 |
| [zhihu.md](zhihu.md) | 知乎开放平台集成：统一 CLI 后端（探测/一键安装/Keychain 凭证）+ 知乎虚拟 Agent（builtin ACP 适配 + 直答大脑）+ 广场知乎面板 + 论文找讨论 / 划词插引用。设计稿，实施未开始 |
| [A2A.md](A2A.md) | 本机桌面与远端 headless 之间的薄 A2A 任务层：Agent Card + Task，把算力和数据留在服务器。设计草稿，实施未开始 |
| [paperreader-web-ai-integration.md](paperreader-web-ai-integration.md) | 网页 AI Host 骨架已落地（见 [../backend/web-ai.md](../backend/web-ai.md)）；ChatGPT / Gemini 的 Windows PDF 附件已接入，真实登录态与 macOS / Linux 仍未验收 |

macOS 签名与公证（已实现流程说明）在 [`../bug_fix/macos-signing.md`](../bug_fix/macos-signing.md)。

本目录没有单独的 Roadmap / TODO 文件：未决项按能力记在对应已实现文档的「限制与后续」里。内置 provider（翻译 / embedding / 正文 OCR）的未决清单——`⟦n⟧` 占位符透传待真实 key 验证、Hunyuan-MT 模板空格待 A/B、目标语言从 en/zh-CN 扩到 37 种、把内嵌 key 换成网关签发的 per-install activation token——在 [`../backend/builtin-provider.md`](../backend/builtin-provider.md) §限制与后续；arXiv 推荐的 `arxiv_rec_state` 不按 model 建键一条在 [`plaza.md`](plaza.md) §3.4。

# PDF 首页面上方空白与双栏翻译无进展

## 现象与证据

原文 PDF 页框上方有较大空白，只读译文面板的页框却贴近顶部。点击全文翻译会打开译文标签，但仍显示英文，没有等待或失败提示。

本机 0.11.4 日志（`app_log_dir/logs/agentero.log`）显示：

- 2026-10-02 21:38:57 开始 `layout_remote_analyze_pdf`，21:49:01 以 `MinerU task timed out` 失败（604023 ms）。这次没有进入 `translate_text`，阻塞点在翻译前的版面解析。
- 更早 07:55–07:56 出现把 Adam 的 `.pdf` 文件作为论文目录读取的 `paper folder not found` / Windows error 267。工作区已有目录解析修复；当前安装应用需重新构建更新后才能包含工作区修复。
- 01:08–01:10 的另一轮 `translate_text` 已实际发出请求，但 OpenAI-compatible 服务多数请求在约 30 秒失败。这与后来没有翻译请求的解析阻塞是不同阶段的故障。

## 根因与修复

`DockviewViewport` 用 `padding` 四向简写预留半个批注栏宽度（236 / 2 = 118 CSS px），纵向也被扩大。在 125% DPR 下会表现为更明显的顶部空白。gutter 改为只增加左右 padding，上下仍为 viewport gap。

精简译文面板的自动启动 effect 原先要求已有版面 regions。没有 regions 时既不调用翻译 hook 的等待流程，也不观察解析失败。它还只依赖打开时复制的版面结果，源面板稍后完成的结果不会自动传入。

译文面板现在缓存读取后就启动或进入等待，通过源文档 key 持续读取版面结果。它向现有 viewer 注册表暴露翻译状态和 toggle，原文工具栏显示实际任务状态并控制同一个任务。复用译文标签使用与创建时一致的 tab id。

等待监听同时匹配 revision-stripped 源文档 id、译文 id 和论文路径。headless 结束时论文路径会被清空，故用上一个 running 快照归属失败。解析或入队失败显示错误 Toast 并解除等待；迟到的入队错误不能取消新一次重试。队列路径拼接使用 `joinVaultPath`，保留 Windows 扩展路径的分隔符；规范化 key 只用于去重。

## 验证与边界

运行 `pnpm typecheck`、`pnpm build`，以及 `pdf-layout-enqueue`、`pdf-layout-translate`、`pdf-layout-translate-agent-lifecycle`、`pdf-layout-translate-lifecycle`、`pdf-layout-translate-run`、`pdf-layout-translate-reliable` 回归（55 个测试）。测试覆盖源面板失败、headless 失败归属、入队失败与重试、Windows 扩展路径与并发入队失败通知，并保留 ACP 完成事件、chain 与 sidecar 写入边界的检查。

这些改动修复任务接线与反馈，不保证外部 MinerU 或翻译 API 可用。本次没有更改用户 provider 设置，也没有更新安装版；安装后仍须核对启动与真实翻译日志。Roadmap / TODO 按 `docs/development/index.md` 的约定分散在功能文档，服务可用性与真实桌面流程验证仍为验收边界。

# 论文单元 `.src` 分层验收（2026-09-28）

## 目标

确认全文翻译、Catalog、版面索引和引用缓存已经按论文单元的 `.src` / `source` 分层运行，旧文件只在迁移时移动，运行时不再回读旧路径。

基线提交：`22b65f0c`。

## 结果

验收通过，本阶段没有修改代码。

指定测试：

- `cargo test -p agentero-core --lib features::paper::catalog::sidecar`：4 passed。
- `cargo test -p agentero-core --lib features::pdf::layout_index`：5 passed。
- `pnpm exec vitest run test/pdf-layout-translate.test.ts test/pdf-layout-translate-lifecycle.test.ts test/pdf-layout-io.test.ts test/paper-metadata.test.ts test/pdf-citation-dest-keys.test.ts`：5 files，92 passed，3 skipped。

链接器有既有的 LNK4098 警告，不是失败。

## 已核对的行为

- 运行时文件位于 `papers/<paper>/.src/`：`metadata.json`、`layout-index.json`、`layout-translate.json`、`glossary.json`、`state.json`、`citations.json`。
- `source/layout.json` 仍是 raw layout，不在迁移清单中。
- Catalog 打开和 paper rescan 调用 `migrate_legacy_sidecars`。
- 六类旧文件一次迁入。目标已存在且内容不同时不覆盖，并保留旧文件；内容相同则删除旧文件。
- sidecar、layout index 和前端翻译读写没有旧路径回退。

## 未验证

- 远端 `remote_paper_rescan` 的 SFTP 迁移没有集成测试。写入成功后删除失败只记日志，旧文件可能留下。
- Connector import 仍用根目录 `metadata.json` 判断目录身份，不读取内容；真正搬迁依赖 remote rescan。
- 只有旧 metadata、没有 `NOTES.md` 的论文，要等 Catalog 打开或 rescan 后才被识别。
- 迁移失败不阻止 catalog 打开，之后运行时也不会回读旧文件。
- 没有跑 CLI `layout list`、完整 rescan 或真实 vault 端到端。
- `docs/development/crate-split-roadmap.md` 仍使用旧称 `agentero-cite.json`，运行时文件名已经是 `citations.json`。

## 下一步

进入网页 AI 转交闭环：provider 选择器接入页面桥、附件确认和手动回退、确认后写入 NOTES，以及 PDF/选区/NOTES 的显式转交入口。

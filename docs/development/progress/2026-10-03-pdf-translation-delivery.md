# PDF 与全文翻译修复交付（2026-10-03）

## 原因与改动

- 当前安装 exe 是更新服务的 0.11.5，卸载登记和仓库版本为 0.11.4；需要用本地完整安装包重新安装验证，不能靠版本字符串认定包含本地修复。
- 松散 PDF 不能按其组织目录枚举 PDF；精确源文件与 stem sidecar 对齐。验收统一用已入 catalog 的 `Adam-A-Method-for-Stochastic-Optimization`，用户原始 PDF 保留。
- 桌面自定义 prompt 指纹与 CLI 免费 MT 缓存不兼容，原文断词清理也导致缓存不命中；按实际服务能力生成 key，归一化原文后校验 region ID / pageIndex。
- CLI 跨 provider 缓存复用会改写来源，改为严格身份验证。
- 第 9 页 `p8-r127` 是数学重音碎片 `b ¯`，两端跳过；CLI outcome 与 sidecar 留存明确原因，不计入译文成功数。
- MinerU 连续 180 秒无进展终止；单次 poll 30 秒且不超过剩余总预算。
- 纳入既有工具栏可见进度和 chain/sidecar 清理修复；补充裸 `Retrying...` 清理，事件入口、chain 分段及写入边界共同检查。

## 已验证

- `pnpm typecheck`、三个翻译/sidecar 测试文件通过（后续改动后需复跑）。Rust 两个 cache / fragment 测试通过。
- 腾讯实时探测返回中文；目标 CLI 正常执行：255 translated，0 failed，单篇 skipped=1，`skippedRegions` 明确记录 `p8-r127`。本轮 CLI 复用同来源有效缓存。
- 初次前端 `pnpm build` 通过，存在既有 CSS highlight 与大 chunk 警告。

## 构建安装与界面验收

- `pnpm tauri build` 完成 release 编译，并生成 `target/release/bundle/msi/Agentero_0.11.4_x64_en-US.msi` 与 `target/release/bundle/nsis/Agentero_0.11.4_x64-setup.exe`。命令最后仅因配置了 updater 公钥但未提供 `TAURI_SIGNING_PRIVATE_KEY` 返回 1；MSI/NSIS 均已生成。
- 已用本地 NSIS 包安装到 `D:\Agentero` 并启动。卸载登记 `DisplayVersion=0.11.4`，运行文件 `D:\Agentero\agentero.exe` 的 `FileVersion/ProductVersion=0.11.4`，与源码和构建产物一致。
- 安装后日志确认 `frontend_boot ok=true`、Vault ensure、PDFium worker 正常启动。cua-driver 新鲜窗口快照显示 Adam PDF 已打开，工具栏有 `全文翻译 · 已翻译 252/253 · 失败 1 项`，页面覆盖层按钮为 `隐藏本页译文`；关闭并重新启动后 sidecar 仍被加载。
- CLI sidecar 的 255 条真实译文在桌面有效区域中覆盖 252/252；`p8-r127` 显示为 `math-accent-fragment` 跳过，不生成伪译文。界面仍可能保留一次历史网络请求失败 Toast（腾讯接口返回异常），该错误不会覆盖或清除已成功缓存的译文。
- PDF 原生页面的顶部页边距在渲染图中正常；阅读器布局修复已包含在本地安装版本，未观察到额外的阅读器顶部空白。

## 已知限制与 Roadmap / Todo

- `docs/development/index.md` 明确无独立 Roadmap / TODO；已核对 CLI roadmap 与翻译文档，后续限制在能力文档维护。
- 腾讯为非官方免费接口，可用性会变化；保留缓存并在失败时报告原因。未声称修复上游 grok-4.7 渠道配置或 MinerU 服务端。
- 工作区其他 Web AI 和视觉排版既有修改保留，未纳入本次提交；构建使用当前工作区，最终记录列明来源。

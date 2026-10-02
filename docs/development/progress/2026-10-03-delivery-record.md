# 2026-10-03 交付记录要求

## 请求

项目维护要求统一固化到 `AGENTS.md`：每次代码修改完成后，必须写工作记录、提交 Git、完成项目应用编译；桌面应用还要安装本次产物并验证安装版本、运行版本与启动日志。

## 改动

- 在 `AGENTS.md` 的开发规则中增加通用交付闭环。
- 本记录作为该规则的首次执行记录，记录文档规则本身的验证与安装结果。

## 验证

- `pnpm typecheck` 通过。
- `pnpm tauri build` 完成 release 编译，并生成 `target/release/bundle/msi/Agentero_0.11.4_x64_en-US.msi` 与 `target/release/bundle/nsis/Agentero_0.11.4_x64-setup.exe`；命令末尾因缺少 `TAURI_SIGNING_PRIVATE_KEY` 返回签名错误，MSI/NSIS 均已生成。
- 使用 NSIS 产物静默安装到现有 `D:\Agentero`，安装器返回 0；注册表 `DisplayVersion=0.11.4`、安装目录为 `D:\Agentero`，运行文件版本为 `0.11.4`。
- 通过 `cua-driver` 启动安装后的应用（PID 26376），启动日志记录 `frontend_boot ok=true`、PDFium worker 启动、Vault 打开成功和 `updater_check ok=true`。

## 限制

若 updater 公钥配置要求 `TAURI_SIGNING_PRIVATE_KEY` 而环境没有签名私钥，`pnpm tauri build` 可能在签名步骤失败；仍必须保留并检查已生成的 MSI/NSIS 产物，区分签名失败与安装包生成失败。

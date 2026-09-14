# Changelog

实际发行状态见 [版本树](docs/versions.md) 和 [GitHub Releases](https://github.com/SvenKunkka/OpenHardwareOS/releases)。
应用版本、规划节点和发布证据由 [版本清单](docs/versions.json) 管理。

## v0.1.2 · 2026-09-14 · Windows 预览版

修复公开发布链路的行尾与可追溯性问题，固定工具链，并把硬件支持计划随源码版本化。
**尚未发布**：条目先写在此处，公开发布后由版本工具登记版本树。

- 公开的 `SHA256SUMS` 改用 LF 行尾。此前它是 CRLF，`shasum -a 256 -c SHA256SUMS`
  在 macOS/Linux 上会把每一条记录都报成文件不存在（哈希值本身一直是对的）。
- 公开发布的 `install.ps1` 与仓库提交逐字节一致。此前它取自 Windows 检出，
  `core.autocrlf` 让它变成 CRLF，发布的摘要无法从仓库复现。打包脚本现在会
  归一化行尾、与提交内的 blob 比对，并在不一致时**拒绝出包**。
- 新增发布打包夹具测试 `scripts/release/test-packaging.ps1`（6 例）：资产集合、
  摘要逐条复核、CRLF 工作区、真正 CRLF 的提交、以及两个守卫本身是否有效；
  已接入 Windows 发布工作流。
- 新增 `.gitattributes`（`* text=auto eol=lf`），让检出内容等于提交内容。
- 新增 `rust-toolchain.toml` 固定 `1.98.1`，使检出处、本地与 CI 使用同一编译器；
  `rust-version = "1.95"` 仍是允许的最低版本。
- 硬件支持六阶段计划落成为 `docs/plans/hardware-support/`，并写明它与
  `C1–C8` 能力域、阶段小阶段编号、`PUMP-01–04` 工作包三套编号的关系。
- 文档订正：roadmap 不再声称"Windows 上什么都没跑过"（CI 自 v0.1.0 起在
  `windows-latest` 上构建、测试并安装）；Windows 验收文档区分"CI 已跑"
  与"真实机器已验收"；requirements 与验证记录补记首次 Windows CI 的两次失败
  （`wmi` 0.18 编译错误、只在 Windows 上失效的只读目录测试）及其根因。

## v0.1.1 · 2026-09-14 · Windows 预览版

- 新增可搜索、可展开、支持键盘操作的版本树；每个版本提供变更范围、源码、下载入口和验证记录。
- 增加版本清单检查、版本准备和真实发行记录工具，统一同步 Rust、桌面及安装验证的版本。
- Windows 安装与发布检查改为按指定版本运行，保留旧版本 tag 和安装文件。
- 修复渐变限制可能把风扇或水泵输出降到安全下限以下的问题。
- 修复热紧急请求达到目标值时仍可能被渐变限制的问题。
- 无效的当前占空比读数不再参与渐变计算；错误原因明确记录。
- 为 v0.2.0 登记水泵支持工作树和验收条件。真实水泵支持尚待验证。

- 验证通过：Windows 490 项 Rust 测试、前端 42 项、版本管理 30 项，以及公开 Windows PowerShell 5.1 安装验证。

[发行页](https://github.com/SvenKunkka/OpenHardwareOS/releases/tag/v0.1.1) ·
[源码提交](https://github.com/SvenKunkka/OpenHardwareOS/commit/b5ddc6da97c47ed1059618f4d654aeea4a92a478) ·
[CI](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34825301892) ·
[公开安装验证](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34826241020)

## v0.1.0 · 2026-09-14 · Windows 预览版

- 首次公开完整源码和历史记录，采用 Apache-2.0 许可证。
- 发布 Windows x64 CLI、NSIS 桌面安装包、PowerShell 安装入口和 SHA-256 校验文件。
- 包含依赖版权材料及指向源码提交的 release.json。
- 修复 Windows WMI 初始化和存储命名空间兼容性。
- 跨平台 CI 全部通过；Windows 487 项 Rust 测试、前端 42 项测试通过。
- Windows PowerShell 5.1 通过公开下载、安装、版本检查、诊断和模拟运行验证。
- 机箱风扇和水泵的真实硬件兼容性尚待目标设备验证。

[发行页](https://github.com/SvenKunkka/OpenHardwareOS/releases/tag/v0.1.0) ·
[源码提交](https://github.com/SvenKunkka/OpenHardwareOS/commit/e64fe2204154fb83cf42f6cd1f0854bd7bd0e4b0) ·
[CI](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34818882884) ·
[公开安装验证](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34819834091)

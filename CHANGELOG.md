# Changelog

实际发行状态见 [版本树](docs/versions.md) 和 [GitHub Releases](https://github.com/SvenKunkka/OpenHardwareOS/releases)。
应用版本、规划节点和发布证据由 [版本清单](docs/versions.json) 管理。

## v0.1.1 · 开发中

- 新增可搜索、可展开、支持键盘操作的版本树；每个版本提供变更范围、源码、下载入口和验证记录。
- 增加版本清单检查、版本准备和真实发行记录工具，统一同步 Rust、桌面及安装验证的版本。
- Windows 安装与发布检查改为按指定版本运行，保留旧版本 tag 和安装文件。
- 修复渐变限制可能把风扇或水泵输出降到安全下限以下的问题。
- 修复热紧急请求达到目标值时仍可能被渐变限制的问题。
- 无效的当前占空比读数不再参与渐变计算；错误原因明确记录。
- 为 v0.2.0 登记水泵支持工作树和验收条件。真实水泵支持尚待验证。

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

# Windows 命令行安装

公开源码：<https://github.com/SvenKunkka/OpenHardwareOS>。
首个 Windows x64 预览版本为 `v0.1.0`。预编译安装无需 Rust 或 Node.js。

## 安装 CLI

在 Windows PowerShell 中执行：

```powershell
Invoke-WebRequest 'https://github.com/SvenKunkka/OpenHardwareOS/releases/download/v0.1.0/install.ps1' -OutFile "$env:TEMP\OpenHardwareOS-install.ps1"
& "$env:TEMP\OpenHardwareOS-install.ps1" -Version v0.1.0
& "$env:LOCALAPPDATA\OpenHardwareOS\cli\ohm-cli.exe" --version
& "$env:LOCALAPPDATA\OpenHardwareOS\cli\ohm-cli.exe" doctor
```

脚本从该版本的 GitHub Release 下载 CLI 压缩包和 `SHA256SUMS`，确认摘要匹配后才解压安装。
CLI 安装在当前账户的 `%LOCALAPPDATA%\OpenHardwareOS\cli`，不需要管理员权限，
不修改系统 PATH，不启动硬件控制。`doctor` 用于查看设备和能力；安装成功不代表硬件兼容已经验证。

如需在当前 PowerShell 窗口直接输入 `ohm-cli`：

```powershell
$env:Path = "$env:LOCALAPPDATA\OpenHardwareOS\cli;$env:Path"
ohm-cli demo --steps 60
```

`demo` 使用模拟设备。上面的 PATH 设置只影响当前窗口。

## 同时安装桌面应用

```powershell
& "$env:TEMP\OpenHardwareOS-install.ps1" -Version v0.1.0 -Desktop
```

桌面安装包也会先核对 SHA-256。安装到所有用户时会请求管理员权限，安装后不会自动启动应用。
安装包未签名；如果 Windows 策略阻止运行，保留提示用于排查，保持现有安全设置。
桌面需要 WebView2，缺少时由安装程序提示并下载微软运行时。

主板传感器和机箱风扇支持取决于具体硬件及单独安装的 LibreHardwareMonitor；
GitHub 构建通过不等于真实风扇已通过验收。按
[实机验收入口](windows-validation/ACCEPTANCE-ENTRY.md)逐项核实。

## 从源码安装 CLI

已具备 Rust 1.95+ 和 Windows MSVC 构建环境的开发者，也可以执行：

```powershell
cargo install --git https://github.com/SvenKunkka/OpenHardwareOS --tag v0.1.0 --locked ohm-cli
ohm-cli --version
```

此命令只编译 CLI。桌面源码构建方式见仓库 README。

## 更新、卸载和排错

更新时下载目标版本的安装脚本，并把 `-Version` 换成该版本的完整 tag。
CLI 可通过删除 `%LOCALAPPDATA%\OpenHardwareOS\cli` 卸载；桌面使用 Windows 的“已安装的应用”卸载。
运行配置和审计记录位于另一个目录，卸载 CLI 不会删除它们。

失败时记录所用命令、完整错误输出、Windows 版本以及安装的 tag，在
[GitHub Issues](https://github.com/SvenKunkka/OpenHardwareOS/issues)反馈。
上传前检查日志中是否含个人信息或硬件标识；不要提交真实配置中的凭据。

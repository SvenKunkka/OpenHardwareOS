# Windows 命令行安装

公开源码：<https://github.com/SvenKunkka/OpenHardwareOS>。
本页安装 Windows x64 预览版本 `v0.1.4`。预编译安装无需 Rust 或 Node.js。

[查看全部版本](versions.md) · [交互版本树](https://svenkunkka.github.io/OpenHardwareOS/)

## 安装 CLI

在 Windows PowerShell 5.1 或更新版本中，完整粘贴下面的命令块。
它直接下载并校验固定版本的压缩包，不依赖运行本地 `.ps1` 文件，也不修改执行策略。

```powershell
& {
    $ErrorActionPreference = 'Stop'
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -and $env:PROCESSOR_ARCHITEW6432 -ne 'AMD64') { throw 'Windows x64 is required.' }
    $ohmVersion = 'v0.1.4'
    $ohmAsset = "ohm-cli-$ohmVersion-windows-x86_64.zip"
    $ohmUrl = "https://github.com/SvenKunkka/OpenHardwareOS/releases/download/$ohmVersion"
    $ohmInstall = Join-Path $env:LOCALAPPDATA "OpenHardwareOS\cli-$ohmVersion"
    if (Test-Path -LiteralPath $ohmInstall) { throw "Already exists; preserved: $ohmInstall" }
    $ohmDownload = Join-Path $env:TEMP ('ohm-download-' + [Guid]::NewGuid().ToString('N'))
    New-Item $ohmDownload -ItemType Directory | Out-Null
    $ohmZip = Join-Path $ohmDownload $ohmAsset
    Invoke-WebRequest -UseBasicParsing "$ohmUrl/$ohmAsset" -OutFile $ohmZip
    Invoke-WebRequest -UseBasicParsing "$ohmUrl/SHA256SUMS" -OutFile (Join-Path $ohmDownload 'SHA256SUMS')
    $ohmMatch = @(Get-Content (Join-Path $ohmDownload 'SHA256SUMS') | Where-Object { $_ -match ('^[0-9a-fA-F]{64}  ' + [Regex]::Escape($ohmAsset) + '$') })
    if ($ohmMatch.Count -ne 1) { throw 'Expected exactly one matching SHA256 entry.' }
    if ((Get-FileHash $ohmZip -Algorithm SHA256).Hash -ne $ohmMatch[0].Substring(0, 64)) { throw 'SHA256 mismatch; installation stopped.' }
    Expand-Archive -LiteralPath $ohmZip -DestinationPath $ohmInstall
    Remove-Item -LiteralPath $ohmDownload -Recurse -Force
    & (Join-Path $ohmInstall 'ohm-cli.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'Installed CLI could not start.' }
    & (Join-Path $ohmInstall 'ohm-cli.exe') doctor
    if ($LASTEXITCODE -ne 0) { throw 'Device diagnostic failed; keep the output for diagnosis.' }
}
```

命令精确匹配 `SHA256SUMS` 中的压缩包文件名，要求仅有一条记录，并在摘要一致后才解压。
从 `v0.1.3` 起，公开的 `SHA256SUMS` 使用 LF 行尾，因此在 macOS/Linux 上
`shasum -a 256 -c SHA256SUMS` 也能直接校验（`v0.1.0`、`v0.1.1` 的该文件是 CRLF：
哈希值本身正确，但 POSIX 工具会把每一条记录都报成找不到文件，需要先 `tr -d '\r'`）。
CLI 安装在当前账户的 `%LOCALAPPDATA%\OpenHardwareOS\cli-v0.1.4`，不需要管理员权限，
不修改 PATH，不启动硬件控制。目标目录已存在时会停止，保留原有文件。
`--version` 显示安装版本，`doctor` 查看设备和能力；安装成功不代表硬件兼容已经验证。

如需在当前 PowerShell 窗口直接输入 `ohm-cli`：

```powershell
$env:Path = "$env:LOCALAPPDATA\OpenHardwareOS\cli-v0.1.4;$env:Path"
ohm-cli demo --steps 60
```

`demo` 使用模拟设备。上面的 PATH 设置只影响当前窗口。

## 可选安装脚本及桌面应用

如果电脑现有策略允许运行本地 PowerShell 脚本，可使用自动更新安装器。
它使用独立的 `%LOCALAPPDATA%\OpenHardwareOS\cli` 目录，不会覆盖上面的版本目录。
加上 `-Desktop` 可同时安装桌面应用；仅需 CLI 自动安装时省略这个参数。

```powershell
Invoke-WebRequest -UseBasicParsing 'https://github.com/SvenKunkka/OpenHardwareOS/releases/download/v0.1.4/install.ps1' -OutFile "$env:TEMP\OpenHardwareOS-install.ps1"
& "$env:TEMP\OpenHardwareOS-install.ps1" -Version v0.1.4 -Desktop
```

桌面安装包也会先核对 SHA-256。安装到所有用户时会请求管理员权限，安装后不会自动启动应用。
脚本及安装包未签名；如果 Windows 策略阻止运行，保留提示用于排查，保持现有安全设置。
桌面需要 WebView2，缺少时由安装程序提示并下载微软运行时。

主板传感器和机箱风扇支持取决于具体硬件及单独安装的 LibreHardwareMonitor；
GitHub 构建通过不等于真实风扇已通过验收。按
[实机验收入口](windows-validation/ACCEPTANCE-ENTRY.md)逐项核实。

## 从源码安装 CLI

已具备 Rust 1.95+ 和 Windows MSVC 构建环境的开发者，也可以执行：

```powershell
cargo install --git https://github.com/SvenKunkka/OpenHardwareOS --tag v0.1.4 --locked ohm-cli
ohm-cli --version
```

此命令只编译 CLI。桌面源码构建方式见仓库 README。

## 更新、卸载和排错

按默认方式更新时，把命令块中的 `$ohmVersion` 改为目标版本的完整 tag，安装到新的版本目录。
CLI 可通过删除对应的 `%LOCALAPPDATA%\OpenHardwareOS\cli-v0.1.4` 目录卸载。
如使用可选脚本安装，则用该脚本的 `-Uninstall` 删除其管理的 `cli` 目录。
桌面使用 Windows 的“已安装的应用”卸载。
运行配置和审计记录位于另一个目录，卸载 CLI 不会删除它们。

失败时记录所用命令、完整错误输出、Windows 版本以及安装的 tag，在
[GitHub Issues](https://github.com/SvenKunkka/OpenHardwareOS/issues)反馈。
上传前检查日志中是否含个人信息或硬件标识；不要提交真实配置中的凭据。

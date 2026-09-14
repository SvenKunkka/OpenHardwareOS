# Windows 验收入口（一页）

本页只说明**现在有什么证据、源码包是哪一版、下一步做什么，以及失败时该交回什么**。
它不是验收结论：下面所有**真实硬件**结果都尚未发生。

---

## 1. 已经成立的证据

**本机（macOS，模拟 provider，不写真实硬件）**

| 证据 | 在哪里看 |
|---|---|
| Rust 全量测试、`clippy -D warnings`、`fmt --check`、`cargo deny check` | `docs/verification-log.md`，每条记录开头写明它跑的**提交**与结果 |
| 前端 typecheck / 测试 / 生产构建 | 同上 |
| 真实桌面 IPC 往返：真实窗口 + 真实 webview + 真实命令（compatibility notes、交接 pending→failed→confirmed、unconfirmed 写入） | 同上；脚本 `scripts/verify-ipc-roundtrip.sh` |
| 交付包脚本控制流（precheck / build / verify / 工具链传递 / 安装包判定） | `docs/windows-validation/package/scripts/tests/run-script-tests.ps1`（**只用测试替身**，不证明 Windows 能编译） |
| 打包器自身（不覆盖旧包、只从提交取文件、校验失败不产出） | `scripts/tests/make-acceptance-package.test.sh` |
| 发布打包（行尾、资产集合、摘要、两个守卫） | `scripts/release/test-packaging.ps1`（替身与夹具 git 仓库，不证明 Windows 能构建） |

**GitHub Actions 的 Windows runner（真实 Windows，但不是你的机器）**

| 证据 | 版本 | 链接 |
|---|---|---|
| Windows 全量测试 487 项、前端 42 项，全部 job 通过 | v0.1.0 | [run 34818882884](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34818882884) |
| Windows CLI 与桌面安装包构建、安装验证 | v0.1.0 | [run 34818882887](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34818882887) |
| 公开下载、校验、安装、诊断与模拟运行（PowerShell 5.1） | v0.1.0 | [run 34819834091](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34819834091) |
| Windows 全量测试 **490 项**、八个 job 全部通过 | v0.1.1 | [run 34825301892](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34825301892) |
| CLI 与 NSIS 安装包构建、13 项安装夹具、实际安装包验证 | v0.1.1 | [run 34825296140](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34825296140) |
| 公开下载、校验、安装、诊断与模拟运行（PowerShell 5.1） | v0.1.1 | [run 34826241020](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34826241020) |

这些 run 的**结论、提交与 job 明细**都在 GitHub 上可查，本页只引用，不复制数字。
它们证明"代码能在 Windows 上编译、测试、打包、安装"，**不证明**任何一台真实机器上的风扇、水泵、
主板传感器或显卡可被读取或控制，也**不是**你手上那台机器的运行记录。

## 2. 明确**没有**证据的部分

- **真实机器上的 Windows 运行**：WMI 存储（CI 上跑过，但对的是 runner 的虚拟盘）、`reg.exe` 自启、
  托盘、NVML、LibreHardwareMonitor 对真实 SuperIO 芯片、NSIS 安装/卸载与安装后的桌面应用——
  全部是 *Prepared*，从未在任何一台真实机器上执行。
- **真实风扇/水泵的物理响应**：从未有任何写入到达真实硬件；回读只证明 provider 报告的设定值，
  不等于空气流动或水在流动。需要转速计独立测量。这与 Windows 是**两项独立证据**。
- **桌面界面的视觉验收**：第五轮尝试过截图，做法是整屏捕获，结果无法使用（区域不含应用窗口，
  且画面包含与本项目无关的私人内容，已删除）。因此界面只由代码与前端自述佐证，**视觉未验收**。
- 许可证决定（ADR 0002 / 0004）仍是 **Proposed**。

## 3. 源码包版本与校验值

| 提交 | 目录 / 压缩包 | 清单文件数 | 压缩包 SHA-256 |
|---|---|---|---|
| `27ee667`（第三轮） | `dist/acceptance/OpenHardwareOS-27ee667-windows-acceptance{,.zip}` | 192 | `53f76a4fc64c738e96341c03c45ce30bccee96cc7a597ba8046014959a6fb21f`（**重建**，见下） |
| `5fd8c23`（第四轮） | `dist/acceptance/OpenHardwareOS-5fd8c23-windows-acceptance{,.zip}` | 201 | `fddcd1886ca96c1ed119b7b6b90202b2fb538cff0e82bb980dd503654ab05e4d` |
| `4a72112`（第五轮） | `dist/acceptance/OpenHardwareOS-4a72112-windows-acceptance{,.zip}` | 204 | `453aeb2028e7d1318e2a77ab8750d6d99b24cbeda658e19c0ff02ac4089de774`（清单文件本身：`da4d9dfebf2a0bb589560aa252e21ab3837b4edd531b1dd0567bb132149ab030`） |
| `499008c` | `dist/acceptance/OpenHardwareOS-499008c-windows-acceptance{,.zip}` | 204 | **请勿使用**：该包由有缺陷的打包器产出，旁边有 `.DO-NOT-USE.txt` |
| `2a0391d`（v0.1.2，第六轮） | `dist/acceptance/OpenHardwareOS-2a0391d-windows-acceptance{,.zip}` | 245 | `db1a875770026568d8662577767905ade22b84320ce9baedbd86f2155384aaaf`（清单文件本身：`67abd843e699daa5cbb5b0965e758e930e6c8d3885564ffae7244a47c4ec027b`） |

**你手上的这一包是哪一版**：看包根 `README-ACCEPTANCE.md` 顶部的 `Commit` 行（打包时写入），
摘要看与压缩包同目录的 `<同名>.zip.sha256`（第一行是压缩包，第二行是包内清单文件）。
包内无法写自己的摘要——那会改变它正在记录的哈希。

**关于第三轮那个包，必须说清楚**：它的原始 ZIP 是
`cb6ac7fe8eeba8be7fc3bc8244dffce817c70ed43620d1366bdf4c9cd2ec3d9c`，在第四轮打包过程中被我
`rm -rf dist/acceptance` 误删；**原始产物无法恢复**。现在同名的包是用同一提交 `27ee667` 重新
生成的（源码内容相同，ZIP 字节不同），校验值为上表所列。第四轮曾写过"未覆盖旧包"，那是不
准确的，第五轮记录已更正，并且打包器现在**拒绝**覆盖任何已存在的产物。

每个包都可用其自带的校验器核对（包内 `MANIFEST.sha256` 覆盖每一个文件）：

```powershell
.\scripts\verify-package.ps1
```

## 4. 首次验收：只读步骤（不写硬件、不安装）

在**解压后的包根目录**依次执行；任何一步失败就停下并交回日志，不要继续。

```powershell
# 1. 预检：平台、工具链（含 clippy 与 rustc 必须同一版本）、Node、空间、
#    包完整性、工作目录可写。失败即停，不会开始构建。
.\scripts\precheck.ps1

# 2. 校验包内容与清单一致（每一项逐一比对，任何多出/改动/缺失都会指名报错）。
.\scripts\verify-package.ps1

# 3. 只读取证：采集硬件、驱动、LHM 可达性、应用配置目录状态。
#    注意 -OutputRoot 必须指向包**外面**，否则取证会写进包里、破坏清单校验
#    （脚本自身也会拒绝在包内写入）。
.\docs\windows-validation\collect.ps1 -DryRun -OutputRoot "$env:USERPROFILE\OpenHardwareOS-evidence"
.\docs\windows-validation\collect.ps1 -OutputRoot "$env:USERPROFILE\OpenHardwareOS-evidence"
```

然后按 `docs/windows-validation/checklist.md` 的顺序执行。构建交由脚本完成，产物与证据都写在
包**旁边**的工作目录里（默认 `<包目录>-build`）：

```powershell
.\scripts\build.ps1                # 如需显式固定工具链：-Toolchain 1.98.1
```

仓库自 v0.1.2 起用 `rust-toolchain.toml` 固定 `1.98.1`（CI 用同一版本），所以**不传**
`-Toolchain` 时，包内检出的 rustup 代理也会选 1.98.1；如果这台机器上的 `cargo` 不是
rustup 代理（例如 Homebrew 装的），它不会读这个文件——那就显式传 `-Toolchain 1.98.1`，
预检也会检查 `clippy` 与 `rustc` 是否来自同一版本。

`build.ps1` 只在本次构建确实产生了 `tauri.conf.json` 所声明的 NSIS 安装包时才报 BUILD COMPLETE；
空目录、旧产物、别的版本都会以 INSTALLER MISSING / EMPTY / STALE / MISMATCH 明确失败。

**安装是单独的人工步骤**（需要管理员）：见 `checklist.md` §7.2。安装包未签名，会触发 SmartScreen
提示——记录原始提示文字，不要关闭保护。

## 5. 失败时请交回什么

1. `precheck.ps1` / `verify-package.ps1` / `build.ps1` 的**完整输出**（控制台文本或 `>\<文件>` 重定向）；
2. `<包目录>-build\evidence\<时间戳>\` 整个目录（每次运行各自一份）；
3. `collect.ps1` 生成的取证目录（"before" 与 "after" 各一份，含 `environment.txt` 与 LHM 探测结果）；
4. `%APPDATA%\OpenHardwareOS\logs\openhardwareos.log.<日期>`；
5. 失败的**原始命令与退出码**（不要只给结论），以及当时的 Windows 版本/构建号。

## 6. 分别还缺什么

| 环节 | 当前状态 | 还缺什么才能标记完成 |
|---|---|---|
| 构建（CI） | v0.1.0/v0.1.1 已在 `windows-latest` 上构建并发布 NSIS 安装包 | —— 已成立；它不等于真实机器上的构建 |
| 构建（你的机器） | 脚本与判定已就绪并被替身测试与夹具测试覆盖 | 一次真实 Windows 上的 `build.ps1`，产出 NSIS 安装包并记录路径/大小/SHA |
| 安装 / 卸载 | CI 只安装过 CLI（ZIP）；NSIS 安装包与桌面应用从未安装 | 管理员权限下安装、启动、卸载，记录安装目录与配置目录策略（`checklist.md` §7.2–7.3） |
| 真实风扇 / 水泵 | 零证据 | 一次真实写入 + 转速计（或可听/可测的转速变化）测量；与 Windows 运行是**两项独立证据** |
| 其他真实硬件 | Prepared | WMI 存储对真实盘、NVML 对真实驱动、LHM 对真实 SuperIO 芯片各自实测 |
| 界面视觉 | **未验收** | 人工查看真实窗口并留图（本机截图不可用，见 §2） |

## 7. 授权边界

本轮（准备 v0.1.2）只做本地源码修改、模拟/夹具验证与本地打包：未写真实风扇或任何硬件、
未改自启与全局环境、未触发远端 CI、未推送、未发布、未代批许可证。
`v0.1.0` / `v0.1.1` 的公开推送、tag 与 GitHub Release 是维护者**另一次**已完成的动作，不属于本轮，
也不构成本页任何硬件结论。Windows、真实风扇与安装包各自实测后才能标记完成。

# Linux 安装

公开源码：<https://github.com/SvenKunkka/OpenHardwareOS>。
本页安装 Linux x86_64 预览版本 `v0.1.4`，两种方式：**命令行（CLI）** 与 **桌面应用（.deb）**。
两者都是预编译产物，安装无需 Rust 或 Node.js。

[查看全部版本](versions.md) · [交互版本树](https://svenkunkka.github.io/OpenHardwareOS/)

## 安装 CLI

```bash
set -euo pipefail
ohm_version=v0.1.4
ohm_platform=linux-x86_64
ohm_asset="ohm-cli-$ohm_version-$ohm_platform.tar.gz"
ohm_meta="release-$ohm_platform.json"
# Linux 的校验和清单带平台后缀：同一个 Release 里 Windows 资产已经占用了
# 名字 `SHA256SUMS`（一个 Release 里同名资产只能有一个）。
ohm_sums="SHA256SUMS-$ohm_platform"
ohm_base="https://github.com/SvenKunkka/OpenHardwareOS/releases/download/$ohm_version"
ohm_dir="$HOME/.local/share/OpenHardwareOS/cli-$ohm_version"
test ! -e "$ohm_dir" || { echo "已存在，未改动：$ohm_dir" >&2; exit 1; }
ohm_tmp="$(mktemp -d)"
trap 'rm -rf "$ohm_tmp"' EXIT

# 用资产的真实文件名下载：`sha256sum -c` 是按文件名核对的，改名会让它找不到文件。
curl -fsSL -o "$ohm_tmp/$ohm_asset" "$ohm_base/$ohm_asset"
curl -fsSL -o "$ohm_tmp/$ohm_meta" "$ohm_base/$ohm_meta"
curl -fsSL -o "$ohm_tmp/$ohm_sums" "$ohm_base/$ohm_sums"
# 清单覆盖这一版**全部** Linux 产物；这里只核对正在安装的那一个。
# 恰好一条匹配记录：多一条、少一条都停下。然后按文件名核对摘要——
# 少下载别的产物不会让这一步失败，摘要有任何不符都会。
ohm_pattern="^[0-9a-f]{64}  $ohm_asset\$"
ohm_matches="$(grep -cE "$ohm_pattern" "$ohm_tmp/$ohm_sums" || true)"
test "$ohm_matches" = "1" || { echo "$ohm_sums 中应有且仅有一条匹配记录，实际 $ohm_matches 条" >&2; exit 1; }
grep -E "$ohm_pattern" "$ohm_tmp/$ohm_sums" > "$ohm_tmp/verify.sha256"
(cd "$ohm_tmp" && sha256sum -c verify.sha256)
mkdir -p "$ohm_dir"
tar -xzf "$ohm_tmp/$ohm_asset" -C "$ohm_dir"
"$ohm_dir/ohm-cli" --version
"$ohm_dir/ohm-cli" doctor
```

校验和清单先核对再解压；校验失败 `set -e` 会直接中止。清单只列**发布出来**的文件
（`LICENSE` 与依赖声明在压缩包内），并覆盖整版 Linux 产物：把四个文件都下载了的话，
`(cd <目录> && sha256sum -c SHA256SUMS-linux-x86_64)` 就是一次完整的核对。CLI 装在当前用户目录下，
不需要 root，也不修改 `PATH`。目标目录已存在时会停止并保留原有文件。
`doctor` 打印这台机器上每个来源的可用性与不可用的原因。

把 `~/.local/share/OpenHardwareOS/cli-v0.1.4` 加入 `PATH` 之后，可以直接运行：

```bash
export PATH="$HOME/.local/share/OpenHardwareOS/cli-v0.1.4:$PATH"
ohm-cli demo --steps 60     # 模拟设备，不接触真实硬件
ohm-cli status              # 读真实设备（只读）
```

## 安装桌面应用（.deb）

桌面应用读取同一批数据来源，多一个界面与托盘。`.deb` 面向 **Debian/Ubuntu**：

```bash
set -euo pipefail
ohm_version=v0.1.4
ohm_platform=linux-x86_64
ohm_deb="OpenHardwareOS-$ohm_version-$ohm_platform.deb"
ohm_meta="release-$ohm_platform.json"
ohm_sums="SHA256SUMS-$ohm_platform"
ohm_base="https://github.com/SvenKunkka/OpenHardwareOS/releases/download/$ohm_version"
ohm_tmp="$(mktemp -d)"
trap 'rm -rf "$ohm_tmp"' EXIT

curl -fsSL -o "$ohm_tmp/$ohm_deb" "$ohm_base/$ohm_deb"
curl -fsSL -o "$ohm_tmp/$ohm_sums" "$ohm_base/$ohm_sums"
ohm_pattern="^[0-9a-f]{64}  $ohm_deb\$"
ohm_matches="$(grep -cE "$ohm_pattern" "$ohm_tmp/$ohm_sums" || true)"
test "$ohm_matches" = "1" || { echo "$ohm_sums 中应有且仅有一条匹配记录，实际 $ohm_matches 条" >&2; exit 1; }
grep -E "$ohm_pattern" "$ohm_tmp/$ohm_sums" > "$ohm_tmp/verify.sha256"
(cd "$ohm_tmp" && sha256sum -c verify.sha256)

# apt 而不是 dpkg：它会自己装上包声明的依赖（GTK、WebKitGTK 等）。
sudo apt-get install -y "$ohm_tmp/$ohm_deb"
openhardwareos --version 2>/dev/null || true
```

装完后应用出现在应用菜单里，可执行文件在 `/usr/bin/openhardwareos`。运行时配置与审计记录在
`~/.config/OpenHardwareOS`。Debian 包名是 `open-hardware-os`（dpkg 只接受小写），
所以查询与卸载用这个名字：

```bash
dpkg -l open-hardware-os
sudo apt-get remove open-hardware-os
```

包声明的依赖由打包时的实际链接决定。发布日志会打印包实际声明的依赖、安装路径与桌面入口；
本地也可以直接查：

```bash
dpkg -s open-hardware-os | grep '^Depends:' || true
```

**没有界面自检这条路。** `openhardwareos --selftest --mock` 会**不建窗口**、用模拟硬件跑一个
自动化周期并打印报告后退出，适合在服务器/CI 上确认"装上了、能跑、读得到数据"。
它不证明界面能画出来——那需要真实的桌面会话。

## AppImage（免安装）

不想装包时用同一个 Release 里的 AppImage：

```bash
set -euo pipefail
ohm_version=v0.1.4
ohm_platform=linux-x86_64
ohm_image="OpenHardwareOS-$ohm_version-$ohm_platform.AppImage"
ohm_sums="SHA256SUMS-$ohm_platform"
ohm_base="https://github.com/SvenKunkka/OpenHardwareOS/releases/download/$ohm_version"
ohm_tmp="$(mktemp -d)"
trap 'rm -rf "$ohm_tmp"' EXIT

curl -fsSL -o "$ohm_tmp/$ohm_image" "$ohm_base/$ohm_image"
curl -fsSL -o "$ohm_tmp/$ohm_sums" "$ohm_base/$ohm_sums"
grep -E "^[0-9a-f]{64}  $ohm_image\$" "$ohm_tmp/$ohm_sums" > "$ohm_tmp/verify.sha256"
(cd "$ohm_tmp" && sha256sum -c verify.sha256)

chmod +x "$ohm_tmp/$ohm_image"
APPIMAGE_EXTRACT_AND_RUN=1 "$ohm_tmp/$ohm_image" --selftest --mock
```

`APPIMAGE_EXTRACT_AND_RUN=1` 让 AppImage 不必依赖 FUSE；没有它需要系统装了
`libfuse2`。AppImage 不会出现在应用菜单里，也不会写系统目录。

## 这台机器上能读到什么

Linux 上每个读数的来源都写在 `doctor` 输出里，能读就给出值，读不到就给出**原因**
（而不是 0）：

| 读数 | 来源 | 说明 |
|---|---|---|
| CPU 名称、占用率、频率 | `/proc`、`sysinfo` | 无需特权 |
| 温度（CPU 封装、主板、硬盘等） | 内核 hwmon 温度传感器 | 有多少读多少 |
| **机箱风扇转速** | 内核 hwmon `fan<N>_input` | 需要主板驱动支持（例如 `nct6798d`、`it87`）；没有这个驱动就没有转速读数，这是硬件/驱动边界，不是软件缺陷 |
| **风扇占空比（当前值）** | 内核 hwmon `pwm<N>` | **只读**：本版本不写 PWM |
| 内存使用量 / 总量 | `/proc/meminfo`、`sysinfo` | —— |
| 磁盘剩余空间 | 挂载点 | —— |
| GPU 温度 / 风扇 | NVML（装了 NVIDIA 驱动时） | 需要驱动；NVML 在 Linux 上不读机箱风扇 |

风扇通道的**设备身份取自芯片名与通道号**，例如 `fan.system.nct6798d_fan1`。
内核把 `hwmon*` 重新编号不会改变这个身份，所以规则不会因此指向别的风扇。

**为什么这一版不写 PWM。** 写 `pwm<N>` 需要 root，而且哪一路 `pwm` 对应机箱上的哪个
接口取决于主板与驱动；在确认的通道映射与实机证据出现之前，这个项目只读不写。
`pwm<N>_enable` 的值会显示出来，说明当前是驱动/固件在控制还是留给软件。

## 更新与卸载

- **CLI：** 更新时把 `ohm_version` 改成目标版本：新版本装进新的目录，旧版本保留。
  卸载即删除对应的 `~/.local/share/OpenHardwareOS/cli-<版本>` 目录。
- **桌面应用：** 下载新版本的 `.deb` 再 `sudo apt-get install -y ./<新文件>` 即可覆盖升级；
  卸载 `sudo apt-get remove openhardwareos`。

运行配置与审计记录在 `~/.config/OpenHardwareOS`（或 `OHM_CONFIG_DIR` 指向的位置），
删除 CLI 或卸载桌面应用都不会删除它们。

## 这些命令验证到哪一步

发布流程在 **GitHub Actions 的 `ubuntu-latest`** 上把 `.deb` 用 `apt-get install` 装进 runner，
再运行装好的 `/usr/bin/openhardwareos --selftest --mock`（含 `--dry-run` 一次），并单独运行
AppImage。这次运行自己打印出它是什么机器：
[run 34931865680](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34931865680)
报告 `PRETTY_NAME="Ubuntu 24.04.5 LTS"`、内核 `6.17.0-1022-azure`，并打印
`installed as /usr/bin/openhardwareos`；同一个 run 还打印包声明的依赖、装出来的可执行文件与
桌面入口——**发布日志本身就是证据**，见 `docs/verification-log.md` 第 9 轮。

**没有验证的：**真实机器上的界面显示（托盘、窗口、菜单项）、真实主板的 hwmon 通道、
风扇控制（本版本只读）、以及 Debian 之外的发行版（RPM 系没有产物）。CI 证明的是
"这个包在 Ubuntu 24.04 上装得上、跑得起来、读得到模拟数据"。

## 失败时请交回什么

`ohm-cli doctor` 的完整输出、`ohm-cli --version`（桌面应用为 `openhardwareos --selftest --mock` 的输出）、
`dpkg -l openhardwareos`、发行版与内核版本
（`cat /etc/os-release`、`uname -r`）、`ls /sys/class/hwmon/*/name` 的结果，
以及失败命令的**原始输出与退出码**。这些信息足以判断是驱动、权限还是软件问题。
反馈入口：[GitHub Issues](https://github.com/SvenKunkka/OpenHardwareOS/issues)。

本页描述的是**监测**能力。真实风扇控制、后台服务与实机验收仍是未完成的工作，
详见[六阶段计划](plans/hardware-support/README.md)。

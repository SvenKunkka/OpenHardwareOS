# Linux 命令行安装

公开源码：<https://github.com/SvenKunkka/OpenHardwareOS>。
本页安装 Linux x86_64 预览版本 `v0.1.3`。预编译安装无需 Rust 或 Node.js。

[查看全部版本](versions.md) · [交互版本树](https://svenkunkka.github.io/OpenHardwareOS/)

## 安装 CLI

```bash
set -euo pipefail
ohm_version=v0.1.3
ohm_platform=linux-x86_64
ohm_asset="ohm-cli-$ohm_version-$ohm_platform.tar.gz"
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
curl -fsSL -o "$ohm_tmp/$ohm_sums" "$ohm_base/$ohm_sums"
# 恰好一条匹配记录：多一条、少一条都停下。
ohm_pattern="^[0-9a-f]{64}  $ohm_asset\$"
ohm_matches="$(grep -cE "$ohm_pattern" "$ohm_tmp/$ohm_sums" || true)"
test "$ohm_matches" = "1" || { echo "$ohm_sums 中应有且仅有一条匹配记录，实际 $ohm_matches 条" >&2; exit 1; }
(cd "$ohm_tmp" && grep -E "$ohm_pattern" "$ohm_sums" | sha256sum -c -)
mkdir -p "$ohm_dir"
tar -xzf "$ohm_tmp/$ohm_asset" -C "$ohm_dir"
"$ohm_dir/ohm-cli" --version
"$ohm_dir/ohm-cli" doctor
```

校验和清单先核对再解压；校验失败 `set -e` 会直接中止。CLI 装在当前用户目录下，
不需要 root，也不修改 `PATH`。目标目录已存在时会停止并保留原有文件。
`doctor` 打印这台机器上每个来源的可用性与不可用的原因。

把 `~/.local/share/OpenHardwareOS/cli-v0.1.3` 加入 `PATH` 之后，可以直接运行：

```bash
export PATH="$HOME/.local/share/OpenHardwareOS/cli-v0.1.3:$PATH"
ohm-cli demo --steps 60     # 模拟设备，不接触真实硬件
ohm-cli status              # 读真实设备（只读）
```

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

更新时把 `ohm_version` 改成目标版本：新版本装进新的目录，旧版本保留。
卸载即删除对应的 `~/.local/share/OpenHardwareOS/cli-<版本>` 目录。
运行配置与审计记录在 `~/.config/OpenHardwareOS`（或 `OHM_CONFIG_DIR` 指向的位置），
删除 CLI 不会删除它们。

## 失败时请交回什么

`ohm-cli doctor` 的完整输出、`ohm-cli --version`、发行版与内核版本
（`cat /etc/os-release`、`uname -r`）、`ls /sys/class/hwmon/*/name` 的结果，
以及失败命令的**原始输出与退出码**。这些信息足以判断是驱动、权限还是软件问题。
反馈入口：[GitHub Issues](https://github.com/SvenKunkka/OpenHardwareOS/issues)。

本页描述的是**监测**能力。真实风扇控制、后台服务与实机验收仍是未完成的工作，
详见[六阶段计划](plans/hardware-support/README.md)。

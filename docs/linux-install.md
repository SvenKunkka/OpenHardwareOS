# Linux 安装

公开源码：<https://github.com/SvenKunkka/OpenHardwareOS>。
本页安装 Linux x86_64 预览版本 `v0.1.10`，两种方式：**命令行（CLI）** 与 **桌面应用（.deb）**。
两者都是预编译产物，安装无需 Rust 或 Node.js。

[查看全部版本](versions.md) · [交互版本树](https://svenkunkka.github.io/OpenHardwareOS/)

## 安装 CLI

```bash
set -euo pipefail
ohm_version=v0.1.10
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

把 `~/.local/share/OpenHardwareOS/cli-v0.1.10` 加入 `PATH` 之后，可以直接运行：

```bash
export PATH="$HOME/.local/share/OpenHardwareOS/cli-v0.1.10:$PATH"
ohm-cli demo --steps 60     # 模拟设备，不接触真实硬件
ohm-cli status              # 读真实设备（只读）
```

## 安装桌面应用（.deb）

桌面应用读取同一批数据来源，多一个界面与托盘。`.deb` 面向 **Debian/Ubuntu**：

```bash
set -euo pipefail
ohm_version=v0.1.10
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
ohm_version=v0.1.10
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

## 写入 PWM：默认关闭，按通道开启

**读取不需要任何配置；写入需要你逐通道确认。** 原因是硬件事实，不是保守：

`pwm<N>` 与 `fan<N>_input` 共用同一颗芯片和通道号，但这**不保证** `pwm1` 驱动的是
`fan1_input` 测速的那个接口——这个对应关系是主板属性，内核接口不声明它。所以这个项目
把"哪一路 pwm 对应哪个物理接口"当成**必须由人确认**的信息，并且默认一个通道都不写。

### 先确认，再开启

在**你的机器**上按 [实机核对清单](field-checklist.md) 的第 2、3 步做，记录：

| 要记的事实 | 从哪里 | 为什么需要 |
|---|---|---|
| 芯片名与通道号 | `ohm-cli doctor` 的设备 id，例如 `fan.system.nct6798d_fan1` | 它就是下面要填的 id |
| `pwm<N>_enable` 当前值 | `cat /sys/class/hwmon/*/pwm<N>_enable` | 现在是驱动/固件在控制还是留给软件 |
| 该 `pwm` 对应哪个物理接口 | 主板手册 + 逐通道试写（见清单） | 唯一无法由软件推断的事实 |
| 转速是否真的跟着变 | 写入前后 `fan<N>_input` 与 `ohm-cli watch` | 证明"写的这一路"确实是"测的那一路" |

确认之后，**只把那一个通道**写进配置（`~/.config/OpenHardwareOS/settings.json`）：

```json
{
  "adapter_settings": {
    "system": {
      "pwm_write_allow": ["fan.system.nct6798d_fan1"]
    }
  }
}
```

```bash
# 提供方应显示 system 可写；这台机器没有可写的 hwmon 通道时，下面两行会各自说明。
ohm-cli doctor | grep -E 'system|pwm_write_allow' || echo "  (没有可写的 system 通道)"
ohm-cli status --json | grep -A 2 fan.speed_percent || echo "  (没有风扇通道读数)"
```

### 开启之后会发生什么

* 只有列出的通道会出现**可写**的 `fan.speed_percent`；其余通道仍然只读。
* 第一次写入时，如果该通道由驱动控制（`pwm<N>_enable` 是自动），本程序会把它切成手动
  并**记住原来的值**；退出时恢复原值，驱动/固件重新接管。
* 通道的**当前归属读不出来**时（文件不可读、内容不是数字），本程序**不写**：
  归属未知的通道不该被接管，也不会把猜测写回去。
* 写 `pwm<N>` 需要 **root**；`ohm-cli doctor` 会把这一点写在提供方能力里。
* 写入仍然经过运行时的安全层：最小值限制、斜坡限制、紧急上限都会生效，
  每一次写入都进审计（`ohm-cli audit`）。

### 这一条**尚未在真实硬件上验证**

按通道写入的代码有测试（假 sysfs 树：能力声明、百分比到 255 的换算、接管与恢复、
未授权通道被拒），但**没有任何一台真实机器**跑过它。清单第 3 步的意义正是把第一次
真机写入变成一条可归档、可回退的记录。在你自己的机器上确认之前，把上面那行配置留空
——默认（只读）永远可用。

## 读数如何与系统来源核对

这一版的每一个读数都来自**这台机器自己的接口**，所以都能用系统工具独立核对。
下面这段在 Linux 上逐项打印对照（非 Linux 会直接说明并退出，不做任何猜测）：

```bash
test "$(uname -s 2>/dev/null)" = Linux || { echo "这些对照命令是 Linux 专用。"; exit 0; }
set -uo pipefail
echo "== 我们报告的读数（机器可读）"
ohm-cli status --json | grep -E '"capability"|"value"|"reason"' | head -40
echo
echo "== 内核自己的风扇与 PWM 文件"
for dir in /sys/class/hwmon/hwmon*; do
  [ -d "$dir" ] || continue
  printf '%s: %s\n' "$dir" "$(cat "$dir/name" 2>/dev/null)"
  for file in "$dir"/fan*_input "$dir"/pwm[0-9]; do
    [ -f "$file" ] || continue
    printf '  %s = %s\n' "$(basename "$file")" "$(cat "$file" 2>/dev/null)"
  done
done
echo
echo "== 内存：我们报的 total 应当等于 MemTotal"
grep -E '^(MemTotal|MemAvailable):' /proc/meminfo 2>/dev/null || echo "  没有 /proc/meminfo"
echo
echo "== 磁盘：我们报的剩余空间应当等于同一挂载点的 df 可用值"
df -k / 2>/dev/null | tail -2
```

每一项怎么对应：

| 我们的读数 | 系统的同一个值 | 说明 |
|---|---|---|
| `fan.system.<芯片>_fan<N>/fan.rpm` | `/sys/class/hwmon/*/fan<N>_input`（`name` 等于 `<芯片>`） | 同一个文件，数值必须相等 |
| `fan.system.<芯片>_fan<N>/fan.pwm` | `/sys/class/hwmon/*/pwm<N>` | 同上；只读展示当前占空比 |
| `memory.system.0/memory.total` | `/proc/meminfo` 的 `MemTotal` | 字节数必须相等（kB × 1024） |
| `memory.system.0/memory.used` | `MemTotal − MemAvailable` | 定义不同，允许 1 % 或 256 MB 的差 |
| `storage.system.0/storage.free` | `df -k <挂载点>` 的第 4 列 | 时间差会造成小幅漂移 |
| 温度 | `/sys/class/hwmon/*/temp*_input`、`/sys/class/thermal/thermal_zone*/temp` | 不是同一个传感器，因此只要求落在平台报告的温度范围内 |
| `cpu.load`、`cpu.frequency` | 没有独立来源 | 占用率是两次采样之间的差值，频率由调频器随时改变——单次采样无法核对，因此不假装能核对 |

仓库里还有把上表逐项自动化并给出结论的脚本，发布流程在真实内核上运行它：

```bash
./scripts/verify-linux-readings.sh            # 需要源码检出；结论逐行打印
```

它输出 `AGREE`（与平台一致）、`DIFFER`（不一致，退出码 1）、`NO-SOURCE`（平台没有可比的对象，
并说明原因）、`NOT-CHECKED`（本质上没有独立来源）四种结论——**"这台机器没有风扇转速可读"
是一个正常结果，不是失败**，但必须说出来。

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

# 实机核对清单（Windows 与 Linux）

这份清单只做一件事：**把"在这台机器上真的能读到、真的能控制、真的能恢复"变成可以归档的证据**。
每一步都写清楚要跑什么、要记录什么、以及**怎么撤回**。

它同时服务于两件事：

* **Windows 机箱风扇与水泵的实测**（计划中的"接下来①"）；
* **Linux 上确认一个风扇通道**——这是本版**只读**的原因，也是开放写入前必须补齐的证据。

> **先读这一节。**
> 本清单会出现**写真实硬件**的步骤（仅 Windows + LibreHardwareMonitor 的已支持通道）。
> 三条规则：一次只碰**一个通道**；**不要**碰 CPU 风扇（`CPU_FAN`）或任何标着 CPU 的通道；
> 每一步都先想好怎么回到原状（多数情况是关掉规则、退出程序，或重启进 BIOS 让固件重新接管）。
> 如果读不懂某一步在写什么，就停在读的那一步——**读的部分本身已经是有效证据**。

---

## 0. 准备

| 平台 | 准备 |
|---|---|
| Windows | 装好 **LibreHardwareMonitor**，打开 `Options -> Remote Web Server -> Run`（默认端口 8085）；本工具只读它的 Web 接口。风扇控制需要**管理员权限**运行 LHM。 |
| Linux | 无需特权即可读取（`/sys/class/hwmon`、`/proc/meminfo`）。本版**不写** `pwm`，所以不需要 root。 |

先确认版本与安装来源：

```bash
ohm-cli --version
```

Windows 若用安装脚本装的，另外记下 `dpkg` 无关的东西：`%LOCALAPPDATA%\OpenHardwareOS\cli-<版本>\ohm-cli.exe` 的存在与否即可。

---

## 1. 收集一份报告（两个平台都做）

```bash
ohm-cli report
```

它会把两半写进**一个 JSON 文件**并打印路径：

* 我们报告的内容：提供方与状态、每个设备与读数、每个缺失读数的原因、能力说明、已安装规则；
* **平台自己的答案**：`hwmon` 目录里每个芯片的 `fan*_input` / `pwm*` / `pwm*_enable` / `temp*_input`
  的原始内容、`/proc/meminfo` 的 `MemTotal`/`MemAvailable`、`df -k` 的输出。

在 Windows 上平台那一半基本是空的（没有 sysfs）；**这不是缺陷**，此时对照对象是
LibreHardwareMonitor 自己的窗口——把同一个传感器的数值抄进记录里即可。

同时留下：

```bash
ohm-cli doctor            # 人可读的完整报告
ohm-cli doctor --json     # 同样的内容，机器可读
```

Linux 上再跑一次逐项对照（`AGREE` / `DIFFER` / `NO-SOURCE` / `NOT-CHECKED`）：

```bash
./scripts/verify-linux-readings.sh
```

**要记录**：三个命令的**完整输出**，以及报告 JSON 文件本身。

---

## 2. 确认"读到的是这台机器"（只读）

| 项 | 怎么做 | 记录什么 |
|---|---|---|
| 设备清单 | `ohm-cli doctor` 的 Devices 与 Capabilities | 是否出现你机器上真实存在的型号（主板/GPU/硬盘） |
| 风扇转速 | 报告 JSON 里 `fan.system.<芯片>_fan<N>/fan.rpm` | 数值；以及它与系统来源是否一致（Linux 用第 1 步的对照脚本，Windows 用 LHM 窗口） |
| 温度 | `temperature.*` 读数 | 是否落在合理范围（例如 30–60 °C 空闲） |
| 内存/磁盘 | `memory.*`、`storage.*` | 与 `MemTotal`、`df` 是否一致 |
| GPU | `gpu.*` 读数与 NVML 是否可用 | 装了 NVIDIA 驱动时应有读数；没有时 `doctor` 会说原因 |

**判定**：出现"读到 0"或"读到一个不可能的数值"就是缺陷，请把报告发回。
**"这台机器没有这个通道"不算缺陷**——只要 `doctor` 给出了原因。

---

## 3. 确认控制（逐通道；Windows 与 Linux 都做）

> **Linux 需要额外记录四件事**，它们是开放某一路写入的唯一依据（见
> [Linux 安装说明](linux-install.md#写入-pwm默认关闭按通道开启)）：
> 芯片名与通道号（`ohm-cli doctor` 里的 `fan.system.<芯片>_fan<N>`）、
> `cat /sys/class/hwmon/*/pwm<N>_enable` 的当前值、
> 这一路 `pwm` 对应主板上的哪个物理接口（查手册或逐通道试）、
> 以及写入后**转速是否真的跟着变**（`cat /sys/class/hwmon/*/fan<N>_input`）。
> 软件无法推断 `pwm<N>` 与 `fan<N>_input` 是否同一个物理接口，所以这四件事必须是人记录的。

> 只做**一个**通道，先选一个**机箱风扇**，不要选 CPU 风扇。

1. 记录规则前的状态：
   ```bash
   ohm-cli status --json        # 记下目标通道当前的占空比与转速
   ```
2. 从**这台机器真正的通道**出发写规则。两个 id 都从
   `ohm-cli doctor` 的 Capabilities 一节（或 `ohm-cli rules suggest` 生成的规则文件）里抄，
   不要照抄本文档——本文档里的 id 全部是模拟设备的：

   ```bash
   ohm-cli rules suggest      # 按这台机器装一条起步规则，再照着改
   ohm-cli rules list         # 看它的 source / target 是什么
   ```

   改写后的规则（**只改一个通道**，曲线两个点保持恒定 40 %，先禁用）：

   ```yaml
   # Windows: %LOCALAPPDATA%\OpenHardwareOS\rules\field-test.yaml
   # Linux:   ~/.config/OpenHardwareOS/rules/field-test.yaml
   name: Field test one channel
   id: field-test
   enabled: false          # 先禁用，确认无误后再启用
   source: { device: <从 doctor 抄的真实传感器 id>, capability: temperature.core }
   target: { device: <从 doctor 抄的真实风扇通道 id>, capability: fan.speed_percent }
   curve: [[0, 40], [100, 40]]
   hysteresis: 0
   deadband: 0
   update_interval_ms: 1000
   ```

3. 校验、启用、观察：
   ```bash
   ohm-cli rules check <该文件路径>
   ohm-cli rules set-enabled field-test true
   ohm-cli watch --interval 1      # 看着转速与占空比
   ohm-cli audit --limit 20        # 每一次写入都有审计条目
   ```
   **记录**：写入前后的转速（RPM 是否跟着变化）、审计条目、`watch` 的输出片段。
4. **撤回**：
   ```bash
   ohm-cli rules set-enabled field-test false
   ohm-cli rules delete field-test
   ohm-cli handovers               # 通道是否已交回、是否被安全处理
   ```

**判定**：转速确实变化 = 真控制；转速不动但审计说写入成功 = 需要记录的实际问题；
写入被拒 = 记录拒绝原因（这同样是有价值的结论）。

### Linux 的逐通道写入（确认之后）

1. 先停掉所有会写这个通道的东西（包括本程序的服务：`ohm-cli service status` 确认没有在跑）。
2. 用 **root** 直接写一次，绕开本程序，确认物理对应关系：
   ```bash
   echo 128 | sudo tee /sys/class/hwmon/*/pwm<N>     # 50 %
   cat /sys/class/hwmon/*/fan<N>_input              # 转速是否跟着变
   ```
   变了 → 这一路 `pwm` 与这一路测速是同一个接口；没变 → **不要**把它加进白名单。
3. 记录 `pwm<N>_enable` 的当前值，然后把它切到手动并交给本程序（修改
   `settings.json` 的 `pwm_write_allow`，见 Linux 安装说明），跑一条只改这一路、
   占空比恒定的规则，观察 `ohm-cli watch` 与 `ohm-cli audit`。
4. **撤回**（三步都做）：
   ```bash
   ohm-cli rules set-enabled field-test false
   ohm-cli rules delete field-test
   # 退出服务：本程序会把 pwm<N>_enable 恢复成记录下来的原值
   ohm-cli service status
   cat /sys/class/hwmon/*/pwm<N>_enable     # 应等于第 3 步记录的原值
   ```
   若原值与现在不同，把 `pwm_write_allow` 清空，并**直接写回原值**：
   ```bash
   echo <原值> | sudo tee /sys/class/hwmon/*/pwm<N>_enable
   ```
   然后重启一次让固件重新接管，并把这台机器的记录发回——这正是需要证据的那一步。

---

## 4. 确认"退出与故障后恢复"

1. **正常退出**（两个平台）：
   ```bash
   ohm-cli service run --heartbeat-ms 1000     # 让它跑一会儿，然后 Ctrl-C
   ```
   **记录**：退出摘要（已确认/未确认/被拒绝/失败/模拟各多少、适配器是否交还控制），
   以及 `ohm-cli audit --limit 20` 里最后几条。
2. **异常终止**：再跑一次，然后**直接结束进程**（Windows 任务管理器"结束任务"；
   Linux `kill -9 <pid>`）。**记录**：之后风扇停在什么状态、`ohm-cli handovers` 是否报告了它。
   这一步说明的是"被强杀时不会自动恢复"——需要如实记录，而不是修掉。
3. **服务在后台时的界面关闭**（Windows 桌面应用 + Linux）：开着服务时关掉桌面窗口，
   确认规则仍在跑（`ohm-cli service status` 的周期计数继续增长）。
4. **传感器失联**（Linux，只读）：拔掉或停掉一个传感器来源（例如 `rmmod` 对应驱动，
   或对准备好的树用 `OHM_HWMON_ROOT` 演练），**记录** `service status` 里该规则的状态与消息。

---

## 5. 交回什么

一份报告 + 三段输出，足以让任何人在另一台机器上判断结果：

| 文件 | 来源 |
|---|---|
| `report-<时间>.json` | `ohm-cli report` |
| `doctor.txt` | `ohm-cli doctor` |
| `doctor.json` | `ohm-cli doctor --json` |
| `crosscheck.txt`（Linux） | `./scripts/verify-linux-readings.sh` |
| `audit-tail.jsonl` | `ohm-cli audit --limit 50` |
| 本清单第 3、4 步的手写记录 | 转速前后、退出摘要、异常终止后的状态 |

反馈入口：[GitHub Issues](https://github.com/SvenKunkka/OpenHardwareOS/issues)。
**请勿附带任何与本项目无关的私人内容**（LHM 窗口截图请只截传感器区域）。

---

## 6. 哪些结论还没有证据

写在这里，免得把"能跑"读成"已验证"：

* 任何一台真实机器上的读取、控制、恢复——**都还没有**。上面每一步都是为了让它们**第一次**有证据。
* Windows 桌面应用的安装/卸载、托盘、自启。
* Linux 上写 `pwm<N>`：本版**没有实现**，因此也没有权限方案可谈。开放它的前提正是第 2、3 步
  在 Linux 上产出的通道证据（芯片名、通道号、`pwm<N>_enable` 的当前值、写多少对应哪个物理接口）。
* 水泵（`v0.2.0`）：计划在机箱风扇之后，同样需要上面的记录。

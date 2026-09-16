# 后台规则服务

桌面应用关闭后，规则就停了。`ohm-cli service run` 就是那个把规则继续跑下去的进程：
它启动运行时与规则循环，**不建窗口**，一直运行到你让它停下。

```bash
ohm-cli service run                  # 用这台机器的真实来源
ohm-cli service run --mock           # 模拟硬件 + dry-run，用于试用与测试
ohm-cli service status               # 看这里是否有服务在跑
```

`service status` 打印状态文件里的内容：pid、启动时间、心跳年龄、已执行的周期数、加载的规则数、
是否在用模拟来源、是否 dry-run，以及**每条规则现在的状态、确认值与它自己给出的一句话**：

```text
rules:
  chassis                  held         at 64  no change worth writing (0.00 % within the 0.00 % deadband)
  gpu-cooling              fallback     at 70  gpu.mock.0/temperature.core is missing: falling back to 70 %
```

这一行是"行为明确"的关键：`3 rules loaded` 无法区分"正在按曲线降温"和"因为传感器掉了而停在
失效保护值上"。**没有服务在跑时 `service status` 以退出码 1 结束**，有服务时是 0，便于脚本判断。
`service run` 在已有服务运行时以退出码 3 结束，并说明是谁持有状态文件。

服务还会把日志写进配置目录下的 `logs/`（它没有终端可以看，必须留下文件）；启动横幅会打印状态
文件与日志目录的准确路径。

## 一个通道只能有一个写入者

桌面应用和后台服务跑的是同一批规则、同一批通道。两个进程轮流往同一个风扇写值，
是这个项目最不该发生的事，所以服务在**开始工作之前**先声明所有权：

* 状态文件位于配置目录下（`ohm-cli paths` 会打印准确路径，通常是
  `~/.config/OpenHardwareOS/service.json`）；
* 创建它是原子的（`create_new`）：两个同时启动的进程只有一个能拿到，另一个被拒绝；
* 持有者每秒（`--heartbeat-ms` 可调）重写一次心跳，所以**心跳过期的文件属于一个已经死掉的进程**
  ——包括被 `SIGKILL` 杀掉的进程，它没有机会清理；
* 拿回一个过期的文件同样是原子的，所以两个进程同时判定"它死了"时也只有一个能接手；
* 文件坏了（不是 JSON）视为无人持有：否则机器会因为一个手改坏的文件而永远起不了服务；
* 干净退出会删除文件，正常情况下什么都不留下；
* **每次写心跳之前先核对"这个文件还是我的吗"**。心跳窗口比一次休眠短，这是有意的：睡了一小时的
  机器不该继续占着通道。但那个判断只有在**醒来的人自己核对**时才安全——否则被唤醒的进程会照旧
  重写文件、照旧往同一个风扇写值，机器上就出现了两个服务交替写同一个接口。现在核对失败时它
  **停手**：不覆盖新持有者的记录、不删除它、退出码 4，并在日志与控制台里点名现在是谁持有通道；
* 被冻结期间**墙上时钟在走、进程自己的单调时钟不走**（Linux/macOS 如此），两者的差值就是"我被
  挂起了多久"。检测到之后会记进状态文件（`resumes`、`last_resume_gap_ms`）并写一条日志，因为
  这正是心跳显示不出来的那件事：进程被冻住时，它的文件在别人眼里是"废弃的"。

**桌面应用也会看这个文件。** 如果它发现有另一个进程正在驱动通道，它**不会启动自己的规则循环**，
而是把原因写进日志与事件流（"another process (pid N) is running the rules and owns the hardware
channels"）。它会照常读数据、显示界面——只是不再写。

## 停止它

`Ctrl-C`，或者服务管理器发送的 `SIGTERM`，做的是同一件事：

1. 停止接受新的规则决策；
2. 关闭运行时——这一步会把控制权交回，并**如实报告结果**：已确认、未确认、被拒绝、失败、模拟
   各自成条，适配器是否会交还控制也分别列出（`ohm-cli` 的退出摘要就是这份报告）；
3. 删除状态文件，让下一次启动不会以为还有服务在跑。

被 `SIGKILL`（`kill -9`）杀掉时第 2、3 步不会发生：状态文件留在原地，但心跳会停止，
于是下一个 `service run` 会把它当作废弃文件接手。**这也是为什么"停止"应当用 `SIGTERM` 而不是 `kill -9`**：
前者让控制权回到固件，后者让它们停在最后一次写入的值上。

## 用一棵准备好的 sysfs 树试跑

`OHM_HWMON_ROOT` 可以让整个进程去读另一棵 hwmon 树，而不是 `/sys/class/hwmon`：

```bash
OHM_HWMON_ROOT=/path/to/fake-sysfs ohm-cli status        # 看看会读到什么
OHM_HWMON_ROOT=/path/to/fake-sysfs ohm-cli service run   # 用假通道跑规则
```

它是**只读**的开关：这一版不写 `pwm<N>`，所以指向别的目录只改变"读什么"，不改变"写什么"。
服务测试正是用它来做到"让一个风扇通道在进程运行期间消失再回来"。

## 在 Linux 上作为用户服务运行

这一版**不会**替你安装任何东西——没有 `--install`，没有自启动项，也不写系统目录。
要让它随登录启动，自己放一份 systemd **用户**单元：

```ini
# ~/.config/systemd/user/openhardwareos.service
[Unit]
Description=OpenHardwareOS background rules service
After=default.target

[Service]
Type=simple
ExecStart=%h/.local/bin/ohm-cli service run
Restart=on-failure
RestartSec=5
# SIGTERM is the default stop signal, which is exactly what the service expects.
KillSignal=SIGTERM

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now openhardwareos.service
systemctl --user status openhardwareos.service
```

**权限**：读取（`/proc`、`hwmon`、`sysinfo`）不需要特权，用户服务足够。写 `pwm<N>` 需要 root，
而这一版**根本不写 PWM**（见 [Linux 安装说明](linux-install.md)）；将来开放某个已确认的通道时，
权限方案会与那个通道一起给出，而不是提前要一份 root。

## Windows

`service run` 在控制台里可以正常运行，`Ctrl-C` 同样干净退出。这一版没有 Windows 服务封装：
若是希望它随登录启动，可用任务计划程序手动添加（"登录时"触发，程序为 `ohm-cli.exe`，
参数为 `service run`），并注意任务结束时发送的是终止信号而不是 `kill`。这条路径**尚未在任何
Windows 机器上验证过**，因此不作为支持承诺。

## 这一版验证到哪一步

有测试（`apps/cli/tests/service_lifecycle.rs`、`tests/tests/service_ownership.rs`、
`crates/ohm-runtime/src/service.rs`）覆盖：

| 行为 | 证据 |
|---|---|
| 干净停止会释放控制并删除状态文件 | 有界运行 `--max-ticks` 与 `SIGTERM` 两条路径各一个进程级测试，断言退出码 0、日志里有释放报告、文件消失 |
| 第二个服务被拒绝 | 进程级测试：退出码 3，消息里带着持有者的 pid |
| 过期的状态文件会被接手 | 单元测试与进程级测试各一：写入一个心跳停止的文件，新的服务照常启动 |
| 坏掉的/无法解析的状态文件不会卡住启动 | 单元测试 |
| 另一个进程持有通道时，引擎不启动且说明原因 | 集成测试：状态文件指向别的 pid 时 `blocked_by` 有条目且 ticks 为 0；指向自己时不受影响；文件过期时不受影响 |
| 心跳窗口与间隔一致 | 单元测试（1 s 窗口容忍 2.5 s 旧的心跳、30 s 窗口容忍、300 ms 窗口不容忍） |
| **传感器失联 → 回退 → 恢复** | 进程级测试（`apps/cli/tests/service_resilience.rs`）：用 `OHM_HWMON_ROOT` 指向一棵准备好的 hwmon 树，规则读取其中的转速计并驱动模拟风扇；删掉转速计文件后，状态文件里该规则变成 `fallback`、值变成失效保护的 70 %、消息点名是哪个传感器不见了；把文件放回去后，规则重新读取该读数（状态离开 `fallback`，消息回到关于该读数的那句话） |
| **进程被挂起再恢复** | 进程级测试：`SIGSTOP` 超过心跳窗口后，`service status` 报"未运行"并指出这是谁的（过期）文件——这正是机器休眠时别的进程可以接手的状态；`SIGCONT` 之后它仍是自己的持有者，心跳恢复、周期计数继续增长 |
| **挂起期间被别人接手，醒来后停手** | 进程级测试（两个真实进程）：第一个 `SIGSTOP` 超过心跳窗口，第二个接手并开始跑规则，第一个 `SIGCONT` 之后**自己退出**（退出码 4，日志与控制台说明"另一个进程已接手通道"、状态文件未被动过）；第二个的 pid 与周期计数继续增长，最后干净退出。另有一个变体把 `SIGSTOP` 打在服务**启动过程中**（先声明所有权、还没启动引擎），要求同样的结局 |
| **心跳不再覆盖别人的记录** | 单元测试：把状态文件换成另一个 pid 的内容后 `heartbeat` 返回 `OwnershipLost` 且文件**逐字节未变**；文件被删掉时同样报告失去所有权而**不会重建**它；`Drop` 只在文件仍属于自己时才删除（否则会删掉新持有者的记录，让通道变成无人认领） |
| **唤醒检测（两个时钟的差值）** | 单元测试，纯函数：8 小时墙上时间对 1 秒进程时间 → 判定为唤醒并算出漏掉的时长；`SIGSTOP`/被饿死/调试器暂停（两个时钟同步前进）→ **不算**唤醒；正常节拍与阈值以下的抖动 → 不算；第一次观测只作基线 |
| 服务把日志写进文件 | 同上（测试与 `service status` 都依赖它；没有终端的进程必须留下记录） |

关于"恢复后为什么没有立刻写新值"：失效保护值（70 %）比曲线值（65 %）更高，规则按自己的滞回
语义**保持更高的那个值，直到读数越过锚点**——这是设计如此（少降温才是危险方向），而
`service status` 的那句话会把它说明白（"holding 70 % until … drops below …"）。这一条曾经看起来像
"传感器回来了但规则永远停在失效保护"，直到状态文件开始报告每条规则在做什么。

**没有验证的：**

* **真实硬件**：所有测试都用模拟来源，`--mock` 还会强制 dry-run。服务从未在一台有风扇的机器上运行过。
* **真实的休眠/恢复**：没有测试真的挂起一台机器。已经验证的是两件**属于本程序**的事：唤醒检测的
  判据（两个时钟的差值，纯函数单测），以及"醒来必须先问谁持有通道"的规则（两个真实进程的
  `SIGSTOP` 场景，退出码 4）。**没有被验证的**是那个判据成立的前提——Linux/macOS 上单调时钟在
  挂起期间停止、墙上时钟继续——以及内核挂起后设备驱动重新枚举那条路径；真实休眠仍需要一台
  可以睡眠的机器实测。
* **接手之后固件控制的归属**（已知缺口，代码位置 `adapters/system/src/lib.rs`、`crates/ohm-runtime/src/service.rs`）：
  如果前一个持有者已经把某个 `pwm<N>_enable` 切成手动并记住了原值，然后被 `SIGKILL` 或休眠中被
  接手，新持有者读到的"原值"就是**手动**——它退出时会把手动写回去，固件控制不会回来。修它需要
  把"我接管了哪些通道、原值是什么"写进状态文件并由接手方采纳（`ServiceState` 目前不含这些字段），
  这是下一步而不是这一版的内容。
* **传感器失联**现在在服务层有测试（见上表），但仍然全部是模拟硬件与准备好的 sysfs 树：
  真实主板的 `hwmon` 通道、真实驱动的消失与重连没有实测。
* **Windows 服务集成**（见上）。
* **开机自启**：这一版不安装任何自启动项，也不修改系统环境。

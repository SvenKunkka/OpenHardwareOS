# 后台规则服务

桌面应用关闭后，规则就停了。`ohm-cli service run` 就是那个把规则继续跑下去的进程：
它启动运行时与规则循环，**不建窗口**，一直运行到你让它停下。

```bash
ohm-cli service run                  # 用这台机器的真实来源
ohm-cli service run --mock           # 模拟硬件 + dry-run，用于试用与测试
ohm-cli service status               # 看这里是否有服务在跑
```

`service status` 打印状态文件里的内容：pid、启动时间、心跳年龄、已执行的周期数、加载的规则数、
是否在用模拟来源、是否 dry-run。**没有服务在跑时它以退出码 1 结束**，有服务时是 0，便于脚本判断。
`service run` 在已有服务运行时以退出码 3 结束，并说明是谁持有状态文件。

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
* 干净退出会删除文件，正常情况下什么都不留下。

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

**没有验证的：**

* **真实硬件**：所有测试都用模拟来源，`--mock` 还会强制 dry-run。服务从未在一台有风扇的机器上运行过。
* **休眠/恢复**：没有测试真的挂起一台机器。它依赖的两个机制各自有测试——心跳过期即视为停止、
  运行时把过旧读数当作失效并回退到安全值——但"挂起 8 小时后恢复"这条路径本身没有实测。
* **传感器失联**：由引擎自身的测试覆盖（读数消失 → 宽限期 → 失效保护占空比），服务只是驱动
  同一个引擎；服务层的重启与恢复仍未实测。
* **Windows 服务集成**（见上）。
* **开机自启**：这一版不安装任何自启动项，也不修改系统环境。

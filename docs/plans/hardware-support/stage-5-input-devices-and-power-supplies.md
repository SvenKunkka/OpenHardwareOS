> 快照：GitHub Issue #6 的正文镜像（逐字节复制），抓取于 2026-09-14 18:45 CST / 2026-09-14T10:45:43Z。**实时进度以 Issue 里的复选框为准**，本文件不会自动同步。
>
> Snapshot: a byte-for-byte mirror of the body of GitHub Issue #6, captured 2026-09-14T10:45:43Z. The issue's checkboxes are the live progress record; this file does not auto-sync.

<!--
source:      https://github.com/SvenKunkka/OpenHardwareOS/issues/6
title:       [Stage 5 / 阶段五] Input Devices & Power Supplies / 输入设备与电源
captured_at: 2026-09-14T10:45:43Z (2026-09-14 18:45 CST)
repo_commit: 2aea128b1bea6170b801a1a1af4724d0efa0b500
authority:   the GitHub issue is authoritative for progress; this file is the versioned snapshot.
-->

**语言 / Language:** [中文](#中文) · [English](#english)

两种语言使用相同的小阶段编号；中文区的复选框是共用进度记录。
Both versions use the same substage IDs. The checkboxes in the Chinese section are the shared progress record.

# 中文

## 阶段五：输入设备与电源

### 要达到的结果

用户可以用一款明确型号的键盘、旋钮或宏键设备切换 OpenHardwareOS 的散热和显示场景，同时查看一款具有遥测接口的电源的真实状态。每个输入动作都有明确作用对象，每项电源读数都能追溯到设备及单位。

状态：后续规划，尚未排期、分配负责人或绑定发行版本。下列目标均未完成；已有设备类型或第三方工具支持不算本项目的实机支持。

总任务：https://github.com/SvenKunkka/OpenHardwareOS/issues/1

### 当前基础与范围

- 已有 `Keyboard`、`Mouse`、`PowerSupply` 设备类型和规则、配置、审计基础，但公开版本没有键鼠或电源专用适配器。
- 首轮选择一款有可用协议的输入设备和一款可提供遥测的电源；型号、固件、连接方式及接口均待核实。普通键鼠、宏键盘、旋钮、媒体 Deck 是候选类别，不承诺整类或整品牌兼容。
- 电源先完成只读遥测；只有具体型号提供明确控制接口时，才单独定义可写能力及恢复条件。
- 依赖[阶段二](https://github.com/SvenKunkka/OpenHardwareOS/issues/3)的稳定设备身份、控制归属和后台运行基础；涉及屏幕或灯光的输入联动依赖[阶段四](https://github.com/SvenKunkka/OpenHardwareOS/issues/5)对应能力。两条设备接入线可分别推进。

### 小阶段目标与验收

- [ ] **5.1 明确两类参考设备与接口。** 目标：确定可执行的首批范围。交付：输入设备和电源的型号、硬件/固件版本、连接方式、公开协议或接口来源、依赖软件、权限及可读/可写能力表。验收：每类至少确定一款可测试样机，未知项明确列出；无法取得接口证据的能力不进入控制开发。

- [ ] **5.2 稳定识别输入设备并读取状态。** 目标：正确识别重插、重启和多设备场景。交付：设备身份映射、可用状态字段、连接和断连事件；电量、连接模式等仅在真实接口提供时展示。验收：同型号多台设备不会串号，缺失读数不会显示为零或虚构状态。

- [ ] **5.3 接收按键、旋钮和其他明确事件。** 目标：输入事件可作为场景触发源。交付：事件类型、设备来源、按下/释放或旋转方向与增量的定义、去抖和重复事件策略。验收：按参考设备实际能力逐项测试；设备重连不会补发旧动作，普通输入功能仍可正常使用。

- [ ] **5.4 将输入映射到可审计的硬件场景。** 目标：用户可切换静音/性能场景、调整已支持输出，或切换已支持屏幕页面。交付：可编辑映射、目标设备和动作说明、冲突处理、撤销或恢复方式及审计记录。验收：同一输入只触发预期动作；失效目标有明确反馈；风扇和水泵操作继续经过统一保护，重复/过期事件不重复写入。

- [ ] **5.5 接入电源只读遥测。** 目标：展示具有真实来源的功率、电压、电流、温度和风扇状态。交付：专用适配或经验证的桥接、字段单位和来源、轮询及过期策略。验收：参考电源实际提供的每项读数与来源接口核对；输入功率、输出功率和效率分开标识，不能用 CPU+GPU 功率之和冒充整机电源功率。

- [ ] **5.6 完成告警与控制范围判定。** 目标：遥测异常可用于通知和已验证的散热联动，电源控制具有清楚边界。交付：阈值、滞回、丢失/过期读数处理及可写能力评估记录。验收：模拟与实机可安全验证的场景分别留证；没有明确接口的电源保持只读。可选风扇模式或功率控制只有在型号范围、回读和恢复验证后才能标记支持，不包括直接开关主供电。

- [ ] **5.7 发布输入与电源兼容记录。** 目标：用户能知道支持到哪项能力。交付：输入样机和电源样机的支持卡、安装/连接说明、配置迁移说明和测试证据。验收：两类参考设备分别完成识别、读数/事件、重连测试；输入场景完成预期动作和恢复测试；涉及真实写入的能力另有独立证据，并关联实际发布版本。

### 阶段完成定义

一款输入设备完成真实事件到硬件场景的闭环，一款电源完成真实遥测与异常状态展示；相关软件回归和目标平台安装验证通过。每项可写能力分别记录控制和恢复结论，其余能力明确为只读或不支持。总任务和版本树只在真实发布后更新为相应完成状态。

### 范围边界与证据入口

本阶段不包含键鼠替换固件、任意宏脚本执行、未定义的用户输入采集、PSU 主供电开关、UPS/电池或整品牌支持。具体样机、开发资源、采购成本和工期在开工时确定，本任务本身不发起采购或设备写入。

- [当前设备类型](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/device.rs)
- [当前适配器集合](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/adapters/src/lib.rs)
- [硬件能力路线图](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/roadmap.md)
- [版本树](https://svenkunkka.github.io/OpenHardwareOS/)

---

# English

## Stage 5: Input Devices and Power Supplies

### Intended outcome

Users can switch OpenHardwareOS cooling and display scenes with a keyboard, knob, or macro-key device of a specified model while viewing real status from a power supply with a telemetry interface. Every input action has an explicit target, and every PSU reading is traceable to its device and unit.

Status: planned for a later stage, with no schedule, owner, or release version assigned. All goals below remain incomplete. Existing device types or support in third-party tools do not establish real-hardware support in this project.

Parent issue: https://github.com/SvenKunkka/OpenHardwareOS/issues/1

### Existing foundation and scope

- The project already has `Keyboard`, `Mouse`, and `PowerSupply` device types, together with rules, configuration, and auditing foundations. The public release has no dedicated keyboard/mouse or PSU adapter.
- Initially select one input device with an accessible protocol and one PSU that provides telemetry. Their models, firmware, connection methods, and interfaces require verification. Ordinary keyboards and mice, macro pads, knobs, and media decks are candidate categories; compatibility is not promised for an entire category or brand.
- Complete read-only PSU telemetry first. Define writable capabilities and recovery requirements separately only where a specific model provides an explicit control interface.
- This work depends on the stable device identity, control ownership, and background-operation foundations from [Stage 2](https://github.com/SvenKunkka/OpenHardwareOS/issues/3). Input actions involving displays or lighting depend on the corresponding [Stage 4](https://github.com/SvenKunkka/OpenHardwareOS/issues/5) capabilities. The two device-integration workstreams can advance independently.

### Substage goals and acceptance criteria

- **5.1 Define reference devices and interfaces for both categories.** Goal: establish an actionable initial scope. Deliverables: input-device and PSU models, hardware/firmware versions, connection methods, public protocol or interface sources, software dependencies, permissions, and a table of readable/writable capabilities. Acceptance: identify at least one available test device in each category and explicitly list unknowns. Capabilities without interface evidence do not enter control development.

- **5.2 Reliably identify input devices and read status.** Goal: maintain correct identity through reconnection, restart, and multiple-device scenarios. Deliverables: device identity mapping, available status fields, and connection/disconnection events. Display battery level, connection mode, and similar fields only when the actual interface provides them. Acceptance: multiple units of the same model are not confused with one another; missing readings are not presented as zero or fabricated status.

- **5.3 Receive key, knob, and other explicitly defined events.** Goal: make input events available as scene triggers. Deliverables: event types and device sources; definitions of press/release or rotation direction and increments; debounce and duplicate-event policies. Acceptance: test each capability actually provided by the reference device. Reconnection must not replay old actions, and ordinary input functionality must remain usable.

- **5.4 Map input to auditable hardware scenes.** Goal: let users switch quiet/performance scenes, adjust supported outputs, or switch supported display pages. Deliverables: editable mappings, target-device and action descriptions, conflict handling, undo or recovery methods, and audit records. Acceptance: each input triggers only its intended action; unavailable targets produce clear feedback. Fan and pump operations continue through the common safeguards, and duplicate or stale events do not cause repeated writes.

- **5.5 Integrate read-only PSU telemetry.** Goal: display power, voltage, current, temperature, and fan status with genuine device sources. Deliverables: a dedicated adapter or verified bridge, field units and sources, polling, and data-expiry policies. Acceptance: cross-check every reading actually provided by the reference PSU against its source interface. Identify input power, output power, and efficiency separately; do not present the sum of CPU and GPU power as whole-system PSU power.

- **5.6 Complete alerts and determine control scope.** Goal: use telemetry abnormalities for notifications and verified cooling coordination, with explicit PSU-control boundaries. Deliverables: thresholds, hysteresis, handling of missing/stale readings, and an assessment of writable capabilities. Acceptance: record simulation evidence separately from scenarios that can safely be verified on real hardware. PSUs without an explicit interface remain read-only. Optional fan-mode or power control may be marked supported only after model scope, readback, and recovery have been validated; direct switching of the main power output is excluded.

- **5.7 Publish input-device and PSU compatibility records.** Goal: show users exactly which capabilities are supported. Deliverables: support records for the input and PSU test devices, installation/connection instructions, configuration-migration notes, and test evidence. Acceptance: both reference devices separately pass identification, readings/events, and reconnection tests. Input scenes pass tests of the intended action and recovery. Capabilities involving real writes have separate evidence linked to an actual published release.

### Definition of done

One input device completes the end-to-end path from real events to hardware scenes. One PSU provides real telemetry and abnormal-state reporting. Relevant software regressions and installation validation on the target platform pass. Record control and recovery conclusions separately for each writable capability; explicitly identify all other capabilities as read-only or unsupported. Update the parent issue and version tree to the corresponding completion state only after an actual release.

### Scope boundaries and evidence references

This stage excludes replacement keyboard/mouse firmware, arbitrary macro-script execution, undefined collection of user input, switching the PSU's main power output, UPS/battery support, and brand-wide compatibility. Determine the specific test devices, engineering resources, procurement costs, and duration when work starts. This issue itself does not initiate procurement or device writes.

- [Current device types](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/device.rs)
- [Current adapters](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/adapters/src/lib.rs)
- [Hardware capability roadmap](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/roadmap.md)
- [Version tree](https://svenkunkka.github.io/OpenHardwareOS/)


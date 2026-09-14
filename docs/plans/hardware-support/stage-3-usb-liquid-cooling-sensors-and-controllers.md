> 快照：GitHub Issue #4 的正文镜像（逐字节复制），抓取于 2026-09-14 18:45 CST / 2026-09-14T10:45:43Z。**实时进度以 Issue 里的复选框为准**，本文件不会自动同步。
>
> Snapshot: a byte-for-byte mirror of the body of GitHub Issue #4, captured 2026-09-14T10:45:43Z. The issue's checkboxes are the live progress record; this file does not auto-sync.

<!--
source:      https://github.com/SvenKunkka/OpenHardwareOS/issues/4
title:       [Stage 3 / 阶段三] USB Liquid Cooling, Sensors & In-House Controllers / USB 水冷、传感器与自研控制器
captured_at: 2026-09-14T10:45:43Z (2026-09-14 18:45 CST)
repo_commit: 2aea128b1bea6170b801a1a1af4724d0efa0b500
authority:   the GitHub issue is authoritative for progress; this file is the versioned snapshot.
-->

**语言 / Language:** [中文](#中文) · [English](#english)

两种语言使用相同的小阶段编号；中文区的复选框是共用进度记录。
Both versions use the same substage IDs. The checkboxes in the Chinese section are the shared progress record.

# 中文

## 第三阶段：USB 水冷、传感器与自研控制器

关联：[六阶段总任务](https://github.com/SvenKunkka/OpenHardwareOS/issues/1) · [第二阶段](https://github.com/SvenKunkka/OpenHardwareOS/issues/3) · [第四阶段](https://github.com/SvenKunkka/OpenHardwareOS/issues/5)

状态：规划草稿，尚未排期；本阶段不绑定发布版本。以下小阶段在本 Issue 内跟踪，不自动创建独立 Issue。

### 用户结果

用户能接入经过验证的 USB 一体式水冷、独立风扇/水泵控制器和液温/流量传感器，辨认每个通道，先读取真实状态，再对已确认可控的通道使用安全策略。连接中断、电脑休眠或应用退出时，设备应进入已验证的本地保护状态，并明确显示当前由谁控制。

OpenFan/OpenHub 是本阶段内并行推进的自研硬件子线：形成可复现的参考板、设备协议和固件，并与第三方设备共用能力模型、自动化和验收记录，不另设第七阶段。

### 当前证据与缺口

- 已有 [设备与传输类型](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/device.rs)、[能力模型](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/capability.rs)、[泵安全策略](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-runtime/src/safety.rs)及 [模拟水泵](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/mock/src/devices.rs)。现有默认泵下限是软件策略，不能作为任意泵型号的安全规格。
- [LHM 映射](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/libre-hardware-monitor/src/mapping.rs)可把名称中含 `pump` 的通道映射为水泵。名称匹配、百分比设定值回读不等于已确认实物接线、转速或流量。
- [ODP 适配器](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/open-protocol/src/lib.rs)目前固定使用 `LoopbackTransport<MockOpenFan>`；[协议传输框架](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-protocol/src/transport.rs)已有字节流抽象，但没有接入真实 HID/CDC 枚举和连接流程。
- [MockOpenFan](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-protocol/src/mock.rs)中的失联保护、bootloader 和升级属于模拟逻辑；当前“多通道”模拟不能证明真实各通道独立控制。
- USB AIO 厂商协议、液温/流量能力、参考板电路、真实设备固件及断电恢复仍需实现或验证。`liquidctl` 仅为接入候选，本项目尚未集成；任何上游支持列表都不能直接转写为本项目兼容列表。

### 范围

主线为经过逐型号核对的 USB AIO、独立风扇/泵控制器、液温及流量传感器；并行子线为 OpenFan/OpenHub 参考板与 ODP 固件。桌面系统必须分别记录验证范围，不以一个平台的结果代表其他平台。灯光与屏幕的数据接口可预留，相关产品功能归第四阶段。

### 依赖

- 依赖 [第二阶段](https://github.com/SvenKunkka/OpenHardwareOS/issues/3)形成的风扇/水泵身份、通道对应、安全下限、控制权交接和故障处理约定；未完成部分必须在开始相应控制工作前补齐。
- 实机验证前须具备具体型号、板号/修订版、固件版本、USB 描述符、接线和供电信息，以及相应协议/SDK 的使用与再分发许可依据。
- 自研固件写入须另行落实匹配板型、可用恢复入口、原始镜像或备份、受控操作方案与明确授权。本 Issue 本身不授权当前连接硬件的任何写入或升级。

### 小阶段

- [ ] **3.1 建立设备候选与验证档案。** 目标：确定本阶段首批实际验证对象，避免按品牌笼统宣称支持。交付物：按型号、板号、固件、平台、USB 接口和功能列出的候选清单，协议来源与许可记录，以及只读调查结果。验收：每个拟进入实现的对象有可追溯身份和访问依据；未知项明确标记，未验证型号不进入兼容名单。
- [ ] **3.2 定义水冷、传感器与通道能力接口。** 目标：让泵、风扇、液温和流量使用明确的含义与单位。交付物：稳定设备/通道 ID，液温、流量、转速、占空比、控制模式、数据有效性和故障状态的接口约定；包含多通道映射及校准信息。验收：示例描述符能够区分只读、可写和不可用；模拟数据可验证单位转换、缺失数据及通道互不串写，结果仅记为软件验证。
- [ ] **3.3 接入真实 HID/CDC 发现与只读连接。** 目标：找到目标设备并可靠读取身份，不因插入普通 USB 设备而尝试控制。交付物：受限设备匹配、HID 报告/CDC 串口连接、超时、断开、重连和访问错误处理。验收：在列明的平台及设备上留下真实枚举、握手、只读采样与拔插记录；无法确认协议或板型时停止，重连不重放历史控制或升级命令。
- [ ] **3.4 验证第三方 USB AIO 与独立控制器读取。** 目标：在具体型号上读取泵/风扇转速、液温及设备状态。交付物：首个经核对的型号适配实现及原始协议记录；对 `liquidctl` 等候选给出接口方式、固定版本、许可和依赖评估。验收：本项目读数与设备自身或独立参考来源逐项比对，并记录偏差与不支持字段；不将上游工具能访问设备视为本项目已完成集成。
- [ ] **3.5 完成液温与流量测量验证。** 目标：让规则使用可解释、可判断失效的传感器数据。交付物：传感器量程、单位、刷新周期、校准依据、零值与断线语义、异常值和数据过期处理。验收：用已说明的参考测量方法核对读数；覆盖无数据、冻结、越界及真实零流量与传感器故障的区分；未完成校准的读数带明确状态，不参与未经验证的保护判断。
- [ ] **3.6 开放经过验证的控制通道。** 目标：用户能确认实物通道后安全控制第三方风扇或泵。交付物：按型号建立的安全范围、启动条件、控制模式、权限与冲突提示，以及设定值、实际 RPM/流量分别显示的闭环流程。验收：在另行授权的受控测试中逐通道验证控制、拒绝写入和恢复默认；禁止用通用默认下限代替型号验证，设备未提供写能力时保持只读。
- [ ] **3.7 验证失联保护与控制权交接。** 目标：应用崩溃、USB 断开、休眠或控制服务消失时，冷却行为可预期。交付物：每类设备的本地曲线/保底模式与超时约定、应用退出交接、故障状态和恢复步骤。验收：分别记录正常退出、强制结束、断线与休眠唤醒时的设定值和真实转速/流量；缺乏设备本地保护的型号不得宣称具备失联保护，也不得进入依赖该能力的无人值守控制范围。
- [ ] **3.8 并行子线：确定 OpenFan/OpenHub 参考板。** 目标：形成可复现并适合受控验证的自研硬件基线。交付物：用途与通道定义、电路、引脚、供电与负载边界、保护设计、BOM、板号/修订版，以及启动和恢复接口说明。验收：设计审查后逐项验证电源、接口与负载条件；参考板数据只对应已测修订版，不外推为量产或其他板型证明。
- [ ] **3.9 并行子线：实现参考固件与真实独立通道。** 目标：OpenFan/OpenHub 通过 ODP 提供真实读写及设备本地保护。交付物：可复现固件构建、真实描述符、独立通道寻址、传感器采样、局部控制回路、看门狗/失联策略和诊断日志。验收：在板型匹配且获授权的参考板上核对每个通道输入输出；验证单通道操作不影响其他通道，宿主停止服务后本地保护仍成立；模拟结果与板上结果分别归档。
- [ ] **3.10 并行子线：建立板型校验与固件恢复链。** 目标：为未来升级建立可阻止错刷且可从中断恢复的流程。交付物：板号/硬件修订版/bootloader 与镜像兼容检查、镜像完整性和授权校验方案、升级前状态检查、备份或可靠恢复镜像，以及升级中断恢复步骤。验收：先完成软件拒绝路径验证；再在另行授权的测试板上验证错板拒绝、坏包拒绝、传输中断和写入中断后的实际恢复；未经恢复验证不得开放常规升级入口，不涉及 OTP/eFuse 等永久写入。
- [ ] **3.11 集成 CLI、界面与自动化。** 目标：用户能在统一入口看清设备来源、通道归属、读取/控制权限及当前保护状态。交付物：设备详情、只读诊断、通道确认、规则配置、控制权状态与故障提示；第三方与自研设备使用同一能力接口。验收：液温可驱动已验证冷却通道；失效数据、权限拒绝、控制冲突和重新连接均有明确行为；演示设备有模拟标记，界面不把请求成功写成实物已响应。
- [ ] **3.12 完成设备兼容验收与受控发布。** 目标：形成用户可据此选型、安装和排障的支持范围。交付物：按型号/修订版/固件/平台及具体功能划分的兼容矩阵、接线与恢复说明、复现日志、已知限制和发布候选。验收：每项“已支持”都能追溯到对应实机记录；未测功能保留待验证状态，真实设备安装、读写与故障恢复全部按声明范围复测通过后才进入发布说明。

### 阶段完成条件

- 主线形成至少一个明确型号的 USB 水冷或独立控制器完整路径，并完成本阶段声明支持的液温/流量传感器验证；兼容矩阵明确哪些设备仅可读、哪些可控、哪些具有设备本地失联保护。
- OpenFan/OpenHub 并行子线按 3.8 确定的交付范围形成参考板与固件；对声明完成的每种板型，真实通道、本地保护及中断恢复均有独立记录。尚未完成的产品不得被“协议完成”或另一块参考板的结果覆盖。
- CLI/界面与自动化能解释异常与控制权；设备身份、许可、实机证据、文档和发布产物可相互追溯。模拟与软件测试仅证明相应软件层行为。

### 边界

不承诺任意 USB AIO 或第三方品牌通用兼容；不默认采用闭源 SDK、不静默安装或修改驱动、不将提高权限当作所有失败的解决办法。固件与板上实验是未来受控工作计划，当前没有由本 Issue 授予的硬件写入授权。灯效和屏幕产品功能由 [第四阶段](https://github.com/SvenKunkka/OpenHardwareOS/issues/5)承接。

---

# English

## Stage 3: USB Liquid Cooling, Sensors, and In-House Controllers

Related: [Six-stage parent issue](https://github.com/SvenKunkka/OpenHardwareOS/issues/1) · [Stage 2](https://github.com/SvenKunkka/OpenHardwareOS/issues/3) · [Stage 4](https://github.com/SvenKunkka/OpenHardwareOS/issues/5)

Status: planning draft, not yet scheduled. This stage is not tied to a release version. The milestones below are tracked within this issue; separate issues are not created automatically.

### User outcome

Users can connect verified USB all-in-one liquid coolers, standalone fan/pump controllers, and coolant temperature/flow sensors; identify each channel; read its actual state first; and then apply safety policies to channels confirmed to be controllable. If the connection is lost, the computer sleeps, or the application exits, the device should enter a verified local protection state, with a clear indication of who currently controls it.

OpenFan/OpenHub is an in-house hardware workstream running in parallel within this stage. It will produce reproducible reference boards, a device protocol, and firmware, sharing the capability model, automation, and acceptance records used for third-party devices. It does not create a seventh stage.

### Current evidence and gaps

- [Device and transport types](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/device.rs), the [capability model](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/capability.rs), [pump safety policies](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-runtime/src/safety.rs), and a [simulated pump](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/mock/src/devices.rs) already exist. The current default pump floor is a software policy, not a safety specification for arbitrary pump models.
- The [LHM mapping](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/libre-hardware-monitor/src/mapping.rs) can map channels whose names contain `pump` to pumps. Name matching and percentage setpoint readback do not establish the physical wiring, rotational speed, or flow.
- The [ODP adapter](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/open-protocol/src/lib.rs) currently uses a fixed `LoopbackTransport<MockOpenFan>`. The [protocol transport framework](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-protocol/src/transport.rs) has a byte-stream abstraction, but real HID/CDC enumeration and connection workflows are not integrated.
- Connection-loss protection, the bootloader, and updates in [MockOpenFan](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-protocol/src/mock.rs) are simulated logic. The current "multichannel" simulation does not demonstrate independent control of real channels.
- USB AIO vendor protocols, coolant temperature/flow capabilities, reference-board circuitry, real-device firmware, and power-loss recovery still require implementation or verification. `liquidctl` is only an integration candidate and has not been integrated into this project. No upstream support list may be copied directly into this project's compatibility list.

### Scope

The main workstream covers USB AIO devices, standalone fan/pump controllers, and coolant temperature and flow sensors checked model by model. The parallel workstream covers OpenFan/OpenHub reference boards and ODP firmware. Validation scope must be recorded separately for each desktop operating system; results on one platform do not establish support on another. Data interfaces for lighting and displays may be reserved, but the corresponding product features belong to Stage 4.

### Dependencies

- This stage depends on the fan/pump identities, channel mappings, safety floors, control handover, and failure-handling agreements established in [Stage 2](https://github.com/SvenKunkka/OpenHardwareOS/issues/3). Any unfinished prerequisites must be completed before the corresponding control work begins.
- Before real-hardware validation, obtain the exact model, board identifier/revision, firmware version, USB descriptors, wiring and power information, and evidence of the applicable protocol/SDK usage and redistribution permissions.
- Writing in-house firmware separately requires a matching board type, an available recovery entry point, an original image or backup, a controlled procedure, and explicit authorization. This issue itself does not authorize any write or update to currently connected hardware.

### Milestones

- **3.1 Establish device candidates and validation records.** Goal: identify the first actual validation targets for this stage, avoiding broad brand-level support claims. Deliverables: a candidate list organized by model, board identifier, firmware, platform, USB interface, and function; protocol sources and licensing records; and read-only investigation results. Acceptance: every candidate proceeding to implementation has a traceable identity and a documented basis for access. Unknowns are explicitly marked, and unverified models are excluded from the compatibility list.
- **3.2 Define capability interfaces for liquid cooling, sensors, and channels.** Goal: give pumps, fans, coolant temperature, and flow unambiguous meanings and units. Deliverables: stable device/channel IDs and interface agreements for coolant temperature, flow, rotational speed, duty cycle, control mode, data validity, and fault state, including multichannel mappings and calibration information. Acceptance: example descriptors distinguish read-only, writable, and unavailable capabilities. Simulated data verifies unit conversion, missing-data handling, and isolation between channel writes; the results are recorded only as software validation.
- **3.3 Integrate real HID/CDC discovery and read-only connections.** Goal: find target devices and reliably read their identities without attempting control merely because an ordinary USB device was connected. Deliverables: restricted device matching, HID report/CDC serial connections, and handling for timeouts, disconnection, reconnection, and access errors. Acceptance: retain real enumeration, handshake, read-only sampling, and unplug/replug records for the listed platforms and devices. Stop when the protocol or board type cannot be confirmed; reconnection must not replay previous control or update commands.
- **3.4 Verify reads from third-party USB AIO devices and standalone controllers.** Goal: read pump/fan speeds, coolant temperature, and device state on specific models. Deliverables: the first implementation for a checked device model and raw protocol records; for candidates such as `liquidctl`, an assessment of the integration interface, pinned version, licensing, and dependencies. Acceptance: compare each project reading with the device's own output or an independent reference source, recording deviations and unsupported fields. An upstream tool's ability to access a device is not evidence that integration into this project is complete.
- **3.5 Complete coolant temperature and flow measurement validation.** Goal: provide rules with interpretable sensor data whose failures can be detected. Deliverables: sensor ranges, units, refresh intervals, calibration basis, zero-value and disconnection semantics, and handling for outliers and stale data. Acceptance: check readings using a documented reference measurement method. Cover absent, frozen, and out-of-range data, and distinguish actual zero flow from sensor failure. Readings without completed calibration carry an explicit status and are not used for unverified protection decisions.
- **3.6 Enable verified control channels.** Goal: let users safely control third-party fans or pumps after confirming the physical channels. Deliverables: model-specific safe ranges, startup conditions, control modes, permission and conflict messages, and a closed-loop workflow that displays setpoints separately from actual RPM/flow. Acceptance: under separately authorized, controlled tests, verify control, write rejection, and restoration of defaults channel by channel. A generic default floor must not substitute for model-specific validation; devices without write capabilities remain read-only.
- **3.7 Verify connection-loss protection and control handover.** Goal: make cooling behavior predictable when the application crashes, USB disconnects, the computer sleeps, or the control service disappears. Deliverables: local curves/fallback modes and timeout agreements for each device class, application-exit handover, fault states, and recovery steps. Acceptance: separately record setpoints and actual speed/flow during normal exit, forced termination, disconnection, and sleep/resume. Models without device-local protection must not be advertised as having connection-loss protection or admitted to unattended control that depends on that capability.
- **3.8 Parallel workstream: define OpenFan/OpenHub reference boards.** Goal: establish a reproducible in-house hardware baseline suitable for controlled validation. Deliverables: intended uses and channel definitions, schematics, pin assignments, power and load limits, protection design, BOM, board identifiers/revisions, and documentation of boot and recovery interfaces. Acceptance: after design review, verify power, interfaces, and load conditions individually. Reference-board data applies only to the tested revision and must not be presented as evidence for production readiness or other board types.
- **3.9 Parallel workstream: implement reference firmware and genuinely independent channels.** Goal: provide real reads/writes and device-local protection on OpenFan/OpenHub through ODP. Deliverables: reproducible firmware builds, real descriptors, independent channel addressing, sensor sampling, local control loops, watchdog/connection-loss policies, and diagnostic logs. Acceptance: check each channel's inputs and outputs on a matching, authorized reference board. Verify that operating one channel does not affect the others and that local protection remains effective after the host service stops. Archive simulation and on-board results separately.
- **3.10 Parallel workstream: establish board validation and firmware recovery.** Goal: provide a future update process that prevents flashing the wrong board and can recover from interruption. Deliverables: compatibility checks between board identifier/hardware revision/bootloader and image; image integrity and authorization verification plans; pre-update state checks; a backup or reliable recovery image; and update-interruption recovery steps. Acceptance: first verify software rejection paths. Then, on separately authorized test boards, verify rejection of incorrect board targets and corrupt packages, and actual recovery after interrupted transfers and interrupted writes. Do not enable routine updates before recovery is verified; permanent writes such as OTP/eFuse operations are excluded.
- **3.11 Integrate the CLI, UI, and automation.** Goal: provide one entry point where users can understand device sources, channel assignments, read/control permissions, and current protection states. Deliverables: device details, read-only diagnostics, channel confirmation, rule configuration, control-ownership states, and fault messages, with third-party and in-house devices using the same capability interface. Acceptance: coolant temperature can drive verified cooling channels. Invalid data, permission denials, control conflicts, and reconnection have defined behavior. Demo devices are marked as simulated, and the UI does not describe a successful request as a confirmed physical response.
- **3.12 Complete device compatibility acceptance and a controlled release.** Goal: establish a support scope users can rely on for selection, installation, and troubleshooting. Deliverables: a compatibility matrix organized by model/revision/firmware/platform and specific function; wiring and recovery instructions; reproduction logs; known limitations; and a release candidate. Acceptance: every "supported" claim is traceable to the corresponding real-hardware record. Untested functions remain pending validation. Real-device installation, reads/writes, and fault recovery enter the release notes only after retesting passes for the claimed scope.

### Stage completion criteria

- The main workstream delivers a complete path for at least one specific USB liquid-cooling device or standalone controller model and completes validation of the coolant temperature/flow sensors claimed as supported in this stage. The compatibility matrix identifies devices that are read-only, controllable, or equipped with device-local connection-loss protection.
- The OpenFan/OpenHub parallel workstream delivers reference boards and firmware within the scope defined in 3.8. Every board type claimed as complete has separate records for real channels, local protection, and recovery from interruption. Unfinished products must not be treated as complete because the "protocol is complete" or another reference board passed.
- The CLI/UI and automation explain faults and control ownership. Device identities, licensing, real-hardware evidence, documentation, and release artifacts are mutually traceable. Simulation and software tests demonstrate only the behavior of the corresponding software layer.

### Boundaries

This stage does not promise universal compatibility with arbitrary USB AIO devices or third-party brands. It does not default to closed-source SDKs, silently install or modify drivers, or treat elevated privileges as the solution to every failure. Firmware work and on-board experiments are future controlled activities; this issue grants no current authorization for hardware writes. Lighting effects and display product features are handled by [Stage 4](https://github.com/SvenKunkka/OpenHardwareOS/issues/5).


> 快照：GitHub Issue #1 的正文镜像（逐字节复制），抓取于 2026-09-14 18:45 CST / 2026-09-14T10:45:43Z。**实时进度以 Issue 里的复选框为准**，本文件不会自动同步。
>
> Snapshot: a byte-for-byte mirror of the body of GitHub Issue #1, captured 2026-09-14T10:45:43Z. The issue's checkboxes are the live progress record; this file does not auto-sync.

<!--
source:      https://github.com/SvenKunkka/OpenHardwareOS/issues/1
title:       [Roadmap / 路线图] OpenHardwareOS Hardware Support: Six Stages / 硬件支持六阶段总任务
captured_at: 2026-09-14T10:45:43Z (2026-09-14 18:45 CST)
repo_commit: 2aea128b1bea6170b801a1a1af4724d0efa0b500
authority:   the GitHub issue is authoritative for progress; this file is the versioned snapshot.
-->

**语言 / Language:** [中文](#中文) · [English](#english)

两种语言使用相同的小阶段编号；中文区的复选框是共用进度记录。
Both versions use the same substage IDs. The checkboxes in the Chinese section are the shared progress record.

# 中文

## OpenHardwareOS 硬件支持六阶段总任务

### 总目标

让用户在同一套界面和 CLI 中发现电脑及桌面硬件、查看可信状态、控制已验证的能力、建立联动，并在异常或退出时知道设备由谁控制。阶段完成以可复查的用户结果和实机证据为准。

本任务将完整硬件支持路线拆为 **6 个大阶段、50 个小阶段**。小阶段的目标、交付物、依赖和验收条件写在对应阶段任务描述中，以未勾选检查项逐项跟踪。OpenFan/OpenHub 是第三阶段的并行工作线。

### 当前基线与推进顺序

- 已公开发布 [v0.1.1 预览版](https://github.com/SvenKunkka/OpenHardwareOS/releases/tag/v0.1.1)，已具备运行时、规则、安全策略、桌面和 CLI，以及 System、LHM、NVIDIA 等访问路径。
- 软件构建与 Windows 安装通过不等于某个硬件型号已通过兼容验收。现有版本清单仍将实机验证标为待验证。
- **下一步：阶段一，目标 v0.2.0 单水泵支持。** 这是唯一已登记的下一发行版本，尚未发布。
- **后续：阶段二、阶段三。** 先完善整机散热，再扩 USB 水冷/传感器；第三阶段的自研控制板调查与原型可以并行，但实机控制依赖相应接口与保护条件。
- **远期：阶段四、阶段五、阶段六。** 进入开发前需要具体场景、参考型号、接口及资源依据。后续版本号、人员、采购预算和日期尚未分配。

### 六个阶段

- [ ] **[[阶段一] v0.2.0 单水泵支持](https://github.com/SvenKunkka/OpenHardwareOS/issues/2)**（8 个小阶段）：在 Windows 上完成一台指定水泵的监测、受限控制、恢复与发布验收。

  小阶段：S1.1 建立目标设备与测试环境记录（对应 PUMP-01）；S1.2 固定泵身份及 RPM／Control 通道映射（对应 PUMP-01）；S1.3 完成 CLI 与界面的只读泵监测（对应 PUMP-01）；S1.4 接通受限手动控制与规则控制（对应 PUMP-02）；S1.5 建立请求、回读与实际转速的确认链（对应 PUMP-02）；S1.6 补齐泵故障与恢复的软件场景（对应 PUMP-03）；S1.7 验证规则停止、退出和重连后的控制归属（对应 PUMP-03）；S1.8 完成指定设备验收与 v0.2.0 发布记录（对应 PUMP-04）。

- [ ] **[[阶段二] 完善整机散热](https://github.com/SvenKunkka/OpenHardwareOS/issues/3)**（8 个小阶段）：完善整机监测、多接口风扇与多风扇显卡，验证身份、控制权、后台运行和恢复。

  小阶段：2.1 确定整机兼容矩阵与验证入口；2.2 建立持久物理身份与跨适配器去重；2.3 补齐 CPU／GPU／主板／内存／存储的监测完整性；2.4 扩展并验证多主板、多风扇接口控制；2.5 完善多风扇 GPU 的真实通道模型；2.6 把已有单目标所有权和安全交接验证到整机通道；2.7 明确后台执行、权限与安装生命周期；2.8 完成原厂软件共存、恢复及整机验收闭环。

- [ ] **[[阶段三] USB 水冷、传感器与自研控制器](https://github.com/SvenKunkka/OpenHardwareOS/issues/4)**（12 个小阶段）：接入真实 USB 水冷与传感器，并行完成 OpenFan/OpenHub 参考控制器与固件验证。

  小阶段：3.1 建立设备候选与验证档案；3.2 定义水冷、传感器与通道能力接口；3.3 接入真实 HID/CDC 发现与只读连接；3.4 验证第三方 USB AIO 与独立控制器读取；3.5 完成液温与流量测量验证；3.6 开放经过验证的控制通道；3.7 验证失联保护与控制权交接；3.8 并行子线：确定 OpenFan/OpenHub 参考板；3.9 并行子线：实现参考固件与真实独立通道；3.10 并行子线：建立板型校验与固件恢复链；3.11 集成 CLI、界面与自动化；3.12 完成设备兼容验收与受控发布。

- [ ] **[[阶段四] 灯光与屏幕](https://github.com/SvenKunkka/OpenHardwareOS/issues/5)**（8 个小阶段）：完成首款灯光控制器和真实硬件屏幕的场景、显示、联动与恢复。

  小阶段：4.1 核对候选设备、接口与电气边界；4.2 建立灯区与屏幕能力约定；4.3 接通首批真实灯光控制器；4.4 完成灯光场景、控制权与异常恢复；4.5 接通硬件屏幕的静态显示路径；4.6 形成硬件状态仪表盘与用户内容流程；4.7 验证灯光、屏幕与散热并行运行；4.8 完成用户安装、兼容矩阵与发布验收。

- [ ] **[[阶段五] 输入设备与电源](https://github.com/SvenKunkka/OpenHardwareOS/issues/6)**（7 个小阶段）：接入输入设备事件与电源遥测，让设备操作触发可追溯的硬件场景。

  小阶段：5.1 明确两类参考设备与接口；5.2 稳定识别输入设备并读取状态；5.3 接收按键、旋钮和其他明确事件；5.4 将输入映射到可审计的硬件场景；5.5 接入电源只读遥测；5.6 完成告警与控制范围判定；5.7 发布输入与电源兼容记录。

- [ ] **[[阶段六] 平台扩展与开放生态](https://github.com/SvenKunkka/OpenHardwareOS/issues/7)**（7 个小阶段）：验证新的设备方向，稳定适配接口、硬件及固件 SDK、兼容目录与平台发行规则。

  小阶段：6.1 确认扩展场景和参考型号；6.2 接入 NPU 或 AI 加速器只读监测；6.3 验证桌面机械设备的状态与受限动作；6.4 固定适配器接口与插件运行边界；6.5 交付固件与硬件 SDK；6.6 建立兼容测试与设备目录；6.7 扩展平台并形成稳定发布规则。

### 阶段依赖

第一阶段完成一台泵的闭环；第二阶段把设备身份、控制权和后台运行完善为整机基础。第三阶段扩展 USB 水冷、传感器与真实自研控制器。第四阶段接入灯光和屏幕，第五阶段接入输入设备和电源，第六阶段在真实设备经验之上稳定 SDK、兼容目录并评估 AI 和桌面机械设备。

监测覆盖作为贯穿工作线：CPU、GPU、主板、内存、存储在第二阶段细化；液温/流量在第三阶段；电源在第五阶段；NPU/AI 加速器在第六阶段。温度、电流、湿度和建议增加的漏液检测按具体接口与样机分别形成能力记录。

前置关系约束的是相应功能的实现与验收，不阻止后续只读接口调查。普通输入协议或 OpenRGB 等独立路径可在满足各自依赖后开展，不要求等待所有自研硬件完成。

### 每款硬件的统一交付标准

每个进入兼容清单的型号必须提供支持卡：

| 项目 | 必须写清的内容 |
|---|---|
| 身份 | 型号、硬件修订版、固件、主板/控制器、接口与通道 |
| 环境 | 操作系统、应用版本、适配器及驱动/桥接工具版本 |
| 能力 | 可识别、可读取、可控制、可联动的具体项目；只读与可写分开 |
| 结果 | 请求是否接受、设定值是否回读、实际 RPM/流量/灯光/屏幕等是否响应 |
| 异常 | 数据过期、拒写、断连、重连、停止规则、退出和恢复的实际结果 |
| 追溯 | 复现步骤、测试证据、已知限制、首次支持版本、源码提交 |

模拟通过、传输成功、设定值回读和物理效果验证分别记录。勾选一个小阶段需要关联其交付物与验收证据；完成一个阶段需要满足该 Issue 的完成定义。只有实际发布并核对来源后，才能更新版本树的“已发布”状态。

### 平台与版本管理

优先 Windows x64，随后按设备路径扩展 Linux 和 macOS。每个平台分别记录本机传感器和外接设备的支持范围，不把跨平台编译当成跨平台硬件兼容。

任务编号与软件版本号分开管理；现有路线文档中的 C1–C8 是能力分类，不是本任务的六阶段，也不是发行承诺。保留 v0.1.0/v0.1.1 的原有 tag 与下载文件。

### 维护入口

- [交互版本树](https://svenkunkka.github.io/OpenHardwareOS/)
- [版本清单](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/versions.json)
- [现有水泵计划](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/plans/pump-support.md)
- [能力路线图](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/roadmap.md)
- [版本与发行记录流程](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/version-management.md)

本任务登记目标与验收范围，不启动采购、刷写或真实设备控制。未来执行以当时的明确设备范围和用户授权为准；未确定的型号与资源保持待确认，不填写虚构负责人、进度或日期。

---

# English

## OpenHardwareOS: Six-Stage Hardware Support Roadmap

### Overall goal

Enable users to discover PC and desktop hardware through one interface and CLI, see trustworthy status, control verified capabilities, configure coordinated actions, and understand who controls each device after a fault or application exit. Stage completion requires reproducible user outcomes and evidence from real hardware.

This roadmap contains **6 major stages and 50 substages**. Each stage issue defines its substage goals, deliverables, dependencies, and acceptance criteria. The shared checklist tracks them individually. OpenFan/OpenHub is a parallel workstream within Stage 3.

### Current baseline and sequence

- The [v0.1.1 preview release](https://github.com/SvenKunkka/OpenHardwareOS/releases/tag/v0.1.1) is public. It includes the runtime, rules, safety policies, desktop application, CLI, and access paths such as System, LHM, and NVIDIA.
- Passing software builds and Windows installation tests does not establish compatibility for a particular hardware model. The current version catalog still marks real-hardware validation as pending.
- **Next: Stage 1, targeting v0.2.0 single-pump support.** This is the only registered next release and has not been published.
- **Following stages: Stage 2 and Stage 3.** Improve whole-PC cooling, then extend to USB liquid cooling and sensors. Investigation and prototyping of the Stage 3 reference controller boards can proceed in parallel; real-device control depends on the relevant interfaces and safeguards.
- **Longer term: Stages 4, 5, and 6.** Development requires specific use cases, reference models, interfaces, and resource evidence. Subsequent version numbers, people, procurement budgets, and dates have not been assigned.

### The six stages

- **[Stage 1: v0.2.0 Single-Pump Support](https://github.com/SvenKunkka/OpenHardwareOS/issues/2)** — 8 substages. Complete monitoring, bounded control, recovery, and release acceptance for one specified pump on Windows.

  Substages: S1.1 Record the target device and test environment (PUMP-01); S1.2 Establish persistent pump identity and RPM/Control channel mapping (PUMP-01); S1.3 Complete read-only pump monitoring in the CLI and UI (PUMP-01); S1.4 Connect bounded manual and rule-based control (PUMP-02); S1.5 Establish a confirmation chain from request to readback to actual RPM (PUMP-02); S1.6 Cover software scenarios for pump faults and recovery (PUMP-03); S1.7 Verify control ownership after rule stop, application exit, and reconnection (PUMP-03); S1.8 Complete acceptance for the specified device and v0.2.0 release records (PUMP-04).

- **[Stage 2: Whole-PC Cooling](https://github.com/SvenKunkka/OpenHardwareOS/issues/3)** — 8 substages. Improve whole-PC monitoring, multiple fan headers, and multi-fan GPUs; verify identity, control ownership, background operation, and recovery.

  Substages: 2.1 Define the whole-PC compatibility matrix and validation prerequisites; 2.2 Establish persistent physical identity and deduplication across adapters; 2.3 Complete monitoring coverage and data integrity for CPU, GPU, motherboard, memory, and storage; 2.4 Extend and verify control across multiple motherboards and fan headers; 2.5 Improve the real channel model for multi-fan GPUs; 2.6 Validate existing single-target ownership and safe handover across whole-PC channels; 2.7 Define background execution, privileges, and the installation lifecycle; 2.8 Complete vendor-software coexistence, recovery, and end-to-end whole-PC acceptance.

- **[Stage 3: USB Liquid Cooling, Sensors, and Reference Controllers](https://github.com/SvenKunkka/OpenHardwareOS/issues/4)** — 12 substages. Integrate real USB liquid-cooling devices and sensors while validating OpenFan/OpenHub reference controllers and firmware in parallel.

  Substages: 3.1 Create candidate-device and validation records; 3.2 Define capability interfaces for liquid cooling, sensors, and channels; 3.3 Integrate real HID/CDC discovery and read-only connections; 3.4 Verify telemetry from third-party USB AIOs and standalone controllers; 3.5 Validate coolant-temperature and flow measurements; 3.6 Enable verified control channels; 3.7 Verify communication-loss protection and control handover; 3.8 Parallel workstream: define OpenFan/OpenHub reference boards; 3.9 Parallel workstream: implement reference firmware and real independent channels; 3.10 Parallel workstream: establish board matching and firmware recovery; 3.11 Integrate the CLI, UI, and automation; 3.12 Complete device compatibility acceptance and a controlled release.

- **[Stage 4: Lighting and Displays](https://github.com/SvenKunkka/OpenHardwareOS/issues/5)** — 8 substages. Complete scenes, display output, coordinated actions, and recovery for the first lighting controller and a real hardware display.

  Substages: 4.1 Verify candidate devices, interfaces, and electrical limits; 4.2 Define lighting-zone and display capabilities; 4.3 Integrate the first real lighting controllers; 4.4 Complete lighting scenes, control ownership, and fault recovery; 4.5 Connect static rendering to real hardware displays; 4.6 Provide hardware-status dashboards and a user-content workflow; 4.7 Verify simultaneous lighting, display, and cooling operation; 4.8 Complete user installation, the compatibility matrix, and release acceptance.

- **[Stage 5: Input Devices and Power Supplies](https://github.com/SvenKunkka/OpenHardwareOS/issues/6)** — 7 substages. Integrate input events and PSU telemetry so device interactions can trigger traceable hardware scenes.

  Substages: 5.1 Define reference devices and interfaces for both categories; 5.2 Reliably identify input devices and read status; 5.3 Receive key, knob, and other explicitly defined events; 5.4 Map input to auditable hardware scenes; 5.5 Integrate read-only PSU telemetry; 5.6 Complete alerts and determine control scope; 5.7 Publish input-device and PSU compatibility records.

- **[Stage 6: Platform Expansion and an Open Ecosystem](https://github.com/SvenKunkka/OpenHardwareOS/issues/7)** — 7 substages. Validate new device directions and stabilize adapter interfaces, hardware and firmware SDKs, the compatibility catalog, and platform release policies.

  Substages: 6.1 Confirm expansion use cases and reference models; 6.2 Integrate read-only monitoring for an NPU or AI accelerator; 6.3 Validate status and bounded actions for desktop mechanical devices; 6.4 Stabilize adapter interfaces and plugin execution boundaries; 6.5 Deliver firmware and hardware SDKs; 6.6 Establish compatibility tests and a device catalog; 6.7 Expand platform coverage and define stable-release policies.

### Stage dependencies

Stage 1 completes the end-to-end workflow for one pump. Stage 2 develops device identity, control ownership, and background operation into a whole-PC foundation. Stage 3 extends USB liquid cooling, sensors, and real reference controllers. Stage 4 integrates lighting and displays; Stage 5 integrates input devices and power supplies. Stage 6 builds on real-device experience to stabilize SDKs and the compatibility catalog and evaluate AI and desktop mechanical devices.

Monitoring coverage runs across the roadmap: Stage 2 covers CPU, GPU, motherboard, memory, and storage in detail; Stage 3 covers coolant temperature and flow; Stage 5 covers power supplies; Stage 6 covers NPUs and AI accelerators. Temperature, current, humidity, and the proposed addition of leak detection receive separate capability records for the specific interfaces and test devices.

Prerequisites govern implementation and acceptance of the corresponding features; they do not prevent later read-only interface investigation. Independent paths, such as standard input protocols or OpenRGB, can proceed once their own dependencies are met, without waiting for all reference hardware to be finished.

### Common delivery requirements for each hardware model

Every model added to the compatibility list must have a support record:

| Area | Required information |
|---|---|
| Identity | Model, hardware revision, firmware, motherboard/controller, interfaces, and channels |
| Environment | Operating system, application version, adapter version, and driver/bridge-tool versions |
| Capabilities | Exactly what can be identified, read, controlled, and coordinated; read-only and writable capabilities listed separately |
| Results | Whether requests were accepted, setpoints were read back, and actual RPM, flow, lighting, display output, or other physical behavior responded |
| Faults | Actual outcomes for stale data, rejected writes, disconnection, reconnection, rule stop, application exit, and recovery |
| Traceability | Reproduction steps, test evidence, known limitations, first supported release, and source commit |

Record simulation success, transport success, setpoint readback, and verification of physical effects separately. Checking off a substage requires linked deliverables and acceptance evidence. Completing a stage requires its issue's definition of done. The version tree may show a release as published only after actual publication and source verification.

### Platforms and version management

Prioritize Windows x64, then extend Linux and macOS according to the device access path. Record support for local sensors and external devices separately on each platform. Cross-platform compilation does not establish cross-platform hardware compatibility.

Keep task numbers separate from software version numbers. C1–C8 in the existing roadmap are capability categories, not these six execution stages or release commitments. Preserve the existing v0.1.0/v0.1.1 tags and download assets.

### Maintenance references

- [Interactive version tree](https://svenkunkka.github.io/OpenHardwareOS/)
- [Version catalog](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/versions.json)
- [Existing pump plan](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/plans/pump-support.md)
- [Capability roadmap](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/roadmap.md)
- [Version and release-record workflow](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/version-management.md)

This issue records goals and acceptance scope. It does not initiate procurement, flashing, or real-device control. Future execution follows the device scope and user authorization applicable at that time. Unconfirmed models and resources remain pending; do not invent owners, progress, or dates.


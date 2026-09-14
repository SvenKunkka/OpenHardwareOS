> 快照：GitHub Issue #7 的正文镜像（逐字节复制），抓取于 2026-09-14 18:45 CST / 2026-09-14T10:45:43Z。**实时进度以 Issue 里的复选框为准**，本文件不会自动同步。
>
> Snapshot: a byte-for-byte mirror of the body of GitHub Issue #7, captured 2026-09-14T10:45:43Z. The issue's checkboxes are the live progress record; this file does not auto-sync.

<!--
source:      https://github.com/SvenKunkka/OpenHardwareOS/issues/7
title:       [Stage 6 / 阶段六] Platform Expansion & Open Ecosystem / 平台扩展与开放生态
captured_at: 2026-09-14T10:45:43Z (2026-09-14 18:45 CST)
repo_commit: 2aea128b1bea6170b801a1a1af4724d0efa0b500
authority:   the GitHub issue is authoritative for progress; this file is the versioned snapshot.
-->

**语言 / Language:** [中文](#中文) · [English](#english)

两种语言使用相同的小阶段编号；中文区的复选框是共用进度记录。
Both versions use the same substage IDs. The checkboxes in the Chinese section are the shared progress record.

# 中文

## 阶段六：平台扩展与开放生态

### 要达到的结果

第三方能够按公开接口接入新硬件，用户能够依据兼容记录判断它可以监测什么、控制什么，以及出错后如何恢复。AI 加速器/NPU 先形成可信监测示例；桌面机械设备作为单独验证的扩展方向。

状态：远期规划，尚未排期、分配负责人或绑定发行版本。该阶段包含待验证的产品方向，不表示已经找到兼容型号、供应商或客户需求。

总任务：https://github.com/SvenKunkka/OpenHardwareOS/issues/1

### 当前基础与入口依赖

- 公开版本已有设备模型、运行时、适配器 trait 和模拟 ODP；没有动态插件加载、NPU 专用适配或桌面运动控制实现。
- 插件与认证工作依赖[阶段二](https://github.com/SvenKunkka/OpenHardwareOS/issues/3)的稳定身份/权限基础、[阶段三](https://github.com/SvenKunkka/OpenHardwareOS/issues/4)的真实 ODP 参考设备，以及[阶段四](https://github.com/SvenKunkka/OpenHardwareOS/issues/5)、[阶段五](https://github.com/SvenKunkka/OpenHardwareOS/issues/6)带来的跨设备类别实践。
- 每条扩展线在开工前确定用户场景、参考型号、接口、维护责任和成本范围。没有达到入口条件的方向继续保留为计划，不阻塞已经可用的散热发行版。

### 小阶段目标与验收

- [ ] **6.1 确认扩展场景和参考型号。** 目标：把远期方向收敛成可验证任务。交付：NPU/AI 加速器和桌面机械设备各自的使用场景、现有痛点、接口调查、样机与工程资源清单。验收：每条拟启动线有真实需求或试用反馈，以及明确可取得的接口/样机；缺少依据的方向保持未启动。

- [ ] **6.2 接入 NPU 或 AI 加速器只读监测。** 目标：先可靠展示一款参考设备的实际运行状态。交付：设备识别和接口实际提供的负载、内存、温度、功耗等字段，以及权限、刷新频率、不可用原因。验收：至少一款目标设备的可读指标与来源接口核对；未提供的指标不估算成实测值，不把现有 GPU 监测冒充 NPU 适配。

- [ ] **6.3 验证桌面机械设备的状态与受限动作。** 目标：独立验证一款具有可用接口的升降桌或电动支架。交付：设备状态、单位、原点/限位、允许动作、停止、连接丢失行为及控制归属说明。验收：先完成只读和模拟验证，再按目标设备条件做受限实测；位置、运动方向、停止和异常行为均有证据。没有可靠停止/保护接口时仅开放监测，不绕过原机限位或防夹保护。

- [ ] **6.4 固定适配器接口与插件运行边界。** 目标：新增设备适配不需要修改核心业务逻辑，并且插件故障可隔离。交付：接口版本、能力声明、权限/进程边界、安装卸载、版本协商、错误与恢复约定；选择并记录插件运行方式。验收：一个样例插件通过安装、启停、升级、拒绝不兼容版本及崩溃恢复测试；过期动作不重放，插件不能绕过硬件写入保护。

- [ ] **6.5 交付固件与硬件 SDK。** 目标：设备开发者可以复现一套真实参考设备。交付：ODP 固件示例、能力描述模板、本地失联保护、硬件参考设计/BOM/接口约束、构建和恢复说明。验收：从干净环境构建并在阶段三的明确板型上运行；板型和固件兼容检查有效，升级中断恢复有实物证据；依赖来源和分发材料可追踪。

- [ ] **6.6 建立兼容测试与设备目录。** 目标：支持声明能被复查，并随版本持续维护。交付：协议一致性测试、读取/控制/恢复验收模板、兼容设备目录、固件与应用版本矩阵、已知限制和撤销支持流程。验收：至少两类真实设备使用同一套规范完成登记；模拟通过、监测通过、控制通过和恢复通过分别标注；标签或认证只对应明确测试范围。

- [ ] **6.7 扩展平台并形成稳定发布规则。** 目标：Windows、Linux、macOS 各自有清楚的支持边界和可维护发行链路。交付：平台适配矩阵、安装升级/卸载说明、长期运行报告、兼容变更和弃用规则、贡献与问题反馈流程。验收：每个宣称支持的平台完成对应真实功能与安装验证；外接设备和本机传感器分别列范围；稳定版范围根据证据确定，不能因版本号达到某值就视为全部硬件已支持。

### 阶段完成定义

已选定的扩展设备线完成各自验收；公开 SDK、插件边界及兼容目录可由其他开发者复现。至少一款 AI 加速器/NPU 有真实监测证据，桌面机械设备分支有明确的监测或受限控制结论。无法取得可行接口的远期方向应通过任务范围修订明确延期，而不是勾选完成。

### 范围边界与证据入口

本阶段不承诺所有 NPU、所有桌面设备或三个系统功能完全相同。通用扩展坞、网络、音频、摄像头、UPS、BIOS/超频和任意固件刷写没有自动纳入范围。具体产品、采购预算、供应商与时间在各子方向启动前确定，本任务不构成采购或实机动作授权。

- [远期硬件域与 SDK 路线](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/roadmap.md)
- [设备模型](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/device-model.md)
- [ODP 协议与当前限制](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/protocol.md)
- [版本维护流程](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/version-management.md)

---

# English

## Stage 6: Platform Expansion and an Open Ecosystem

### Intended outcome

Third parties can integrate new hardware through public interfaces, and users can consult compatibility records to determine what can be monitored, what can be controlled, and how recovery works after an error. Establish trustworthy monitoring examples for AI accelerators/NPUs first. Treat desktop mechanical devices as a separately validated expansion direction.

Status: a long-term plan, with no schedule, owner, or release version assigned. This stage includes product directions that still need validation; it does not establish that compatible models, suppliers, or customer demand have already been found.

Parent issue: https://github.com/SvenKunkka/OpenHardwareOS/issues/1

### Existing foundation and entry dependencies

- The public release includes a device model, runtime, adapter trait, and simulated ODP. It has no implementation of dynamic plugin loading, dedicated NPU adapters, or desktop motion control.
- Plugin and certification work depends on the stable identity/permission foundations from [Stage 2](https://github.com/SvenKunkka/OpenHardwareOS/issues/3), real ODP reference devices from [Stage 3](https://github.com/SvenKunkka/OpenHardwareOS/issues/4), and experience across device categories from [Stage 4](https://github.com/SvenKunkka/OpenHardwareOS/issues/5) and [Stage 5](https://github.com/SvenKunkka/OpenHardwareOS/issues/6).
- Before each expansion workstream starts, define its user scenario, reference model, interface, maintenance responsibility, and cost range. Directions that do not meet their entry conditions remain planned and do not block usable cooling releases.

### Substage goals and acceptance criteria

- **6.1 Confirm expansion use cases and reference models.** Goal: turn long-term directions into verifiable tasks. Deliverables: separate use cases, current pain points, interface investigations, test-device lists, and engineering-resource lists for NPU/AI accelerators and desktop mechanical devices. Acceptance: every workstream proposed for development has genuine demand or trial feedback, plus identifiable interfaces and test devices that can be obtained. Directions without this evidence remain unstarted.

- **6.2 Integrate read-only monitoring for an NPU or AI accelerator.** Goal: reliably show the actual operating state of one reference device first. Deliverables: device identification and the load, memory, temperature, power, or other fields actually provided by its interface, together with permission requirements, refresh rates, and reasons for unavailable readings. Acceptance: cross-check the readable metrics of at least one target device against its source interface. Do not present estimates of unavailable metrics as measurements, or existing GPU monitoring as NPU integration.

- **6.3 Validate status and bounded actions for desktop mechanical devices.** Goal: independently validate one height-adjustable desk or motorized mount with an accessible interface. Deliverables: device status, units, home position/limits, permitted actions, stopping behavior, connection-loss behavior, and control-ownership documentation. Acceptance: complete read-only and simulation validation first, then bounded real-device tests under the target device's conditions. Provide evidence for position, movement direction, stopping, and fault behavior. Offer monitoring only if reliable stopping/protection interfaces are unavailable; do not bypass the original limits or anti-pinch protection.

- **6.4 Stabilize adapter interfaces and plugin execution boundaries.** Goal: allow new device adapters without changes to core business logic and isolate plugin failures. Deliverables: interface versions, capability declarations, permission/process boundaries, installation and removal, version negotiation, and error/recovery conventions; select and document the plugin execution model. Acceptance: a sample plugin passes installation, start/stop, upgrade, incompatible-version rejection, and crash-recovery tests. Stale actions are not replayed, and plugins cannot bypass hardware-write safeguards.

- **6.5 Deliver firmware and hardware SDKs.** Goal: enable device developers to reproduce a real reference device. Deliverables: ODP firmware examples, capability-description templates, local communication-loss protection, hardware reference designs/BOM/interface constraints, and build/recovery instructions. Acceptance: build from a clean environment and run on an explicitly identified Stage 3 board. Board/firmware compatibility checks work, and recovery from interrupted upgrades has physical-device evidence. Dependency sources and distributed materials are traceable.

- **6.6 Establish compatibility tests and a device catalog.** Goal: make support claims reviewable and maintain them across versions. Deliverables: protocol-conformance tests; reading/control/recovery acceptance templates; a compatible-device catalog; firmware/application version matrices; known limitations; and a process for withdrawing support. Acceptance: at least two real device categories are registered under the same specification. Mark simulation, monitoring, control, and recovery results separately. Labels or certification apply only to an explicit tested scope.

- **6.7 Expand platform coverage and define stable-release policies.** Goal: give Windows, Linux, and macOS explicit support boundaries and maintainable release processes. Deliverables: platform-adaptation matrices, installation/upgrade/removal instructions, long-running test reports, compatibility-change and deprecation policies, and contribution/issue-reporting processes. Acceptance: every platform claimed as supported passes the relevant real-function and installation validation. State external-device and local-sensor coverage separately. Define stable-release scope from evidence; reaching a particular version number does not establish support for all hardware.

### Definition of done

The selected expansion-device workstreams pass their respective acceptance criteria. Other developers can reproduce the public SDK, plugin-boundary, and compatibility-catalog workflows. At least one AI accelerator/NPU has real monitoring evidence. The desktop mechanical-device workstream has an explicit conclusion on monitoring or bounded control. Long-term directions without a feasible interface must be explicitly deferred through a scope revision, not checked off as complete.

### Scope boundaries and evidence references

This stage does not promise support for all NPUs, all desktop devices, or identical functionality across all three operating systems. General-purpose docks, networking, audio, cameras, UPS devices, BIOS/overclocking, and arbitrary firmware flashing are not automatically included. Determine the specific products, procurement budgets, suppliers, and timing before each workstream starts. This issue does not authorize procurement or real-device actions.

- [Long-term hardware domains and SDK roadmap](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/roadmap.md)
- [Device model](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/device-model.md)
- [ODP protocol and current limitations](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/protocol.md)
- [Version-maintenance workflow](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/version-management.md)


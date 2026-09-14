> 快照：GitHub Issue #2 的正文镜像（逐字节复制），抓取于 2026-09-14 18:45 CST / 2026-09-14T10:45:43Z。**实时进度以 Issue 里的复选框为准**，本文件不会自动同步。
>
> Snapshot: a byte-for-byte mirror of the body of GitHub Issue #2, captured 2026-09-14T10:45:43Z. The issue's checkboxes are the live progress record; this file does not auto-sync.

<!--
source:      https://github.com/SvenKunkka/OpenHardwareOS/issues/2
title:       [Stage 1 / 阶段一] v0.2.0 Single-Pump Support / 单水泵支持
captured_at: 2026-09-14T10:45:43Z (2026-09-14 18:45 CST)
repo_commit: 2aea128b1bea6170b801a1a1af4724d0efa0b500
authority:   the GitHub issue is authoritative for progress; this file is the versioned snapshot.
-->

**语言 / Language:** [中文](#中文) · [English](#english)

两种语言使用相同的小阶段编号；中文区的复选框是共用进度记录。
Both versions use the same substage IDs. The checkboxes in the Chinese section are the shared progress record.

# 中文

**状态：计划中。目标版本：v0.2.0。** 这是硬件支持路线的下一步；版本号对应本阶段范围，不代表已完成实机验证。目前没有登记为已验证的水泵型号。

总任务：https://github.com/SvenKunkka/OpenHardwareOS/issues/1  
后续阶段：https://github.com/SvenKunkka/OpenHardwareOS/issues/3（完善整机散热，未排期）

### 用户最终得到什么

在 Windows 上，用户能够准确找到一台指定型号、指定接法的真实水泵，查看 RPM，了解是否允许控制，在已确认的范围内调节占空比或使用规则，并清楚看到写入是否得到确认。停止规则、退出应用或遇到异常时，用户能够知道水泵由谁控制、采取了什么保护动作、是否成功恢复。

本阶段追求一台设备上可复查的完整闭环。支持声明必须包含型号、主板或控制器、接法、控制模式和软件环境；不能从这一台设备外推“所有水泵均支持”。

### 已有基础与缺口

- 已有 `Pump` 设备类型、`pump.rpm`、`pump.speed_percent`、规则引擎、最低占空比保护、故障状态及恢复机制，优先复用并核实其实际行为。
- LHM 适配器已有 HTTP 读写与设定值回读。现有泵识别和 RPM／Control 配对仍依赖名称、编号等信息，必须解决重名、重排及歧义映射，避免写错通道。
- 已有模拟泵、拒写、未确认和断连场景；停转、RPM 丢失、恢复场景需要补齐。当前模拟泵未接入热模型，模拟结果不能作为真实冷却能力的证明。
- 现有软件默认泵保护值 60% 不能当成所有型号的硬件要求。实际测试范围和恢复步骤应由目标设备信息及验证证据确定。

依据：[现有 v0.2.0 水泵计划](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/plans/pump-support.md)、[设备能力与适配器边界](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/device-model.md)、[自动化机制](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/automation.md)。

### 硬件范围

首个验收对象是一台通过现有 LHM 路径暴露明确 RPM 和 Control 通道的水泵。记录其主板或控制器接口及 PWM／DC 等实际模式，仅开放能够确认身份、控制范围和恢复方式的通道。

读取、请求占空比、设定值回读和实际 RPM 分开呈现。接口接受请求或设定值相符，不等于真实转速已达到目标；控制映射存在歧义时保留只读状态并解释原因。

### 入口依赖

- 选定目标水泵、主板或控制器及接法，记录 Windows、LHM、相关驱动版本。
- 获取首轮只读设备清单，以及 LHM 的设备路径、RPM 路径、Control 路径和可读／可写／可回读状态。
- 在任何实机控制前，确认该设备允许的控制模式、范围和恢复操作；无法确认时继续只读工作。
- 保留现有规则、安全策略、审计与恢复机制；新增逻辑须经过同一运行时入口。
- 软件故障场景先在模拟器和脱敏样本中验证，不通过把真实泵设为零或拔除冷却设备来制造故障。

### 小阶段检查列表

- [ ] **S1.1 建立目标设备与测试环境记录（对应 PUMP-01）**

  **目标：** 固定本阶段测试对象，明确哪些信息已知、未知及需要只读确认。

  **交付物：** 一份设备卡，包含水泵、主板或控制器型号、接法、控制模式、软件版本、LHM 路径、候选控制范围和恢复资料。

  **验收：** 每个身份字段都有来源；RPM、Control 与目标设备对应关系能够复查；未知项显式标注。未确认控制范围或恢复方法时，设备卡不能标为可控。

- [ ] **S1.2 固定泵身份及 RPM／Control 通道映射（对应 PUMP-01）**

  **目标：** 消除仅靠名称、显示顺序或松散编号配对造成的误控制风险。

  **交付物：** 映射规则、来源可追踪的脱敏 LHM 样本、歧义处理说明和自动化测试。

  **验收：** 样本覆盖单泵、多通道、无编号、重名、枚举重排、断开后重连；能够唯一确认的泵保持身份一致，不能唯一确认的通道不开放控制；历史请求不会因重排转移到另一个通道。

- [ ] **S1.3 完成 CLI 与界面的只读泵监测（对应 PUMP-01）**

  **目标：** 用户能识别目标泵，并分辨有效 RPM、零 RPM、数据过期和数据缺失。

  **交付物：** CLI／界面中的泵名称、RPM、单位、更新时间、读写能力及不可用原因显示；配套界面和数据转换测试。

  **验收：** 指定实机的设备显示与 S1.1 设备卡一致，RPM 可与同一时段的 LHM 读数核对；缺失读数不显示为零，设定占空比不被当成实际 RPM，模拟设备始终有清晰标记。

- [ ] **S1.4 接通受限手动控制与规则控制（对应 PUMP-02）**

  **目标：** 在已确认范围内控制目标泵，沿用已有规则所有权及统一安全入口。

  **交付物：** 手动／自动控制接线、控制模式与范围说明、正常渐变及热紧急回归测试。

  **验收：** 手动、自动及回退路径都经过运行时安全策略；低于保护下限的当前读数不会拖低输出，热紧急动作不会被正常渐变延迟；不新增绕过安全层的设备直写入口。实机操作范围须先由 S1.1 的证据确定。

- [ ] **S1.5 建立请求、回读与实际转速的确认链（对应 PUMP-02）**

  **目标：** 让用户分清请求发出、请求被接受、设定值得到回读、实际 RPM 有响应四种证据。

  **交付物：** 写入状态、回读结果、时间戳、实际 RPM 和审计记录的关联展示，以及拒写、回读不符和回读缺失测试。

  **验收：** 在指定实机的已确认范围内进行受限设定变化，分别记录请求、结果、回读和 RPM；回读缺失保持“未确认”，不能显示虚假的成功；实际响应不符合预期时明确报告，不仅凭 HTTP 返回成功判定完成。

- [ ] **S1.6 补齐泵故障与恢复的软件场景（对应 PUMP-03）**

  **目标：** 可解释地处理停转读数、RPM 丢失、拒写、未确认、回读失败及断连。

  **交付物：** 模拟器或测试夹具、故障用例表、状态变化和保护动作报告。

  **验收：** 故障先在软件环境注入；覆盖超时、重试边界、持续故障和恢复后的状态；不存在无限沿用旧低速输出、无限无反馈重试或把未知状态当正常值的路径。测试结果清楚注明模拟性质，不宣称真实泵或热模型已经验证。

- [ ] **S1.7 验证规则停止、退出和重连后的控制归属（对应 PUMP-03）**

  **目标：** 应用结束控制时能够明确交还控制，恢复失败时保留可见状态与诊断证据。

  **交付物：** 停止／禁用／删除规则、正常退出、重新连接的验证记录，明确恢复到哪种原控制方式及其结果。

  **验收：** 软件用例覆盖安全交接和恢复失败；指定实机按事先确认的步骤验证正常停止与退出后的实际控制状态。重连后重新确认设备身份和状态，不回放对旧设备的历史请求；未确认恢复不能显示“已恢复”。

- [ ] **S1.8 完成指定设备验收与 v0.2.0 发布记录（对应 PUMP-04）**

  **目标：** 把一台真实水泵的完整验收结果变成用户可查的支持声明和可安装版本。

  **交付物：** 支持表、环境记录、原始证据索引、Windows 构建／CLI 安装／界面与回归结果，以及发布后的 tag、源码 SHA 和发行链接。

  **验收：** S1.1–S1.7 的证据能够串联到同一型号和接法；支持表明确读取、控制、恢复分别是否通过及限制。只有真实发布完成且来源核对一致，才将 v0.2.0 在版本树中改成“已发布”；CI 或模拟通过不能代替实机结论。

### 完成定义

本阶段完成必须同时满足：一台明确型号和接法的泵完成身份确认、RPM 读取、受限控制和正常退出恢复；歧义映射、读数缺失、未确认及故障状态都有可复查处理；所有控制路径遵守保护范围；Windows 安装和回归通过；公开支持表写明实测对象、环境、限制和证据。

验收记录沿用 [Windows 验收入口](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/windows-validation/ACCEPTANCE-ENTRY.md)，版本状态通过 [版本管理说明](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/version-management.md) 维护。没有实机证据的条目继续保留“待验证”。

### 排除范围

- 不把本阶段扩展为 USB 一体水冷厂商协议开发、真实 ODP 控制器或固件烧写。
- 不包含流量计、漏液／压力传感器、灯效、小屏、多泵协同或整机散热策略重构。
- 不承诺通用的最低占空比、所有泵可控或所有主板接口行为一致。
- 不另建复杂规则仲裁系统；复用已有单目标所有权、安全交接、故障处理和审计机制。
- 不通过停掉真实冷却设备来替代软件故障注入；未经验证的恢复路径不能被写成已通过。

实现参考：[LHM 映射](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/libre-hardware-monitor/src/mapping.rs)、[LHM 读写与回读](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/libre-hardware-monitor/src/lib.rs)、[运行时安全策略](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-runtime/src/safety.rs)、[自动化引擎](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-automation/src/engine.rs)。

---

# English

**Status: Planned. Target version: v0.2.0.** This is the next step in the hardware support roadmap. The version number identifies this stage's scope; it does not mean that validation on physical hardware is complete. No pump model is currently recorded as verified.

Parent issue: https://github.com/SvenKunkka/OpenHardwareOS/issues/1  
Following stage: https://github.com/SvenKunkka/OpenHardwareOS/issues/3 (Complete whole-system cooling; not scheduled)

### What users will get

On Windows, users will be able to accurately identify a real pump of a specified model and connection configuration, view its RPM, understand whether control is permitted, adjust its duty cycle or use rules within a confirmed range, and clearly see whether a write has been confirmed. When a rule stops, the application exits, or an abnormal condition occurs, users will be able to tell who controls the pump, what protective action was taken, and whether restoration succeeded.

This stage aims to complete a reviewable end-to-end workflow on one device. Every support statement must specify the model, motherboard or controller, connection configuration, control mode, and software environment. Results from this one device must not be extrapolated into a claim that all pumps are supported.

### Existing foundations and gaps

- The `Pump` device type, `pump.rpm`, `pump.speed_percent`, rule engine, minimum duty-cycle protection, fault states, and recovery mechanisms already exist. Reuse them first and verify their actual behavior.
- The LHM adapter already provides HTTP reads and writes, plus setpoint readback. Current pump identification and RPM/Control pairing still depend on names, numbers, and similar information. Duplicate names, reordered enumeration, and ambiguous mappings must be addressed to prevent writes to the wrong channel.
- Simulated pumps, rejected writes, unconfirmed writes, and disconnection scenarios already exist. Stalled-pump, missing-RPM, and recovery scenarios still need to be completed. The simulated pump is not currently connected to a thermal model, so simulation results cannot demonstrate real cooling capability.
- The current software default pump protection value of 60% must not be treated as a hardware requirement for every model. The actual test range and restoration steps must be determined from information about the target device and validation evidence.

References: [Existing v0.2.0 pump plan](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/plans/pump-support.md), [Device capabilities and adapter boundaries](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/device-model.md), and [Automation mechanisms](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/automation.md).

### Hardware scope

The first acceptance target is one pump that exposes explicit RPM and Control channels through the existing LHM path. Record its motherboard or controller interface and its actual operating mode, such as PWM or DC. Enable only channels whose identity, control range, and restoration method can be confirmed.

Present readings, requested duty cycle, setpoint readback, and actual RPM separately. An interface accepting a request or returning a matching setpoint does not mean the actual speed has reached the target. When the control mapping is ambiguous, retain read-only access and explain why.

### Entry dependencies

- Select the target pump, motherboard or controller, and connection configuration; record the Windows, LHM, and relevant driver versions.
- Obtain an initial read-only device inventory, including LHM device, RPM, and Control paths and whether each can be read, written, or read back.
- Before any control of physical hardware, confirm the device's permitted control mode, range, and restoration procedure. Continue with read-only work if these cannot be confirmed.
- Preserve the existing rules, safety policies, audit, and recovery mechanisms. New logic must pass through the same runtime entry point.
- Validate software fault scenarios in the simulator and sanitized samples first. Do not create faults by setting a real pump to zero or unplugging cooling equipment.

### Substage checklist

- **S1.1 Establish the target device and test environment record (maps to PUMP-01)**

  **Goal:** Fix the test target for this stage and identify what is known, unknown, or requires read-only confirmation.

  **Deliverables:** A device card containing the pump model, motherboard or controller model, connection configuration, control mode, software versions, LHM paths, candidate control range, and restoration documentation.

  **Acceptance:** Every identity field has a source; the relationship between the RPM and Control channels and the target device can be reviewed; unknowns are explicitly marked. The device card must not mark the device as controllable until its control range and restoration method are confirmed.

- **S1.2 Establish stable pump identity and RPM/Control channel mapping (maps to PUMP-01)**

  **Goal:** Eliminate the risk of controlling the wrong device caused by pairing based solely on names, display order, or loosely matched numbers.

  **Deliverables:** Mapping rules, sanitized LHM samples with traceable sources, an explanation of ambiguity handling, and automated tests.

  **Acceptance:** Samples cover a single pump, multiple channels, missing numbers, duplicate names, reordered enumeration, and reconnection after disconnection. Uniquely identifiable pumps retain a consistent identity; channels that cannot be uniquely identified remain unavailable for control. Reordering must not redirect historical requests to another channel.

- **S1.3 Complete read-only pump monitoring in the CLI and UI (maps to PUMP-01)**

  **Goal:** Let users identify the target pump and distinguish valid RPM, zero RPM, stale data, and missing data.

  **Deliverables:** CLI and UI displays for the pump name, RPM, units, update time, read/write capabilities, and reasons for unavailability, with associated UI and data-conversion tests.

  **Acceptance:** The specified physical device's display matches the S1.1 device card, and RPM can be compared with LHM readings from the same period. Missing readings are not displayed as zero; a duty-cycle setpoint is not presented as actual RPM; simulated devices are always clearly marked.

- **S1.4 Connect bounded manual and rule-based control (maps to PUMP-02)**

  **Goal:** Control the target pump within the confirmed range, reusing existing rule ownership and the shared safety entry point.

  **Deliverables:** Integration of manual and automatic control, documentation of control modes and ranges, and regression tests for normal ramping and thermal emergencies.

  **Acceptance:** Manual, automatic, and fallback paths all pass through runtime safety policies. A current reading below the protective minimum must not pull the output below that minimum, and thermal emergency actions must not be delayed by normal ramping. No direct device-write entry point bypassing the safety layer is added. The operating range for physical hardware must first be established by the evidence in S1.1.

- **S1.5 Establish the confirmation chain from request to readback to actual speed (maps to PUMP-02)**

  **Goal:** Help users distinguish four types of evidence: a request was sent, the request was accepted, the setpoint was read back, and actual RPM responded.

  **Deliverables:** A linked presentation of write status, readback results, timestamps, actual RPM, and audit records, plus tests for rejected writes, mismatched readback, and missing readback.

  **Acceptance:** Make bounded setpoint changes within the confirmed range on the specified physical device, separately recording each request, result, readback, and RPM. Missing readback remains “unconfirmed” and must not produce a false success display. Unexpected physical responses are explicitly reported; a successful HTTP response alone is insufficient to declare completion.

- **S1.6 Complete software scenarios for pump faults and recovery (maps to PUMP-03)**

  **Goal:** Handle stalled-pump readings, missing RPM, rejected writes, unconfirmed writes, readback failures, and disconnections in an explainable way.

  **Deliverables:** A simulator or test fixtures, a fault-case table, and a report of state transitions and protective actions.

  **Acceptance:** Faults are first injected in software. Coverage includes timeouts, retry limits, persistent faults, and states after recovery. No path indefinitely retains an old low-speed output, retries indefinitely without feedback, or treats an unknown state as a normal value. Results clearly identify their simulated nature and do not claim that a real pump or thermal model has been validated.

- **S1.7 Verify control ownership after rule stops, application exit, and reconnection (maps to PUMP-03)**

  **Goal:** Explicitly hand control back when the application ends control, while retaining visible status and diagnostic evidence if restoration fails.

  **Deliverables:** Validation records for stopping, disabling, and deleting rules, normal application exit, and reconnection, specifying which original control mode is restored and the outcome.

  **Acceptance:** Software cases cover safe handover and failed restoration. On the specified physical device, verify the actual control state after a normal stop and exit using steps confirmed in advance. Reconfirm device identity and state after reconnection, without replaying historical requests for the old device. Unconfirmed restoration must not be displayed as “restored.”

- **S1.8 Complete acceptance for the specified device and the v0.2.0 release record (maps to PUMP-04)**

  **Goal:** Turn complete acceptance results for one real pump into a support statement users can inspect and a version they can install.

  **Deliverables:** A support table, environment record, raw-evidence index, Windows build, CLI installation, UI, and regression results, plus the tag, source SHA, and release link after publication.

  **Acceptance:** Evidence from S1.1–S1.7 can be linked to the same model and connection configuration. The support table explicitly states whether reading, control, and restoration each passed, together with their limitations. Mark v0.2.0 as “released” in the version tree only after the release is actually published and its provenance has been checked for consistency. Passing CI or simulation cannot replace conclusions from physical hardware.

### Definition of done

Completing this stage requires all of the following: one pump with an explicit model and connection configuration has completed identity confirmation, RPM reading, bounded control, and restoration on normal exit; ambiguous mappings, missing readings, unconfirmed writes, and fault states all have reviewable handling; every control path respects the protective range; Windows installation and regression checks pass; and the public support table records the tested device, environment, limitations, and evidence.

Use the existing [Windows acceptance entry point](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/windows-validation/ACCEPTANCE-ENTRY.md) for acceptance records and the [Version management guide](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/version-management.md) to maintain version status. Entries without physical-hardware evidence remain “pending validation.”

### Out of scope

- Do not expand this stage into vendor-specific USB all-in-one liquid-cooler protocol development, real ODP controllers, or firmware flashing.
- Flow meters, leak or pressure sensors, lighting effects, small displays, coordinated multi-pump control, and redesign of whole-system cooling strategies are excluded.
- Do not promise a universal minimum duty cycle, controllability of every pump, or identical behavior across all motherboard headers.
- Do not build a separate complex rule arbitration system. Reuse existing single-target ownership, safe handover, fault handling, and audit mechanisms.
- Do not substitute stopping real cooling equipment for software fault injection. Unverified restoration paths must not be recorded as passed.

Implementation references: [LHM mapping](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/libre-hardware-monitor/src/mapping.rs), [LHM reads, writes, and readback](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/adapters/libre-hardware-monitor/src/lib.rs), [Runtime safety policies](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-runtime/src/safety.rs), and [Automation engine](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-automation/src/engine.rs).


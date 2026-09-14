> 快照：GitHub Issue #5 的正文镜像（逐字节复制），抓取于 2026-09-14 18:45 CST / 2026-09-14T10:45:43Z。**实时进度以 Issue 里的复选框为准**，本文件不会自动同步。
>
> Snapshot: a byte-for-byte mirror of the body of GitHub Issue #5, captured 2026-09-14T10:45:43Z. The issue's checkboxes are the live progress record; this file does not auto-sync.

<!--
source:      https://github.com/SvenKunkka/OpenHardwareOS/issues/5
title:       [Stage 4 / 阶段四] Lighting & Displays / 灯光与屏幕
captured_at: 2026-09-14T10:45:43Z (2026-09-14 18:45 CST)
repo_commit: 2aea128b1bea6170b801a1a1af4724d0efa0b500
authority:   the GitHub issue is authoritative for progress; this file is the versioned snapshot.
-->

**语言 / Language:** [中文](#中文) · [English](#english)

两种语言使用相同的小阶段编号；中文区的复选框是共用进度记录。
Both versions use the same substage IDs. The checkboxes in the Chinese section are the shared progress record.

# 中文

## 第四阶段：灯光与屏幕

关联：[六阶段总任务](https://github.com/SvenKunkka/OpenHardwareOS/issues/1) · [第三阶段](https://github.com/SvenKunkka/OpenHardwareOS/issues/4) · [第五阶段](https://github.com/SvenKunkka/OpenHardwareOS/issues/6)

状态：规划草稿，尚未排期；本阶段不绑定发布版本。以下小阶段在本 Issue 内跟踪，不自动创建独立 Issue。

### 用户结果

用户能在明确兼容的设备上统一设置灯光，并把真实硬件状态显示到机箱屏、冷头屏或桌面小屏。用户能辨认控制的灯区和屏幕，预览内容、调整亮度并恢复默认；界面清楚区分已经发送、设备已确认和实际显示已验证。

### 当前证据与缺口

- 当前 [设备模型](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/device.rs)有 `Rgb`、`Display` 类型，[能力模型](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/capability.rs)具备读写、单位和范围的通用表达，但没有对应的实际灯光或屏幕适配器。
- [已注册适配器](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/adapters/src/lib.rs)目前是 System、LHM、NVIDIA、ODP 和 Mock；ODP 仍连接模拟 OpenFan。桌面应用自己的窗口、HTML 或截图不构成外接硬件屏幕实现证据。
- 灯区、灯珠拓扑、颜色和效果能力，屏幕分辨率、像素格式、帧传输、布局与设备状态确认均需建立。现有热控能力不能直接替代灯光或屏幕协议。
- `OpenRGB` 仅为接入候选，本项目尚未集成；任何上游型号名单均需在本项目逐型号核对接口、许可和实际效果，不能直接视为本项目支持范围。

### 范围

覆盖已确认接口和电气规格的 RGB/ARGB 控制器，以及明确型号的机箱屏、冷头屏、桌面小屏。包括设备识别、灯区与屏幕能力、用户内容、传输与控制权、异常恢复及兼容说明。第三方设备接入和自研控制器扩展可并行进行，不能以一个产品类别的完成覆盖其余类别。

### 依赖

- 复用 [第三阶段](https://github.com/SvenKunkka/OpenHardwareOS/issues/4)的真实 USB 连接、稳定设备身份、板型校验与恢复边界；对不走该通道的候选接口，先验证连接和权限再进入写入。
- 散热控制与安全策略必须独立于灯效和屏幕工作负载，用户原有硬件规则不因内容播放或传输故障而失效。
- 每个候选设备须有准确型号/修订版/固件、接口与供电信息、可用协议和许可依据。需要设备固件变更时，沿用第三阶段的独立授权与恢复要求。

### 小阶段

- [ ] **4.1 核对候选设备、接口与电气边界。** 目标：选出可实际验证的灯光控制器和机箱/冷头/桌面屏，避免按连接器外形或品牌推断兼容。交付物：逐型号调查表，RGB/ARGB 电压、引脚、灯珠/通道与负载限制，屏幕接口、尺寸/分辨率及许可记录；对 OpenRGB 等候选写明接入方式及限制。验收：每个拟实现对象具备明确通信依据和接线信息；不清楚电压、协议或板型时不执行试写，不混接 RGB/ARGB。
- [ ] **4.2 建立灯区与屏幕能力约定。** 目标：用通用模型准确表达不同设备能做什么。交付物：稳定灯区/灯珠 ID、颜色格式、亮度、效果及更新限制；屏幕 ID、分辨率、方向、像素格式、刷新/传输预算与确认状态；支持缺失字段的显式表示。验收：描述符和软件示例能区分静态灯光、设备原生效果、逐灯珠控制，以及整帧/局部更新；不支持的能力不可被配置或写入，模拟标记始终保留。
- [ ] **4.3 接通首批真实灯光控制器。** 目标：让用户对已确认灯区完成基本亮度、静态颜色及关闭/恢复操作。交付物：逐型号适配器、请求日志、可控灯区映射，以及经核对的上游依赖固定版本和许可说明。验收：在明确授权的目标设备上逐区观察并记录真实灯光结果，验证颜色顺序、亮度范围、关闭与恢复默认；只有设备确实提供的效果和拓扑才被公开，失败或无确认不得显示为已应用。
- [ ] **4.4 完成灯光场景、控制权与异常恢复。** 目标：用户能保存场景、同步兼容灯区，并理解与厂商软件的控制冲突。交付物：静态/动态场景配置、支持能力降级规则、更新节流、控制权状态、停止/退出/断连处理。验收：场景重新打开和设备重连行为与说明一致；厂商程序争用、设备拒绝与传输故障可复现且有提示；灯光更新不会阻塞散热轮询，重连不重放未确认的历史写入。
- [ ] **4.5 接通硬件屏幕的静态显示路径。** 目标：在真实机箱屏、冷头屏或桌面屏上正确显示测试图与静态页面。交付物：逐型号屏幕发现、像素转换、方向/尺寸处理、传输和错误恢复，以及基础布局预览。验收：在每个拟宣称支持的屏幕类别及具体型号上，实拍核对方向、颜色、裁切、文字与实际内容；应用内预览或传输成功日志不能代替设备显示验证，未测类别保持待验证。
- [ ] **4.6 形成硬件状态仪表盘与用户内容流程。** 目标：用户能把可信传感器和自选内容放到外部屏幕。交付物：字段绑定、单位、可配置布局、亮度/方向、内容尺寸与格式校验；缺失/过期数据和设备断连占位状态；动态内容仅在设备能力允许时提供。验收：屏幕数值与同一时刻的数据源及时间状态可核对，数据过期不会继续显示为实时正常；不支持内容被明确拒绝，演示或估算数据有标记，导入内容不会执行脚本或触发硬件控制。
- [ ] **4.7 验证灯光、屏幕与散热并行运行。** 目标：多类设备同时工作时保持散热优先及界面可响应。交付物：轮询/渲染/传输调度与限流、独立错误隔离、休眠唤醒和热插拔恢复策略，以及目标设备的资源测量记录。验收：在已声明设备组合上测试动态灯效与屏幕更新、断连、厂商程序争用和应用退出；散热安全动作仍能执行，屏幕或灯光故障不会导致冷却控制丢失。资源与帧率结论仅对应实测组合。
- [ ] **4.8 完成用户安装、兼容矩阵与发布验收。** 目标：用户能按明确边界完成安装、配置和排障。交付物：按灯光/机箱屏/冷头屏/桌面屏分别列明的型号、修订版、固件、平台、支持能力与限制；用户说明、恢复步骤、素材许可说明和发布候选。验收：每项公开功能都有对应实机证据；首次安装、选择设备、实际显示/发光、保存配置、停止控制与恢复路径按说明复测；未实现或仅模拟的功能不得标为已支持。

### 阶段完成条件

- 首轮至少完成一款灯光控制器和一款真实屏幕的完整路径。机箱屏、冷头屏、桌面小屏分别登记覆盖情况；其余类别继续保留扩展目标，未测类别不因另一类屏幕通过而标为支持。后续新增型号按同一验收标准推进。
- 对正式声明支持的每个型号，基础能力、控制权、异常恢复与散热并行运行均达到上述验收条件，且文档与实际功能一致。
- 用户内容和依赖许可可追溯，设备适配范围可查询；软件模拟、传输确认和真实物理显示/发光分别记录，不互相替代。

### 边界

不承诺任何品牌全系支持，不将 RGB/ARGB 接头视为可任意互换；不默认采用闭源 SDK、不自动替换驱动或刷写第三方固件。大屏显示器 DDC/CI、显示器支架机械运动等功能若另列在后续阶段，不由本阶段的 `Display` 类型自动涵盖。涉及自研屏幕固件时须依赖第三阶段经过验证的板型和恢复流程，并另获具体操作授权。本阶段也不以高频灯效或动态图像为默认负载，不影响既有冷却保护。

---

# English

## Stage 4: Lighting and Displays

Related: [Six-stage parent issue](https://github.com/SvenKunkka/OpenHardwareOS/issues/1) · [Stage 3](https://github.com/SvenKunkka/OpenHardwareOS/issues/4) · [Stage 5](https://github.com/SvenKunkka/OpenHardwareOS/issues/6)

Status: planning draft, not yet scheduled. This stage is not tied to a release version. The milestones below are tracked within this issue; separate issues are not created automatically.

### User outcome

Users can configure lighting through a unified interface on explicitly compatible devices and display actual hardware state on case displays, cooler-block displays, or compact desktop displays. Users can identify the lighting zones and screens they control, preview content, adjust brightness, and restore defaults. The UI clearly distinguishes content that has been sent, acknowledged by the device, and verified on the physical display.

### Current evidence and gaps

- The current [device model](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/device.rs) includes `Rgb` and `Display` types. The [capability model](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-device-model/src/capability.rs) provides generic representations of read/write access, units, and ranges, but there are no corresponding real lighting or display adapters.
- The [registered adapters](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/adapters/src/lib.rs) are currently System, LHM, NVIDIA, ODP, and Mock. ODP still connects to a simulated OpenFan. The desktop application's own windows, HTML, or screenshots are not evidence of an implementation for an external hardware display.
- Lighting zones, LED topology, color and effect capabilities, display resolution, pixel formats, frame transmission, layouts, and device-state confirmation still need to be established. Existing thermal-control capabilities cannot directly substitute for lighting or display protocols.
- `OpenRGB` is only an integration candidate and has not been integrated into this project. Every model in an upstream list requires model-specific verification of its interface, licensing, and actual output within this project; the list cannot be treated directly as this project's support scope.

### Scope

This stage covers RGB/ARGB controllers with confirmed interfaces and electrical specifications, and specific models of case displays, cooler-block displays, and compact desktop displays. It includes device identification, lighting-zone and display capabilities, user content, transmission and control ownership, fault recovery, and compatibility documentation. Third-party device integration and extensions to in-house controllers may proceed in parallel, but completing one product category does not establish completion of the others.

### Dependencies

- Reuse the real USB connections, stable device identities, board-type validation, and recovery boundaries from [Stage 3](https://github.com/SvenKunkka/OpenHardwareOS/issues/4). For candidate interfaces that use a different path, verify connections and permissions before attempting writes.
- Cooling control and safety policies must remain independent of lighting-effect and display workloads. Content playback or transmission failures must not invalidate the user's existing hardware rules.
- Each candidate device requires an exact model/revision/firmware, interface and power information, a usable protocol, and licensing evidence. Device firmware changes must follow Stage 3's separate authorization and recovery requirements.

### Milestones

- **4.1 Check candidate devices, interfaces, and electrical boundaries.** Goal: select lighting controllers and case/cooler-block/desktop displays that can actually be validated, without inferring compatibility from connector shape or brand. Deliverables: model-specific investigation records; RGB/ARGB voltages, pin assignments, LED/channel and load limits; display interfaces, physical dimensions/resolution, and licensing records; and the integration method and limitations for candidates such as OpenRGB. Acceptance: every target proceeding to implementation has a documented communication basis and wiring information. Do not attempt writes when voltage, protocol, or board type is unknown, and do not cross-connect RGB/ARGB interfaces.
- **4.2 Establish lighting-zone and display capability agreements.** Goal: use the common model to describe precisely what each device can do. Deliverables: stable lighting-zone/LED IDs, color formats, brightness, effects, and update limits; display IDs, resolution, orientation, pixel formats, refresh/transmission budgets, and acknowledgment states; and explicit representation of missing fields. Acceptance: descriptors and software examples distinguish static lighting, device-native effects, per-LED control, and full-frame/partial updates. Unsupported capabilities cannot be configured or written, and simulation labels are always retained.
- **4.3 Connect the first real lighting controllers.** Goal: let users perform basic brightness, static-color, off, and restore operations on confirmed lighting zones. Deliverables: model-specific adapters, request logs, controllable-zone mappings, and checked, pinned upstream dependency versions and licensing notes. Acceptance: on explicitly authorized target devices, observe and record actual lighting results zone by zone, verifying color order, brightness range, off behavior, and restoration of defaults. Expose only effects and topology actually provided by the device. Failed or unconfirmed operations must not be shown as applied.
- **4.4 Complete lighting scenes, control ownership, and fault recovery.** Goal: let users save scenes, synchronize compatible zones, and understand control conflicts with vendor software. Deliverables: static/dynamic scene configurations, capability fallback rules, update throttling, control-ownership states, and handling for stop, exit, and disconnection. Acceptance: reopening scenes and reconnecting devices behave as documented. Contention with vendor applications, device rejection, and transmission failures are reproducible and surfaced to the user. Lighting updates do not block cooling polls, and reconnection does not replay unconfirmed past writes.
- **4.5 Connect the static-content path for hardware displays.** Goal: correctly show test images and static pages on real case, cooler-block, or desktop displays. Deliverables: model-specific display discovery, pixel conversion, orientation/size handling, transmission and error recovery, and basic layout previews. Acceptance: for every display category and specific model to be advertised as supported, use photographs of the physical display to check orientation, colors, cropping, text, and actual content. In-app previews or successful transmission logs cannot replace physical-display verification; untested categories remain pending validation.
- **4.6 Provide hardware-state dashboards and a user-content workflow.** Goal: let users place trustworthy sensor readings and content of their choice on external displays. Deliverables: field bindings, units, configurable layouts, brightness/orientation controls, content dimension and format validation, and placeholder states for missing/stale data and device disconnection. Dynamic content is offered only when device capabilities allow it. Acceptance: display values can be checked against the data source and its timestamp/status at the same instant. Stale data is not shown as current and normal. Unsupported content is explicitly rejected, demo or estimated data is labeled, and imported content neither executes scripts nor triggers hardware control.
- **4.7 Verify concurrent operation of lighting, displays, and cooling.** Goal: keep cooling prioritized and the UI responsive while multiple device classes operate together. Deliverables: polling/rendering/transmission scheduling and rate limits, independent fault isolation, sleep/resume and hotplug recovery policies, and resource-usage measurements for the target devices. Acceptance: test dynamic lighting and display updates, disconnection, contention with vendor applications, and application exit on the declared device combinations. Cooling safety actions remain executable, and lighting or display failures do not cause loss of cooling control. Resource-usage and frame-rate conclusions apply only to the tested combinations.
- **4.8 Complete user installation, the compatibility matrix, and release acceptance.** Goal: let users install, configure, and troubleshoot within clear boundaries. Deliverables: models, revisions, firmware, platforms, supported capabilities, and limitations listed separately for lighting/case displays/cooler-block displays/desktop displays; user instructions; recovery steps; content licensing notes; and a release candidate. Acceptance: every advertised feature has corresponding real-hardware evidence. Retest first installation, device selection, physical display/lighting output, configuration saving, stopping control, and recovery according to the instructions. Unimplemented or simulated-only features must not be marked as supported.

### Stage completion criteria

- The first round completes a full path for at least one lighting controller and one real display. Coverage is recorded separately for case displays, cooler-block displays, and compact desktop displays. The remaining categories stay as extension targets; an untested category is not marked supported because another display category passed. Additional models follow the same acceptance standard.
- Every model officially declared supported meets the above acceptance criteria for basic capabilities, control ownership, fault recovery, and concurrent cooling operation, with documentation matching actual behavior.
- User-content and dependency licenses are traceable, and device support scope is discoverable. Software simulation, transmission acknowledgment, and actual physical display/lighting output are recorded separately and do not substitute for one another.

### Boundaries

This stage does not promise support for an entire brand or treat RGB/ARGB connectors as freely interchangeable. It does not default to closed-source SDKs, automatically replace drivers, or flash third-party firmware. Features such as DDC/CI for full-size monitors or mechanical movement of monitor arms, if assigned to later stages, are not automatically included by this stage's `Display` type. In-house display firmware work depends on the board types and recovery processes verified in Stage 3 and requires separate authorization for the specific operation. High-frequency lighting effects or animated images are not the default workload, and existing cooling protection must remain effective.


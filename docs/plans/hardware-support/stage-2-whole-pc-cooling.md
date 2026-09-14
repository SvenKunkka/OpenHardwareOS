> 快照：GitHub Issue #3 的正文镜像（逐字节复制），抓取于 2026-09-14 18:45 CST / 2026-09-14T10:45:43Z。**实时进度以 Issue 里的复选框为准**，本文件不会自动同步。
>
> Snapshot: a byte-for-byte mirror of the body of GitHub Issue #3, captured 2026-09-14T10:45:43Z. The issue's checkboxes are the live progress record; this file does not auto-sync.

<!--
source:      https://github.com/SvenKunkka/OpenHardwareOS/issues/3
title:       [Stage 2 / 阶段二] Whole-PC Cooling / 完善整机散热
captured_at: 2026-09-14T10:45:43Z (2026-09-14 18:45 CST)
repo_commit: 2aea128b1bea6170b801a1a1af4724d0efa0b500
authority:   the GitHub issue is authoritative for progress; this file is the versioned snapshot.
-->

**语言 / Language:** [中文](#中文) · [English](#english)

两种语言使用相同的小阶段编号；中文区的复选框是共用进度记录。
Both versions use the same substage IDs. The checkboxes in the Chinese section are the shared progress record.

# 中文

## 阶段 2：完善整机散热

状态：未排期。本 Issue 定义待完成的范围与验收条件，尚未指定发布版本、负责人、日期或预算，也不表示以下工作已经开工。

- 总规划：https://github.com/SvenKunkka/OpenHardwareOS/issues/1
- 入口依赖：https://github.com/SvenKunkka/OpenHardwareOS/issues/2
- 后续衔接：https://github.com/SvenKunkka/OpenHardwareOS/issues/4

### 用户最终能得到什么

用户能够在同一界面查看 CPU、GPU、主板、内存和存储的可用监测数据，知道每项读数来自哪里、是否新鲜、哪些能力受硬件或驱动限制；能够辨认并控制已验证支持的主板风扇接口与 GPU 风扇通道，而不会把同一物理通道误认为两个可同时控制的设备。

已经配置的规则应在真实整机场景下保持可解释的行为：谁拥有当前通道、写入是否得到确认、停止控制后交给谁、权限不足或原厂软件接管时发生了什么，均能被看见和复查。升级、重启、休眠、应用退出和依赖服务异常后，设备不会仅凭旧记录被当成“已经恢复控制”。

“完善整机散热”指完成约定硬件矩阵的覆盖和证据闭合，不等于保证所有 PC、所有传感器和所有风扇都能读写。未支持、未验证、硬件不提供和权限不足必须分别记录，不能用零值、重复设备或成功提示填补缺口。

### 已有基础与当前缺口

已有基础可直接复用，阶段 2 不把以下能力写成从零实现：

- 已有设备／能力模型、runtime 写入检查、安全策略、审计与模拟设备。
- 已有温度曲线、数值条件、条件不满足时的行为、传感器失联处理和回退机制。
- 已有以 `(device, capability)` 为目标的单一启用规则所有权检查；保存、启用和加载冲突规则已有拒绝或停用路径。
- 已有禁用、删除、改目标等退出控制场景的交接记录，以及未完成交接、未确认写入的持久化和重启后重新验证。规则声明拥有目标不等于设备已经确认接管。
- 已有 system、LHM、NVIDIA 等适配入口；LHM HTTP 假服务器和协议模拟测试证明部分软件链路，不证明某块主板或某张显卡已经通过真实读写验收。

本阶段要补齐的是：整机传感器覆盖的可核对清单、跨适配器的持久物理身份、更多主板接口和多风扇 GPU 的真实能力映射、后台权限机制，以及原厂软件共存和恢复的实机证据。当前按是否启用 LHM 来选择 NVIDIA provider 的临时规则，不能替代物理身份和逐能力来源选择。

基础参考：

- [需求状态及已有规则、回退和交接行为](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/requirements.md)
- [自动化说明](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/automation.md)
- [现有所有权与交接实现](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-automation/src/engine.rs)
- [现有安全策略](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-runtime/src/safety.rs)

### 硬件范围与边界

本阶段以 Windows 整机散热链路为主，保留共用模型和已有其他平台能力；不据此承诺其他平台具有同等监测或控制覆盖。

| 硬件类别 | 本阶段需要明确的范围 | 不得默认存在的能力 |
| --- | --- | --- |
| CPU | 型号、负载，以及硬件／provider 实际提供的温度、功耗、频率等；明确 package、core 等读数含义 | 所有 CPU 都有全部温度／功耗项目，或各 provider 的同名读数完全等价 |
| GPU | 每张卡的持久身份、可用温度／负载／功耗、各风扇或风扇组的 RPM 与控制范围 | 所有显卡都能读热点、逐风扇独立调速或读取硬件曲线 |
| 主板与风扇接口 | 对已纳入矩阵的板型记录接口标识、RPM、可写能力、控制模式与恢复行为；覆盖实际存在的多个接口 | 所有标为风扇的读数都有对应可写通道，或所有接口都支持相同的 PWM／DC 行为 |
| 内存 | 容量／使用率；对确实提供模块温度等额外数据的硬件记录来源和支持状态 | 普通内存均可读取 DIMM 温度、SPD 或提供主动散热控制 |
| 存储 | 设备身份、容量／使用情况，以及可访问的温度、健康等项目；明确系统卷与物理盘的关系 | 所有 SATA／NVMe／外置盒都能透传同样的数据，或存储读数可以作为直接控制接口 |
| 散热通道 | 已验证可控的 CPU 散热风扇、机箱风扇、主板其他风扇接口及 GPU 风扇；泵接口应识别类型并保留已有安全边界 | 将泵当普通风扇做停转实验，或把本阶段扩大为完整水冷管理 |

具体主板、显卡、CPU、内存和存储样本在小阶段 2.1 中确定。本 Issue 不预先宣布任何型号已受支持，也不设置未经确认的样本数量、占空比、温度阈值或长测时长。

### 进入本阶段的依赖

1. 阶段 1（https://github.com/SvenKunkka/OpenHardwareOS/issues/2）的完成定义、未闭合项和已有证据可被引用；依赖未完成时保持本阶段对应工作项未完成，不能默认上阶段已交付。
2. 真实测试前，应具备可识别的测试整机、板卡和接口清单，以及当前固件、驱动、provider 与应用构建标识；控制上限、最低安全输出和停止条件应针对样本事先记录。
3. 对依赖 LHM 的路径，应明确用户安装／运行 LHM、启用数据接口和所需驱动／权限的条件。软件能访问 HTTP 不等于目标主板具有可写 `Control` 通道。
4. 如需新增后台 helper，应先记录所选进程边界、安装与卸载路径、权限和通信约束。相关 ADR 中的 Task Scheduler、Windows service 或 sidecar 仍是方案输入，不能写成已安装、已验收或已承诺在某版本发布。
5. 每项真实写入都需要有可验证的释放／恢复路径。不能可靠恢复的样本可保持监测状态并记录限制，不得通过绕过固件保护或自动结束原厂进程来取得控制。

相关设计依据：[LHM 集成](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0003-libre-hardware-monitor-integration.md)、[厂商 SDK 与 AMD 的待批准提案](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0004-vendor-sdks-and-amd.md)、[Windows 权限与自启动](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0006-windows-privileges-and-autostart.md)。其中仍为 Proposed 或 Planned 的内容需要明确决策，不能视为既定交付。

### 8 个小阶段

#### 2.1 确定整机兼容矩阵与验证入口

- [ ] 完成小阶段 2.1：形成可追踪的硬件与能力清单。

**目标：** 把“支持这台电脑”拆成可验证的设备、传感器和控制通道，明确准备覆盖什么、依赖什么、什么尚未有证据。

**交付物：** 一份纳入仓库的兼容与验证矩阵，按整机／板卡／接口记录型号、硬件修订、BIOS／固件、驱动、provider、应用构建、监测项目、写入项目、权限要求和恢复方式；另列明确不纳入本阶段的样本与原因。

**可复查验收条件：**

- CPU、GPU、主板、内存、存储五类都有条目和支持状态，不因“没有读数”而从表中消失。
- 已支持、部分支持、未支持、待实机验证四类结论有统一定义；有实机结论的条目能追溯到样本与证据，模拟结果单独标明。
- 所有准备进行真实控制的通道都有编号、关联风扇／风扇组、预先确认的安全限制和退出恢复方法；缺少这些条件的通道不进入写入验收。
- 任何代表性、样本数量和持续运行时长要求先在矩阵中说明理由；本 Issue 不替代尚未完成的样本选择。

#### 2.2 建立持久物理身份与跨适配器去重

- [ ] 完成小阶段 2.2：让同一物理设备及控制通道拥有一致、可追溯的身份。

**目标：** 同一 GPU 或主板通道被多个 adapter 发现时，界面和规则能识别它们的关系；重启、枚举顺序变化或 provider 切换不把既有规则绑定到别的硬件。

**交付物：** 物理设备／物理控制通道的身份约定、provider 来源字段、确定性的读数来源和写入路径选择、去重实现、旧标识兼容或重新确认策略，以及对应迁移说明。

**可复查验收条件：**

- 同一显卡通过 LHM 与 NVML 出现时可归为同一物理设备，同时保留每项能力的实际来源；不同实体但型号相同的设备不能被合并。
- 重启、重新发现、设备顺序变化以及选定 provider 失效／恢复后，身份与规则目标保持正确；身份不足或发生歧义时明确要求重新确认，不能猜测映射后自动写入。
- 同一物理输出的不同 adapter 别名，进入既有 owner 检查前映射到同一个规范目标；它们不能成为两个绕过冲突检查的写入口。
- 去重前后可用传感器不会被无说明地丢弃；多个来源冲突时能查看选择依据。旧配置迁移不得将未确认的控制状态当成已接管。

参考：[当前 LHM／NVML 优先规则及未实现的物理身份方案](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0005-nvml-precedence-and-device-identity.md)。本小阶段不引入多规则竞争、动态竞价或通用复杂仲裁框架。

#### 2.3 补齐 CPU／GPU／主板／内存／存储的监测完整性

- [ ] 完成小阶段 2.3：完成矩阵内整机监测数据的覆盖、质量和可解释性验证。

**目标：** 让用户能辨认需要的温度和负载信号，并在缺项、过期或异常时知道原因，避免用看似完整但实际错误的数据驱动散热。

**交付物：** 五类设备的监测覆盖表、字段含义与单位、来源优先关系、采样与过期策略、无效值处理，以及 UI／CLI 的一致展示与诊断输出。

**可复查验收条件：**

- 对矩阵中约定的每个字段，记录可读取、硬件不提供、provider 不支持、权限不足或待验证的具体状态；不存在凭空补零、用其他温度冒充目标温度等行为。
- 温度、RPM、百分比、功耗等单位及传感器归属正确；同名读数的实际含义可区分。GPU 停转的真实零 RPM、断连和过期数据不会被混为一类。
- 按事先记录的来源与比较方法复查样本读数；偏差、刷新间隔和容许范围有依据，不将不同采样窗口的数据强行判为相同。
- 来源消失、恢复、无效数值和延迟更新时，沿用既有失联／回退策略，规则不能把过期或无效值当作新证据。对新增来源补齐回归和实机记录。

#### 2.4 扩展并验证多主板、多风扇接口控制

- [ ] 完成小阶段 2.4：在矩阵内主板上验证多个物理接口的识别、读写与恢复。

**目标：** 用户可以区分并操作确实可控的 CPU 风扇、机箱风扇等接口，避免错控相邻接口、误报支持或将泵接口当成普通风扇。

**交付物：** 按板型／固件记录的接口映射、adapter 能力补充、只读与可写限制、通道标识说明、测试方法及逐接口验收记录。

**可复查验收条件：**

- 覆盖矩阵中已约定的不同主板与接口布局；每个被标为可控的接口有实际对应关系、读数来源及写入证据，不能只凭 SuperIO 名称或通道序号推定。
- 对一个接口的操作不会改变未选定的独立接口；硬件实际联动或共享控制的通道按组表达，并明确组内关系。
- 写入范围、最低安全输出与 PWM／DC 等模式受能力约束；拒绝、权限不足和固件接管准确可见，不把“请求已发送”当成写入确认。
- 禁用规则、改目标、停止 provider、退出应用后，逐接口验证既有安全交接和适配器可提供的原厂恢复行为；缺少可控或可恢复能力的接口保留为限制项。

#### 2.5 完善多风扇 GPU 的真实通道模型

- [ ] 完成小阶段 2.5：验证多风扇显卡的监测、独立／分组控制与恢复边界。

**目标：** 避免把多风扇显卡简化成一个虚假的统一 RPM，也避免在只有分组控制能力的硬件上提供并不存在的逐风扇控制。

**交付物：** GPU 型号／驱动与 provider 组合矩阵、物理风扇与逻辑控制组的映射、逐项能力声明、NVIDIA／AMD 支持差异说明和失败诊断。

**可复查验收条件：**

- 对纳入矩阵的多风扇显卡，明确每项 RPM 对应哪个风扇或组；无法取得逐风扇读数时展示实际能力，不复制一个读数假装完整。
- 只有实测可独立控制的输出才显示独立调节；绑定／联动的风扇作为同一控制组，且沿用一个规范目标的 owner 约束。
- 记录静止风扇、零转速模式、最低可接受输出、驱动拒绝和原厂自动模式的行为；在已确认安全范围内验证控制与恢复，不通过停掉驱动保护扩大能力。
- provider 切换、LHM 与 NVML 同时可见、驱动或权限异常时，不产生双写、错卡或错风扇；未验证的 AMD／NVIDIA 型号继续明确标为未验证。

#### 2.6 把已有单目标所有权和安全交接验证到整机通道

- [ ] 完成小阶段 2.6：沿用现有机制，闭合多通道和跨 adapter 场景的冲突与交接证据。

**目标：** 整机规模扩大后，仍然保证每个规范输出最多只有一个启用规则 owner，停止控制时安全交接可见、有限重试、可恢复。

**交付物：** 规范目标与现有 owner 机制的连接、覆盖全部规则入口的回归用例、真实通道交接记录，以及必要的诊断补充；新增工作限于接入和发现的缺陷修复。

**可复查验收条件：**

- 保存、启用、导入／重载与跨 adapter 别名场景中，两个规则不能同时取得同一规范输出；不同物理通道可以独立拥有规则，不将整块主板误锁成单通道。
- 复用已有数值条件、曲线、条件关闭行为和失联回退，验证新增传感器与通道接入后的行为；不重新设计这些机制。
- 禁用、删除、改目标、来源消失、应用重启和未确认写入的恢复均有可追踪结果。声明 owner 不等于确认接管，旧交接不能覆盖当前已确认 owner。
- 交接失败仍可见、重试有界，重启后依据当前设备、能力与 owner 重新验证，不重放旧写入。回退占空比与“归还固件控制”必须作为不同结果记录。
- 不能可靠执行运行中 release 的 adapter 不得到虚假的 release 动作；已有拒绝／降级边界保留。真正的固件恢复只有在 adapter 支持且证据确认后才标记完成。

#### 2.7 明确后台执行、权限与安装生命周期

- [ ] 完成小阶段 2.7：交付并验证选定的最小后台权限方案及其生命周期。

**目标：** 普通权限界面能准确告知硬件访问条件；需要持续运行的散热任务有明确的后台主体、失效行为和恢复路径，而不是依赖“开机启动”开关隐式提权。

**交付物：** 一份决策记录，确定本阶段采用的 helper／sidecar／服务边界与用户行为；实现所选最小方案及必要安装、升级、卸载与通信控制；提供 LHM、驱动和权限缺失时的诊断。

**可复查验收条件：**

- 明确普通 UI、硬件访问进程和外部 provider 各自职责；需要权限的动作、授权发生点和失败提示可复查。HKCU 自启动不得被当作高权限后台服务。
- 若使用本机 IPC，调用者权限、消息校验与允许执行的硬件动作有明确边界；不提供任意命令执行或无限制的写入接口。
- 根据选定产品行为验证 UI 关闭、登录／注销、重启、休眠唤醒及依赖进程退出；后台继续运行或安全停止的结果与说明一致，不能只验证“进程还在”。
- 安装、升级和卸载不会留下失效路径、重复后台实例或无说明的控制占用；后台不可用时，前台诊断与已有失联／安全交接机制反映实际状态。
- 不把所有候选机制同时作为必须交付；所选方案及不覆盖的生命周期应在验收前明确，Windows 实测记录单独于模拟与静态检查保留。

#### 2.8 完成原厂软件共存、恢复及整机验收闭环

- [ ] 完成小阶段 2.8：完成矩阵内原厂软件共存、退出恢复和端到端整机证据闭合。

**目标：** 用户知道 OpenHardwareOS 与 BIOS／固件、LHM、显卡或主板原厂软件之间的控制关系；遇到竞争、退出或异常时能恢复到明确、经过验证的状态。

**交付物：** 按板型／显卡／软件组合记录的共存矩阵、冲突提示与处置说明、恢复操作说明、整机验收报告和未支持清单；向后续阶段提供可引用的规范身份、能力和控制生命周期接口。

**可复查验收条件：**

- 对矩阵中选定的原厂软件组合，记录可同时监测、可独占控制、存在写入竞争或无法确认等状态；不能可靠判断外部写入者时明确说明，不宣称已经自动检测全部冲突。
- 不自动结束原厂程序、不自动禁用系统／固件保护、不反复抢写争夺控制。需要用户选择控制主体时，明确指出受影响的具体物理通道。
- 对声明支持的每种控制路径，验证正常退出、禁用、依赖崩溃／不可用、重启和恢复后的实际输出或模式；接口返回成功但无足够设备证据时保持未确认。
- 恢复原厂自动模式、保持安全回退输出和需要人工恢复分别记录；不能证明原厂已重新接管的场景不得展示“已恢复原厂控制”。
- 用约定整机工作负载、采样方法、停止条件和时长完成端到端记录；每条支持结论能关联到样本、构建、配置、步骤、读数／日志和结果。公开材料移除个人信息及不应公开的凭据。
- 将可复用接口、限制和剩余问题交给后续阶段（https://github.com/SvenKunkka/OpenHardwareOS/issues/4）；后续阶段的链接不代表它已经开始、已确定版本或必须等待本阶段所有非阻塞增强项。

### 本阶段完成定义

以下条件全部成立后，才可关闭本阶段；仅有软件测试通过或界面演示不满足完成定义。

1. 上述 8 个小阶段均有可引用的交付物和验收结论，阶段 1 的入口依赖已按其完成定义确认。
2. 兼容矩阵中约定的五类监测对象、多主板接口和多风扇 GPU 范围逐项闭合；每项明确为已验证支持、有限支持或已确认不支持。未完成的实机验证不能通过改名为“支持”关闭。
3. 规范物理身份、单目标启用规则 owner、写入确认、安全交接和重启后的重新验证能够串成同一条证据链；没有通过新增另一套复杂仲裁绕开既有安全约束。
4. 后台权限及原厂软件共存／恢复达到所选方案的验收条件；未覆盖的行为在用户文档中可见。
5. 现有数值条件、回退和交接回归保持通过；模拟、真实应用链路与实机硬件结果分别标识，所有已知失败有处理结论。
6. 阶段完成、发布打包和真实硬件兼容是分别记录的结论；关闭本 Issue 不自动指定发布日期、创建发布、扩大支持范围或批准硬件量产。

### 排除范围

- 新建通用复杂仲裁系统、多规则优先级市场、自动抢占原厂控制权；本阶段沿用已有单 target 启用 owner 与安全交接。
- 将已有数值条件、温度曲线、失联回退、交接持久化重新列为“从零开发”。
- 对所有主板、所有显卡、所有内存／存储传感器做普遍支持保证，或将 LHM／驱动可见性当成实机写入证明。
- OpenHub／OpenFan 实体硬件设计、USB HID／CDC 新设备总线接入、自研设备固件升级，以及其他阶段的外设功能。
- 完整水冷／泵系统管理、未经单独定义的超频、电压调节、BIOS／EC 改写或关闭保护；识别泵接口并避免误控仍属于本阶段边界。
- 未经单独决策引入封闭厂商 SDK、重新分发驱动／供应商二进制，或把仍为 Proposed 的许可与厂商策略当成已批准。
- 本阶段未指定的其他操作系统全功能适配、安装发布执行、负责人分配、日期和预算承诺。

---

# English

## Stage 2: Complete Whole-System Cooling

Status: Unscheduled. This issue defines the scope and acceptance criteria for work that remains to be completed. No release version, owner, date, or budget has been assigned, and it does not mean that the work below has started.

- Overall plan: https://github.com/SvenKunkka/OpenHardwareOS/issues/1
- Entry dependency: https://github.com/SvenKunkka/OpenHardwareOS/issues/2
- Handoff to the next stage: https://github.com/SvenKunkka/OpenHardwareOS/issues/4

### What users should ultimately gain

Users can view available CPU, GPU, motherboard, memory, and storage monitoring data in one interface, understand where each reading comes from, whether it is fresh, and which capabilities are limited by the hardware or driver. They can identify and control motherboard fan headers and GPU fan channels whose support has been verified, without mistaking one physical channel for two devices that can be controlled simultaneously.

Configured rules should remain explainable on a real, complete system: who owns the current channel, whether a write has been confirmed, who receives control when it stops, and what happens when permissions are insufficient or manufacturer software takes over must all be visible and reviewable. After an upgrade, restart, sleep, application exit, or dependency service failure, control of a device must not be treated as "already restored" solely on the basis of old records.

"Complete whole-system cooling" means completing coverage and supporting evidence for the agreed hardware matrix. It does not guarantee that every PC, sensor, or fan can be read or written. Unsupported, unverified, not provided by the hardware, and insufficient permissions must be recorded separately. Gaps must not be filled with zero values, duplicate devices, or success messages.

### Existing foundations and current gaps

The existing foundations can be reused directly. Stage 2 does not describe the following capabilities as work to implement from scratch:

- The device/capability model, runtime write checks, safety policies, auditing, and simulated devices already exist.
- Temperature curves, numeric conditions, behavior when a condition is not met, handling of lost sensor contact, and fallback mechanisms already exist.
- Ownership checks already enforce a single enabled rule targeting `(device, capability)`. Paths for rejecting or disabling conflicting rules during save, enable, and load already exist.
- Handover records already cover exits from control such as disabling, deleting, or retargeting a rule. Persistence of unfinished handovers and unconfirmed writes, and revalidation after restart, also exist. A rule claiming ownership of a target does not mean that the device has confirmed the takeover.
- Adapter entry points such as system, LHM, and NVIDIA already exist. The fake LHM HTTP server and protocol simulation tests demonstrate parts of the software path; they do not prove that a particular motherboard or graphics card has passed real read/write acceptance testing.

This stage must complete an auditable inventory of whole-system sensor coverage, persistent physical identity across adapters, mappings of actual capabilities for more motherboard headers and GPUs with multiple fans, background privilege mechanisms, and real-hardware evidence for coexistence with manufacturer software and restoration of control. The current temporary rule that selects the NVIDIA provider based on whether LHM is enabled cannot replace physical identity or source selection for each capability.

Foundational references:

- [Requirements status and existing rule, fallback, and handover behavior](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/requirements.md)
- [Automation documentation](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/automation.md)
- [Existing ownership and handover implementation](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-automation/src/engine.rs)
- [Existing safety policies](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/crates/ohm-runtime/src/safety.rs)

### Hardware scope and boundaries

This stage focuses on whole-system cooling on Windows, while retaining shared models and existing capabilities on other platforms. It does not promise equivalent monitoring or control coverage on those platforms.

| Hardware category | Scope that must be specified in this stage | Capabilities that must not be assumed to exist |
| --- | --- | --- |
| CPU | Model, load, and temperature, power, frequency, and other data actually supplied by the hardware/provider; clarify the meaning of package, core, and other readings | Every CPU exposes every temperature/power field, or readings with the same name from different providers are fully equivalent |
| GPU | Persistent identity for each card, available temperature/load/power data, and RPM and control ranges for each fan or fan group | Every graphics card exposes hotspot readings, independent control of each fan, or hardware curve readback |
| Motherboard and fan headers | For board models included in the matrix, record header identifiers, RPM, write capability, control modes, and restoration behavior; cover the multiple headers that actually exist | Every reading labeled as a fan has a corresponding writable channel, or every header supports the same PWM/DC behavior |
| Memory | Capacity/utilization; record sources and support status for hardware that actually provides additional data such as module temperature | Ordinary memory always allows DIMM temperature or SPD reads, or provides active cooling control |
| Storage | Device identity, capacity/usage, and accessible temperature, health, and other fields; clarify the relationship between system volumes and physical drives | Every SATA/NVMe device or external enclosure passes through the same data, or storage readings can serve as direct control interfaces |
| Cooling channels | CPU cooler fans, case fans, other motherboard fan headers, and GPU fans whose controllability has been verified; identify pump headers by type and preserve existing safety boundaries | Treating a pump as an ordinary fan for stop tests, or expanding this stage into complete liquid-cooling management |

The specific motherboard, graphics card, CPU, memory, and storage samples will be determined in substage 2.1. This issue does not declare any model supported in advance, or set unconfirmed sample counts, duty cycles, temperature thresholds, or extended-test durations.

### Dependencies for entering this stage

1. The definition of done, unresolved items, and existing evidence from Stage 1 (https://github.com/SvenKunkka/OpenHardwareOS/issues/2) must be available for reference. When a dependency is incomplete, the corresponding work item in this stage remains incomplete; delivery of the preceding stage must not be assumed.
2. Before real testing, an identifiable inventory of test systems, boards/cards, and headers must be available, together with the current firmware, driver, provider, and application build identifiers. Control ceilings, minimum safe outputs, and stopping conditions must be recorded in advance for the samples concerned.
3. For paths that depend on LHM, specify the conditions for users to install/run LHM, enable its data interface, and obtain the necessary drivers/permissions. The software being able to access HTTP does not mean the target motherboard has a writable `Control` channel.
4. If a new background helper is required, first record the chosen process boundaries, installation and uninstallation paths, privileges, and communication constraints. Task Scheduler, Windows service, or sidecar approaches in the related ADRs are still inputs to the design; they must not be described as installed, accepted, or committed for a particular release.
5. Every real write requires a verifiable release/restoration path. Samples for which reliable restoration is unavailable may remain in monitoring mode with the limitation documented. Control must not be obtained by bypassing firmware protections or automatically terminating manufacturer processes.

Related design references: [LHM integration](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0003-libre-hardware-monitor-integration.md), [the vendor SDK and AMD proposal awaiting approval](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0004-vendor-sdks-and-amd.md), and [Windows privileges and autostart](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0006-windows-privileges-and-autostart.md). Material that remains Proposed or Planned requires an explicit decision and must not be treated as a committed deliverable.

### Eight substages

#### 2.1 Define the whole-system compatibility matrix and validation entry criteria

- Complete substage 2.1: produce a traceable inventory of hardware and capabilities.

**Objective:** Break "support this computer" down into verifiable devices, sensors, and control channels, specifying the intended coverage, dependencies, and areas where evidence is still missing.

**Deliverables:** A compatibility and validation matrix committed to the repository, recording model, hardware revision, BIOS/firmware, driver, provider, application build, monitoring fields, write operations, permission requirements, and restoration methods by system, board/card, and header. Separately list samples explicitly excluded from this stage and the reasons for exclusion.

**Reviewable acceptance criteria:**

- CPU, GPU, motherboard, memory, and storage each have entries and support statuses. Categories do not disappear from the table because they have "no readings."
- Supported, partially supported, unsupported, and awaiting real-hardware validation have consistent definitions. Entries with real-hardware conclusions can be traced to samples and evidence; simulation results are labeled separately.
- Every channel intended for real control has an identifier, an associated fan/fan group, safety limits confirmed in advance, and a method for restoration on exit. Channels lacking these prerequisites do not enter write acceptance testing.
- The rationale for any requirements concerning representativeness, sample counts, and continuous-run duration is documented in the matrix first. This issue does not substitute for sample selection that has not yet been completed.

#### 2.2 Establish persistent physical identity and deduplication across adapters

- Complete substage 2.2: give the same physical device and control channel a consistent, traceable identity.

**Objective:** When multiple adapters discover the same GPU or motherboard channel, the interface and rules can recognize their relationship. Restarts, changes in enumeration order, or provider switches must not bind existing rules to different hardware.

**Deliverables:** Identity conventions for physical devices and physical control channels, provider source fields, deterministic selection of reading sources and write paths, deduplication implementation, a strategy for compatibility with old identifiers or renewed confirmation, and corresponding migration documentation.

**Reviewable acceptance criteria:**

- The same graphics card appearing through LHM and NVML can be represented as one physical device while preserving the actual source of each capability. Distinct physical devices of the same model must not be merged.
- Identity and rule targets remain correct after restart, rediscovery, device-order changes, and failure/recovery of the selected provider. Insufficient or ambiguous identity explicitly requires renewed confirmation; the system must not guess a mapping and then write automatically.
- Different adapter aliases for the same physical output are mapped to one canonical target before entering the existing owner checks. They must not become two write entry points that bypass conflict checks.
- Available sensors are not discarded without explanation during deduplication. When sources conflict, the basis for selection is visible. Migration of old configurations must not treat unconfirmed control state as a completed takeover.

Reference: [Current LHM/NVML precedence rules and the unimplemented physical identity design](https://github.com/SvenKunkka/OpenHardwareOS/blob/main/docs/decisions/0005-nvml-precedence-and-device-identity.md). This substage does not introduce competition among multiple rules, dynamic bidding, or a general-purpose complex arbitration framework.

#### 2.3 Complete monitoring coverage for CPU, GPU, motherboard, memory, and storage

- Complete substage 2.3: validate the coverage, quality, and explainability of whole-system monitoring data within the matrix.

**Objective:** Enable users to identify the temperature and load signals they need and understand missing, stale, or abnormal data, so that cooling is not driven by data that appears complete but is actually incorrect.

**Deliverables:** Monitoring coverage tables for the five device categories, field meanings and units, source precedence, sampling and staleness policies, handling of invalid values, and consistent UI/CLI presentation and diagnostic output.

**Reviewable acceptance criteria:**

- For every field agreed in the matrix, record its specific status: readable, not provided by the hardware, unsupported by the provider, insufficient permissions, or awaiting validation. Do not invent zero values or substitute another temperature for the intended temperature.
- Units for temperature, RPM, percentage, power, and other readings, and the devices to which sensors belong, are correct. The actual meanings of same-named readings can be distinguished. Genuine zero RPM from a stopped GPU fan, disconnection, and stale data are not conflated.
- Sample readings are reviewed using sources and comparison methods recorded in advance. Deviations, refresh intervals, and tolerances have a stated basis; data from different sampling windows is not forced to count as equivalent.
- Existing contact-loss/fallback policies apply when a source disappears, recovers, returns invalid values, or updates late. Rules must not treat stale or invalid values as new evidence. Add regression coverage and real-hardware records for new sources.

#### 2.4 Expand and validate control across multiple motherboards and fan headers

- Complete substage 2.4: validate identification, reading, writing, and restoration for multiple physical headers on the motherboards in the matrix.

**Objective:** Users can distinguish and operate CPU fan, case fan, and other headers that are actually controllable, avoiding control of the wrong neighboring header, false support claims, or treating pump headers as ordinary fan headers.

**Deliverables:** Header mappings by board model/firmware, adapter capability additions, read-only and writable limitations, channel identification documentation, test methods, and acceptance records for each header.

**Reviewable acceptance criteria:**

- Cover the different motherboards and header layouts agreed in the matrix. Every header labeled controllable has a verified physical mapping, reading source, and write evidence; these must not be inferred solely from a SuperIO name or channel index.
- Operating one header does not change an unselected independent header. Channels that are physically linked or share control are represented as groups, with their relationships stated explicitly.
- Write ranges, minimum safe outputs, and modes such as PWM/DC are constrained by capabilities. Refusals, insufficient permissions, and firmware takeovers are accurately visible. "Request sent" is not treated as write confirmation.
- After disabling a rule, retargeting, stopping a provider, or exiting the application, verify the existing safe handover and manufacturer-control restoration behavior the adapter can provide for each header. Headers lacking control or restoration capabilities remain documented limitations.

#### 2.5 Complete the actual channel model for GPUs with multiple fans

- Complete substage 2.5: validate monitoring, independent/grouped control, and restoration boundaries for graphics cards with multiple fans.

**Objective:** Avoid reducing a graphics card with multiple fans to one misleading unified RPM reading, and avoid offering nonexistent per-fan control on hardware that only supports grouped control.

**Deliverables:** A matrix of GPU model/driver and provider combinations, mappings between physical fans and logical control groups, capability declarations for each item, documentation of NVIDIA/AMD support differences, and failure diagnostics.

**Reviewable acceptance criteria:**

- For graphics cards with multiple fans included in the matrix, identify which fan or group each RPM reading represents. When per-fan readings are unavailable, show the actual capability rather than duplicating one reading to imply complete coverage.
- Independent adjustment is shown only for outputs verified by actual testing to be independently controllable. Bound or linked fans are represented as one control group and retain the owner constraint for one canonical target.
- Record the behavior of stopped fans, zero-RPM modes, minimum acceptable outputs, driver refusals, and manufacturer automatic modes. Validate control and restoration within confirmed safe ranges; do not disable driver protections to expand capabilities.
- Provider switching, simultaneous visibility through LHM and NVML, and driver or permission failures do not result in duplicate writes, the wrong card being controlled, or the wrong fan being controlled. Unverified AMD/NVIDIA models remain explicitly labeled unverified.

#### 2.6 Validate existing single-target ownership and safe handover across system channels

- Complete substage 2.6: retain the existing mechanisms and complete evidence for conflicts and handovers across multiple channels and adapters.

**Objective:** As coverage expands to the whole system, each canonical output still has at most one enabled rule owner. When control stops, safe handover remains visible, uses bounded retries, and can be recovered.

**Deliverables:** Integration of canonical targets with the existing owner mechanism, regression cases covering every rule entry path, handover records for real channels, and any necessary diagnostic additions. New work is limited to integration and fixing defects discovered along the way.

**Reviewable acceptance criteria:**

- During save, enable, import/reload, and use of aliases across adapters, two rules cannot simultaneously acquire the same canonical output. Different physical channels can have independent rules; the whole motherboard must not mistakenly be locked as a single channel.
- Reuse existing numeric conditions, curves, behavior when conditions are not met, and contact-loss fallback to validate behavior after new sensors and channels are integrated. Do not redesign these mechanisms.
- Disabling, deleting, retargeting, source disappearance, application restart, and recovery of unconfirmed writes all have traceable outcomes. A declared owner is not a confirmed takeover, and old handovers must not overwrite the current confirmed owner's output.
- Handover failures remain visible and retries remain bounded. After restart, revalidate against the current device, capabilities, and owner rather than replaying old writes. Fallback duty and "returning control to firmware" must be recorded as different outcomes.
- An adapter that cannot reliably perform a runtime release must not be given a fictitious release action. Existing refusal/degradation boundaries remain in place. Actual restoration of firmware control is marked complete only when the adapter supports it and evidence confirms it.

#### 2.7 Specify background execution, privileges, and the installation lifecycle

- Complete substage 2.7: deliver and validate the selected minimal background privilege mechanism and its lifecycle.

**Objective:** An unprivileged interface accurately explains hardware access requirements. Cooling tasks that must run continuously have an explicit background component, failure behavior, and recovery path, rather than relying on an "autostart" switch to grant privileges implicitly.

**Deliverables:** A decision record defining the helper/sidecar/service boundaries and user-facing behavior selected for this stage; implementation of the chosen minimal approach and necessary installation, upgrade, uninstallation, and communication controls; and diagnostics for missing LHM, drivers, or permissions.

**Reviewable acceptance criteria:**

- The responsibilities of the normal UI, hardware access process, and external provider are clear. Actions requiring privileges, authorization points, and failure messages are reviewable. HKCU autostart must not be treated as a privileged background service.
- If local IPC is used, caller permissions, message validation, and permitted hardware actions have explicit boundaries. Do not expose arbitrary command execution or unrestricted write interfaces.
- Validate UI closure, logon/logoff, restart, sleep/resume, and dependency process exit against the selected product behavior. Whether the background component continues running or stops safely must match the documented behavior; checking only that "the process is still running" is insufficient.
- Installation, upgrades, and uninstallation do not leave invalid paths, duplicate background instances, or unexplained control ownership. When the background component is unavailable, foreground diagnostics and the existing contact-loss/safe-handover mechanisms reflect the actual state.
- Do not make every candidate mechanism a mandatory deliverable. The selected approach and lifecycle behavior it does not cover must be explicit before acceptance testing. Keep Windows test records separate from simulations and static checks.

#### 2.8 Complete manufacturer-software coexistence, restoration, and whole-system acceptance evidence

- Complete substage 2.8: complete evidence within the matrix for coexistence with manufacturer software, restoration on exit, and end-to-end whole-system behavior.

**Objective:** Users understand the control relationships among OpenHardwareOS, BIOS/firmware, LHM, and graphics card or motherboard manufacturer software. When contention, exit, or failure occurs, the system can return to an explicit, verified state.

**Deliverables:** A coexistence matrix by board model/graphics card/software combination, conflict messages and handling instructions, restoration instructions, a whole-system acceptance report, and an unsupported-items list. Provide the next stage with referenceable interfaces for canonical identity, capabilities, and the control lifecycle.

**Reviewable acceptance criteria:**

- For manufacturer-software combinations selected in the matrix, record whether simultaneous monitoring, exclusive control, write contention, or an unconfirmed state applies. If an external writer cannot be identified reliably, state that limitation; do not claim automatic detection of all conflicts.
- Do not automatically terminate manufacturer programs, disable system/firmware protections, or repeatedly write to seize control. When users need to choose the controlling component, identify the specific physical channels affected.
- For every control path declared supported, validate the actual output or mode after normal exit, disablement, dependency crash/unavailability, restart, and recovery. A successful interface response without sufficient device evidence remains unconfirmed.
- Record restoration of manufacturer automatic mode, maintenance of safe fallback output, and the need for manual restoration separately. Do not display "manufacturer control restored" when renewed manufacturer control cannot be demonstrated.
- Complete end-to-end records using the agreed whole-system workloads, sampling methods, stopping conditions, and durations. Every support conclusion can be linked to samples, builds, configurations, steps, readings/logs, and outcomes. Remove personal information and credentials that must not be public from published materials.
- Hand reusable interfaces, limitations, and remaining issues to the next stage (https://github.com/SvenKunkka/OpenHardwareOS/issues/4). Linking the next stage does not mean it has started, has an assigned version, or must wait for every non-blocking enhancement in this stage.

### Definition of done for this stage

This stage may be closed only when all the following conditions hold. Passing software tests or demonstrating the interface alone does not meet the definition of done.

1. All eight substages above have referenceable deliverables and acceptance conclusions, and the Stage 1 entry dependency has been confirmed against its definition of done.
2. The agreed scope of the five monitoring categories, multiple motherboard headers, and GPUs with multiple fans in the compatibility matrix has been resolved item by item. Each item is explicitly classified as verified supported, limited support, or confirmed unsupported. Incomplete real-hardware validation cannot be closed by relabeling it "supported."
3. Canonical physical identity, a single enabled rule owner per target, write confirmation, safe handover, and revalidation after restart form one traceable chain of evidence. No second complex arbitration system has been added to bypass existing safety constraints.
4. Background privileges and coexistence/restoration with manufacturer software meet the acceptance criteria for the selected approach. Behavior outside that coverage is visible in the user documentation.
5. Existing numeric-condition, fallback, and handover regression tests continue to pass. Simulation results, real application-path results, and real-hardware results are labeled separately, and every known failure has a documented disposition.
6. Stage completion, release packaging, and actual hardware compatibility are recorded as separate conclusions. Closing this issue does not automatically assign a release date, create a release, expand the scope of support, or approve hardware mass production.

### Out of scope

- Building a new general-purpose complex arbitration system, a marketplace for priorities among multiple rules, or automatic preemption of manufacturer control. This stage retains the existing single enabled owner per target and safe handover.
- Relisting existing numeric conditions, temperature curves, contact-loss fallback, or handover persistence as "development from scratch."
- Promising universal support for all motherboards, graphics cards, or memory/storage sensors, or treating LHM/driver visibility as proof of real-hardware writes.
- Physical OpenHub/OpenFan hardware design, new USB HID/CDC device bus integration, firmware upgrades for self-developed devices, and peripheral features assigned to other stages.
- Complete liquid-cooling/pump-system management, overclocking not separately defined, voltage adjustment, BIOS/EC modification, or disabling protections. Identifying pump headers and preventing incorrect control remain within this stage's boundaries.
- Introducing closed vendor SDKs without a separate decision, redistributing drivers/vendor binaries, or treating licensing and vendor policies that remain Proposed as approved.
- Full-feature support for other operating systems not specified in this stage, executing installation or publication, assigning owners, or making date and budget commitments.


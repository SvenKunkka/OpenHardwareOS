# 硬件支持六阶段计划（仓库快照）

> **语言 / Language:** 中文（本页）· [English summary](#english-summary)

本目录把「硬件支持六阶段」路线图落成随源码版本化的文件。在此之前，这份路线图
**只存在于 GitHub Issues**（#1–#7），不进版本库、不进评审、也不随源码备份；本目录
消除了这个缺口。

## 权威与同步规则

| 载体 | 角色 |
|---|---|
| GitHub Issues [#1](https://github.com/SvenKunkka/OpenHardwareOS/issues/1)–[#7](https://github.com/SvenKunkka/OpenHardwareOS/issues/7) | **实时进度记录**。复选框在 Issue 里勾选，进度以 Issue 为准 |
| 本目录下的文件 | **快照**：Issue 正文的逐字节镜像，随源码提交保存，**不会自动同步** |

两者必然随时间分叉，这是设计选择而非缺陷：源码库保证"计划不会丢失、改动可追溯、
可离线阅读"，Issue 保证"进度只有一个可写入口"。发现问题时以 Issue 为准，并按下面的
方式重新抓取快照。

## 快照来源与可复现方式

- 来源：`gh api repos/SvenKunkka/OpenHardwareOS/issues/<n> --jq .body`
- 抓取时间：**2026-09-14 18:45 CST**（2026-09-14T10:45:43Z）
- 快照时的仓库提交：`2aea128b1bea6170b801a1a1af4724d0efa0b500`
- 镜像方式：正文逐字节复制，不做任何改写；每个文件顶部只加一个来源头（Issue 编号、
  标题、抓取时间、快照提交），因此可用"去掉前 12 行后与 Issue 正文 `diff`"直接校验。
- 快照时 56 个复选框（总任务 6 个 + 50 个小阶段）**全部未勾选**。

重新抓取（需要 `gh` 已登录）：

```bash
for n in 1 2 3 4 5 6 7; do
  gh api "repos/SvenKunkka/OpenHardwareOS/issues/$n" --jq .body
done
```

## 阶段索引

阶段编号、小阶段编号均沿用 Issue 原文；"状态"一行照抄 Issue 正文，未做润色。

| 阶段 | 文件 | Issue | 小阶段 | 状态 |
|---|---|---|---|---|
| 总任务 | [`overview-six-stage-plan.md`](overview-six-stage-plan.md) | [#1](https://github.com/SvenKunkka/OpenHardwareOS/issues/1) | 6 个大阶段 / 50 个小阶段 | 总规划与统一交付标准 |
| 阶段一 · v0.2.0 单水泵支持 | [`stage-1-single-pump-support.md`](stage-1-single-pump-support.md) | [#2](https://github.com/SvenKunkka/OpenHardwareOS/issues/2) | S1.1–S1.8（8） | 计划中，目标版本 v0.2.0 |
| 阶段二 · 完善整机散热 | [`stage-2-whole-pc-cooling.md`](stage-2-whole-pc-cooling.md) | [#3](https://github.com/SvenKunkka/OpenHardwareOS/issues/3) | 2.1–2.8（8） | 未排期 |
| 阶段三 · USB 水冷、传感器与自研控制器 | [`stage-3-usb-liquid-cooling-sensors-and-controllers.md`](stage-3-usb-liquid-cooling-sensors-and-controllers.md) | [#4](https://github.com/SvenKunkka/OpenHardwareOS/issues/4) | 3.1–3.12（12） | 规划草稿，尚未排期 |
| 阶段四 · 灯光与屏幕 | [`stage-4-lighting-and-displays.md`](stage-4-lighting-and-displays.md) | [#5](https://github.com/SvenKunkka/OpenHardwareOS/issues/5) | 4.1–4.8（8） | 规划草稿，尚未排期 |
| 阶段五 · 输入设备与电源 | [`stage-5-input-devices-and-power-supplies.md`](stage-5-input-devices-and-power-supplies.md) | [#6](https://github.com/SvenKunkka/OpenHardwareOS/issues/6) | 5.1–5.7（7） | 后续规划，尚未排期 |
| 阶段六 · 平台扩展与开放生态 | [`stage-6-platform-expansion-and-open-ecosystem.md`](stage-6-platform-expansion-and-open-ecosystem.md) | [#7](https://github.com/SvenKunkka/OpenHardwareOS/issues/7) | 6.1–6.7（7） | 远期规划，尚未排期 |

## 与其它文档的关系

三套编号各自回答不同问题，不要互相替代：

| 文档 | 回答的问题 | 编号 |
|---|---|---|
| 本目录 | 按什么顺序、分几步把硬件支持做完 | 六阶段 / S1.1–6.7 |
| [`docs/roadmap.md`](../../roadmap.md) | 每个能力域现在处于什么状态（能力清单，不是版本号） | C1–C8 |
| [`docs/plans/pump-support.md`](../pump-support.md) | v0.2.0 这一版具体要交付什么、怎么验收 | PUMP-01–PUMP-04 |
| [`docs/versions.json`](../../versions.json) | 已经发布了什么、证据链接在哪 | v0.1.0 / v0.1.1 / v0.2.0(计划) |

阶段一的小阶段与水泵计划里的工作包是**对应关系**，不是同一套编号：S1.1–S1.3 ↔
PUMP-01、S1.4–S1.5 ↔ PUMP-02、S1.6–S1.7 ↔ PUMP-03、S1.8 ↔ PUMP-04（见阶段一文件与
总任务文件正文）。

## 不作出的声明

- 本目录是**计划文本**，不含任何硬件证据。它不改变
  [`docs/versions.json`](../../versions.json) 里 `hardware_validation: not_verified`
  的状态，也不构成"某型号已通过兼容验收"。
- 阶段五、阶段六在 Issue 正文中明确写着"不表示已经找到兼容型号、供应商或客户需求"。
- 勾选一个小阶段需要关联交付物与验收证据；完成一个阶段需要满足该 Issue 的完成定义。
  真实硬件的结论只能来自实机测量，软件测试、模拟器结果与"传输成功"都不算。

## English summary

This directory carries the six-stage hardware-support roadmap **as versioned repository
files**. The roadmap previously lived only in GitHub issues #1–#7, outside the source
tree. Each file is a byte-for-byte mirror of one issue body plus a provenance header
(issue number, title, capture time, snapshot commit); the issues remain the live progress
record and these snapshots do not auto-sync. See the table above for the stage → file →
issue mapping. Nothing here is hardware evidence: the version catalogue still records
`hardware_validation: not_verified` for every release.

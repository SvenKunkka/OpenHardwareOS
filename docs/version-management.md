# 版本管理

[打开交互版本树](https://svenkunkka.github.io/OpenHardwareOS/) · [GitHub 文本版本树](versions.md) · [更新记录](../CHANGELOG.md)

版本树用于选择发行版、查看改动、找到源码和安装入口。点击左侧版本查看详情；支持搜索、展开收起、方向键导航和复制版本号。下载 `version-tree.html` 后也能离线打开，访问外部链接仍需联网。

每个节点有明确状态：**计划中**只有范围和验收计划；**开发中**对应正在维护的分支；**已发布**必须对应真实的 GitHub Release、tag、源码提交和发布清单。预览版已经发布，也不代表真实风扇或水泵已经通过验收。版本树描述发行版之间的关系，完整提交历史仍保存在 Git 中。

## 日常查看与维护

仓库根目录运行，使用 Python 3.11 或更新版本：

```bash
python scripts/versions.py tree
python scripts/versions.py show v0.2.0
python scripts/versions.py check --generated
```

`docs/versions.json` 是唯一版本数据源。修改节点标题、范围、计划或证据后，重新生成页面和文本树：

```bash
python scripts/versions.py render
python scripts/versions.py check --generated
```

不要单独修改生成的 `docs/version-tree.html` 或 `docs/versions.md`。页面样式和交互在 `scripts/templates/version-tree.html` 中维护。CI 会检查生成文件是否与数据一致；推送到 `main` 后，版本树页面自动发布。

## 开始下一版本

水泵支持已登记为 `v0.2.0`，具体范围见[水泵计划](plans/pump-support.md)。当前开发版本完成发布后，再创建对应开发分支并准备下一版：

```bash
git switch main
git pull --ff-only
git switch -c codex/pump-support
python scripts/versions.py prepare v0.2.0 --branch codex/pump-support
python scripts/versions.py prepare v0.2.0 --branch codex/pump-support --apply
python scripts/versions.py render
python scripts/versions.py check --generated
```

第一次 `prepare` 只显示变更，带 `--apply` 才写入。工具同步 Rust workspace、内部依赖和锁文件、桌面 npm 包及锁文件、Tauri 版本，并把计划节点设为开发中。它不会创建远端分支、tag 或 Release。文件更新失败时会尝试恢复原内容；如遇并发编辑或恢复失败，会保留备份并报告路径，需检查后再继续。

发布前还要把 README 的第一个 PowerShell 安装块、其他安装示例及 `docs/windows-install.md` 更新为目标版本；这些文档由维护者审核，不由版本工具批量改写。公开安装检查会读取目标 tag 的 README，发现版本不符立即停止。

## 发布一版并留下证据

1. 提交目标源码，运行 CI，并以明确的 `vMAJOR.MINOR.PATCH` 触发 `Windows release assets`。工作流检查版本、测试、构建 CLI 和桌面安装包，输出安装文件、SHA-256 和包含源码提交的 `release.json`。
2. 检查成功工作流的源码提交、下载文件和摘要，再针对该提交创建 tag 和 GitHub Release；早期版本标记为预览版。已经发布的 tag 和安装文件保留不动，修复应使用新的版本号。
3. 公开发布后，将真实发行信息登记到版本清单。需要已登录并能读取仓库的 GitHub CLI：

```bash
python scripts/versions.py record-release v0.1.1
python scripts/versions.py record-release v0.1.1 --apply
python scripts/versions.py render
python scripts/versions.py check --remote --generated
```

工具核对实际 Release、tag 指向的提交和 `release.json` 的源码提交。草稿、未发布版本或来源不一致的资产不能登记为已发布；已有版本的提交也不能被替换。

4. 以同一个版本号触发 `Published Windows CLI install check`。它在 Windows PowerShell 5.1 中执行该 tag README 的真实下载安装命令，验证版本、校验和、诊断及模拟运行。
5. 将成功的 CI、构建和公开安装验证链接加入该版本的 `evidence`，更新 `CHANGELOG.md`，重新生成版本树，再提交到 `main`。这次目录记录提交可以晚于发行提交，不能为了让二者相同而移动旧 tag。

实际硬件验收证据应单独记录。只有模拟测试或 GitHub Windows 安装成功时，`hardware_validation` 仍保持 `not_verified`。

# MC CLI Launcher

一个简洁的 Minecraft 命令行启动器，支持 Microsoft 登录、离线模式、Fabric / Forge / NeoForge 加载器，以及 Modrinth / CurseForge 双模组源。

## 功能

- **Microsoft 设备码登录** — 一行命令，无需复制粘贴 URL
- **离线模式** — 无需正版账号即可本地启动
- **多加载器支持** — Fabric、Forge、NeoForge 可选安装、并列独立
- **双模组源** — Modrinth 优先，未找到时自动回退 CurseForge；也可手动指定 `--source`
- **游戏启动** — 版本管理、Java 检测、内存配置、窗口分辨率
- **多线程下载** — 带进度条和 SHA-1 校验

## 下载

前往 [Actions](https://github.com/Hotsteel2901/mc_py_cli_launcher/actions) 页面，选择最新的构建，下载对应平台的可执行文件：

| 平台 | 文件名 |
|------|--------|
| Windows | `mc_launcher.exe` |
| Linux | `mc_launcher-linux` |
| macOS | `mc_launcher-macos` |

> Linux/macOS 下载后需要添加执行权限：`chmod +x mc_launcher-*`

## 使用教程

### 1. 登录

```bash
# 设备码登录（推荐，简单安全）
./mc_launcher login

# 浏览器登录（需复制粘贴 URL）
./mc_launcher login --browser

# 离线模式
./mc_launcher offline Steve
```

### 2. 安装模组加载器

```bash
# Fabric
./mc_launcher install-fabric -v 1.21.4

# Forge
./mc_launcher install-forge -v 1.20.1

# NeoForge
./mc_launcher install-neoforge -v 1.21.4

# 指定加载器版本
./mc_launcher install-fabric -v 1.21.4 --loader-version 0.16.10
```

### 3. 搜索模组

```bash
# 默认：Modrinth 优先，无结果回退 CurseForge
./mc_launcher search sodium

# 只搜索 Modrinth
./mc_launcher search sodium --source modrinth

# 只搜索 CurseForge（需要提供 API Key）
./mc_launcher search jei --source curseforge --curseforge-key YOUR_KEY

# 限制返回数量
./mc_launcher search iris --limit 5
```

> CurseForge API Key 也可通过环境变量 `CURSEFORGE_API_KEY` 设置。

### 4. 安装模组

```bash
# 自动检测加载器
./mc_launcher install-mod sodium -v 1.21.4

# 指定加载器
./mc_launcher install-mod iris -v 1.21.4 --loader fabric

# 指定模组版本
./mc_launcher install-mod sodium -v 1.21.4 --mod-version xxxxxxxx

# 从 CurseForge 安装
./mc_launcher install-mod jei -v 1.20.1 --source curseforge --curseforge-key YOUR_KEY
```

### 5. 启动游戏

```bash
# 启动指定版本
./mc_launcher play -v 1.21.4

# 启动最新版本（自动检测）
./mc_launcher play

# 使用指定加载器
./mc_launcher play -v 1.21.4 --fabric
./mc_launcher play -v 1.20.1 --forge
./mc_launcher play -v 1.21.4 --neoforge

# 指定 Java 路径
./mc_launcher play -v 1.21.4 --java "C:\Program Files\Java\jdk-21\bin\javaw.exe"

# 指定内存（支持 G/M 格式）
./mc_launcher play -v 1.21.4 --ram 4G
./mc_launcher play -v 1.21.4 --ram 2048M

# 指定窗口大小
./mc_launcher play -v 1.21.4 --width 1920 --height 1080
```

### 6. 其他命令

```bash
# 查看帮助
./mc_launcher --help

# 查看已登录账号
./mc_launcher accounts

# 登出
./mc_launcher logout

# 查看可安装的游戏版本
./mc_launcher list-versions

# 查看已安装版本
./mc_launcher list-installed

# 查看已安装模组
./mc_launcher list-mods -v 1.21.4

# 启用/禁用/卸载模组
./mc_launcher disable-mod sodium -v 1.21.4
./mc_launcher enable-mod sodium -v 1.21.4
./mc_launcher uninstall-mod sodium -v 1.21.4

# 仅下载游戏文件
./mc_launcher download -v 1.21.4
./mc_launcher download -v 1.21.4 --no-assets
```

## 从源码运行

```bash
# 克隆仓库
git clone https://github.com/Hotsteel2901/mc_py_cli_launcher.git
cd mc_py_cli_launcher

# 运行
python mc_launcher.py --help
```

## 依赖

- Python 3.10+（从源码运行时）
- 无需额外依赖，仅使用标准库

## 许可证

[GPL v3](LICENSE)

# MC CLI Launcher

一个简洁的 Minecraft 命令行启动器，支持 Microsoft 登录、离线模式、Fabric / Forge / NeoForge 加载器，以及 Modrinth 模组源。

## 功能

- **Microsoft 浏览器登录** — 默认方式，复制粘贴 URL 即可完成登录
- **离线模式** — 无需正版账号即可本地启动
- **多加载器支持** — Fabric、Forge、NeoForge 可选安装、并列独立，同一版本可安装多个加载器并在启动时切换
- **Java 自动匹配** — 按游戏版本要求自动选择本机 Java（8/17/21 等），没有时自动下载 Mojang 官方 Java 运行时（独立存放于 `minecraft/java/`，不影响系统）
- **Modrinth 模组源** — 搜索与安装，无需 API Key
- **加载器支持一览** — 搜索结果直接显示每个模组在 Fabric / Forge / NeoForge 下支持的最高游戏版本
- **精细搜索** — `search-more <slug>` 查看模组详细信息（支持版本表、最近发布、链接等）
- **依赖检测** — 安装模组时检测 required / optional 依赖并给出安装命令
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
# 浏览器登录（默认，需复制粘贴 URL）
./mc_launcher login

# 设备码登录（需要自备 Azure 应用，因为官方客户端 ID 不支持设备码）
./mc_launcher login --device-code

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
# 搜索 Modrinth 上的模组
# 每个结果附带 fabric / forge / neoforge 支持的最高游戏版本
./mc_launcher search sodium

# 限制返回数量
./mc_launcher search iris --limit 5

# 精细搜索：查看指定模组的详细信息（必须使用完整 slug）
# 显示下载量、客户端/服务端要求、加载器支持表、支持的 MC 版本、最近发布等
./mc_launcher search-more sodium
```

### 4. 安装模组

```bash
# 自动检测加载器
./mc_launcher install-mod sodium -v 1.21.4

# 指定加载器
./mc_launcher install-mod iris -v 1.21.4 --loader fabric

# 指定模组版本
./mc_launcher install-mod sodium -v 1.21.4 --mod-version xxxxxxxx
```

安装时会明确显示所装模组版本、要求的游戏版本和加载器；
若该模组没有适配当前版本/加载器，会列出它实际支持的最高版本，方便判断替代方案。
安装后如有缺失的 required 依赖，会给出对应的安装命令。

### 5. 启动游戏

```bash
# 启动指定版本
./mc_launcher play -v 1.21.4

# 启动最新版本（自动检测）
./mc_launcher play

# 使用指定加载器（同一版本可安装多个加载器，启动时切换）
./mc_launcher play -v 1.21.4 --fabric
./mc_launcher play -v 1.20.1 --forge
./mc_launcher play -v 1.21.4 --neoforge

# 指定 Java 路径（不指定时自动匹配版本要求的 Java，
# 本机没有则自动下载 Mojang 官方运行时）
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

## 注意事项

- **mods 目录按版本共享**（`minecraft/versions/<版本>/mods`），不区分加载器。
  同一版本装了多个加载器时，切换前建议用 `disable-mod` 禁用另一加载器的模组。
- Forge / NeoForge 启动时模组由加载器从 mods 目录自行发现，Fabric 亦然。
- 自动下载的 Java 运行时独立存放于 `minecraft/java/<component>/`，
  不修改 PATH / JAVA_HOME / 系统包管理器，删除该目录即可清除。
- Linux 上如果控制台出现 Narrator / `libflite.so` 报错，属于旁白语音库缺失，
  无害可忽略；安装系统包 `flite` 可消除。

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

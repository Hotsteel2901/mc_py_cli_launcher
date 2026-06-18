# MC CLI Launcher

一个简洁的 Minecraft 命令行启动器，支持 Microsoft 设备码登录、Fabric 模组加载器、Modrinth 模组管理。

## 功能

- **Microsoft 设备码登录** — 一行命令，无需复制粘贴 URL
- **Fabric 模组加载器** — 自动安装/管理
- **Modrinth 模组管理** — 搜索、安装、更新模组
- **游戏启动** — 版本管理、Java 检测、内存配置
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

```bash
<<<<<<< HEAD
python mc_launcher.py -h 
=======
# 设备码登录（推荐，简单安全）
./mc_launcher login

# 浏览器登录（需复制粘贴 URL）
./mc_launcher login --browser
```

设备码登录会显示一个代码，在浏览器打开 https://microsoft.com/link 并输入即可。

### 2. 安装 Fabric 模组加载器

```bash
# 安装最新版 Fabric（自动选择版本）
./mc_launcher install-fabric -v 26.1.2

# 指定 Fabric Loader 版本
./mc_launcher install-fabric -v 26.1.2 --loader-version 0.16.14
```

### 3. 搜索模组

```bash
# 在 Modrinth 上搜索模组
./mc_launcher search sodium

# 限制返回数量
./mc_launcher search iris --limit 5
```

### 4. 安装模组

```bash
# 安装模组（自动检测加载器）
./mc_launcher install-mod sodium -v 26.1.2

# 指定加载器
./mc_launcher install-mod iris -v 26.1.2 --loader fabric

# 指定模组版本
./mc_launcher install-mod sodium -v 26.1.2 --mod-version xxxxxxxx
```

### 5. 启动游戏

```bash
# 启动指定版本
./mc_launcher play -v 26.1.2

# 启动最新版本（自动检测）
./mc_launcher play

# 指定 Java 路径
./mc_launcher play -v 26.1.2 --java "C:\Program Files\Java\jdk-21\bin\javaw.exe"

# 指定内存（支持 G/M 格式）
./mc_launcher play -v 26.1.2 --ram 4G
./mc_launcher play -v 26.1.2 --ram 2048M

# 指定窗口大小
./mc_launcher play -v 26.1.2 --width 1920 --height 1080

# 启动 Fabric 模组
./mc_launcher play -v 26.1.2 --fabric
```

### 6. 其他命令

```bash
# 查看帮助
./mc_launcher --help

# 查看已登录账号
./mc_launcher accounts

# 查看已安装版本
./mc_launcher list-installed

# 查看已安装模组
./mc_launcher list-mods -v 26.1.2

# 管理 Modrinth 上的版本和加载器
./mc_launcher list-versions
./mc_launcher list-loaders
>>>>>>> 1299533 (fix: 修复安全问题、CLI不一致、文档错误，美化落地页)
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

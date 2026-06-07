# MC CLI Launcher

一个简洁的 Minecraft 命令行启动器，支持 Microsoft 账号登录、Fabric 模组加载器、Modrinth 模组管理。

## 功能

- **Microsoft OAuth2 登录** — 支持设备码流程
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

### 1. 登录 Microsoft 账号

```bash
./mc_launcher login
```

按提示在浏览器中完成 Microsoft 账号授权。

### 2. 安装 Fabric 模组加载器

```bash
# 安装最新版 Fabric（自动选择版本）
./mc_launcher fabric install 26.1.2

# 指定 Fabric Loader 版本
./mc_launcher fabric install 26.1.2 --loader 0.16.14
```

### 3. 搜索模组

```bash
# 在 Modrinth 上搜索模组
./mc_launcher mod search "sodium"

# 限制返回数量
./mc_launcher mod search "iris" --limit 5
```

### 4. 安装模组

```bash
# 安装模组（自动检测加载器）
./mc_launcher mod install sodium 26.1.2

# 指定加载器
./mc_launcher mod install iris 26.1.2 --loader fabric

# 指定模组版本
./mc_launcher mod install sodium 26.1.2 --version-id xxxxxxxx
```

### 5. 启动游戏

```bash
# 启动指定版本
./mc_launcher play 26.1.2

# 指定 Java 路径
./mc_launcher play 26.1.2 --java "C:\Program Files\Java\jdk-21\bin\javaw.exe"

# 指定内存
./mc_launcher play 26.1.2 --memory 4G

# 指定窗口大小
./mc_launcher play 26.1.2 --width 1920 --height 1080
```

### 6. 其他命令

```bash
# 查看帮助
./mc_launcher --help

# 查看已安装版本
./mc_launcher versions

# 查看已登录账号
./mc_launcher accounts
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

# rtdo — Root Task Do

一个可交互的、环境自适应的提权工具，用来替代传统 sudo。

- **认证**：以 **root 密码**（PAM）认证，不是你自己账户的密码；
- **策略**：信任目录自动放行、高危黑名单硬拦截、其余交互确认（终端 y/N 或图形弹窗）；
- **审计**：所有提权请求（放行/拒绝/拦截）写入审计日志；
- **双语**：所有输出中英双语，按 `RTDO_LANG` / `LANG` 自动选择；
- **可选禁用 sudo**：Go 守护进程 + systemd 服务拦截 `sudo`，`sudo-force` 调用原版 sudo。

## 这是什么

rtdo 是一个「带策略的 sudo」。你用它在终端里执行命令时，它会先校验 root 密码，然后根据配置文件决定怎么处理这条命令：

- **直接放行**：命令只碰信任目录里的东西；
- **问你**：拿不准的，让你在终端输入 y/N，或者在桌面上点弹窗；
- **直接拦下**：命中高危黑名单的命令，只有加 `--force` 才放行。

所有过程都会写进审计日志。

它和 sudo 的区别：sudo 验证一次密码之后就放行一切；rtdo 会对每条命令单独做判断，危险操作多一道确认，日常操作尽量不打扰。适合经常要在服务器（比如 NixOS）上以 root 跑命令、又不想让 `sudo !!` 变成习惯性动作的人。

## 特性（v0.5.0）

- 首次/每次运行自动检查 root 密码：
  - **未设置**（WSL 默认、发行版默认锁定 root）→ 强制引导设置（等价于自动执行 `sudo passwd`）：验证当前用户密码 → 输入两次新 root 密码 → 写入；
  - **过弱**（等于系统内任意一个用户名，或等于任意一个用户的密码）→ 同样强制重新设置；
  - 新 root 密码约束：不能等于任意用户名、不能与当前用户密码相同、长度 ≥ 6、不含 `:`/换行；
- **可选禁用 sudo（守护进程拦截，替代旧版别名方案）**：
  - **首次运行**时询问一次是否禁用 sudo（已询问过或已启用则不再提示）；
  - 启用后 `sudo` 被 Go 守护进程（`rtdo-sudod`，systemd 服务 `rtdo-sudod.service` 管理）拦截并提示改用 rtdo；
  - 确需原版 sudo 时用 `sudo-force <命令>`（原版 sudo 移动为 `/usr/bin/sudo.real`）；
  - 原版 `alias sudo=rtdo` 方案已废弃（别名挡不住真实存在的 sudo 二进制），启用拦截时会自动清理旧别名文件；
- **sudo 前置检查**：未安装 sudo 或当前用户不在 sudo/wheel 组 → 明确提示先安装并正确配置 sudo 再操作；
- **一键分发**：`rtdo.sh --install / --uninstall`（curl 拉源码编译，或 git clone 本地编译），详见「安装」；
- PAM 认证（root 密码）、策略判定（信任目录/黑名单）、审计日志（JSON/text）、中英双语输出。

## 架构

```
用户输入 rtdo <命令> / sudo <命令> / sudo-force <命令>
        │
        ├── rtdo（Rust，setuid root）
        │      ├── 配置加载 /etc/rtdo/rtdo.conf
        │      ├── PAM 认证（root 密码）
        │      ├── 策略判定 + 交互确认 + 审计
        │      └── 以 root 执行（环境清理）
        │
        ├── sudo（shim，Go）──> rtdo-sudod（Go 守护进程，unix socket /run/rtdo/sudo.sock）
        │        │                ├── 默认拦截：提示使用 rtdo
        │        │                └── 记录日志（journald）
        │        └── sudo-force ──> /usr/bin/sudo.real（原版 sudo）
        │
        └── systemd: rtdo-sudod.service（开机自启，守护进程管理）
```

- `rtdo`：Rust 实现的主程序（`src/`），策略、认证、交互、审计；
- `rtdo-sudod`：Go 实现（`daemon/`），守护进程 + sudo/sudo-force shim，监听 `/run/rtdo/sudo.sock`；
- `rtdo.sh`：安装/卸载管理脚本。

## 安装

### 前置条件

- Linux + systemd；
- Rust 工具链（`cargo`）与 Go 工具链（`go`，编译守护进程用）；
- libpam 开发头文件（`libpam0g-dev` / `pam-devel` / `pam`，脚本会自动安装）；
- **sudo 已安装并正确配置**（当前用户在 sudo/wheel 组）。未安装时 `rtdo.sh` 会提示先安装配置。

### 方式一：Reimilia 部署（推荐）

[Reimilia](https://github.com/Reimilia617/Reimilia) 是本项目的统一部署器。
项目根目录的 [`Reimilia_Setup/`](Reimilia_Setup/) 已经写好了 Build / Binary 两种安装说明。

```bash
# 启动 WebUI，在页面上点「部署」
reimilia

# 或者终端交互
reimilia --cli RTDO-Project --yes

# 只想要快：用预编译产物（需要 Release 里已经有对应架构的产物）
reimilia --cli RTDO-Project --kind binary --yes
```

默认走 **Build（源码编译）**：不依赖是否发布过产物，任何机器上都能装。
细节见 [Reimilia_Setup/README.md](Reimilia_Setup/README.md)。

### 方式二：curl 一键安装

```bash
curl -fsSL https://raw.githubusercontent.com/Reimilia617/RTDO-Project/main/rtdo.sh | sudo sh -s -- --install
```

脚本会从 GitHub 仓库拉取源码 → 编译 rtdo（Rust）与 rtdo-sudod（Go）→ 安装到系统 → 注册并启动 systemd 服务 → 拦截 sudo。

### 方式三：git clone 安装

```bash
git clone https://github.com/Reimilia617/RTDO-Project.git
cd RTDO-Project
sudo ./rtdo.sh --install
```

### 方式四：预编译二进制安装

如果 Release 里已经有产物（见 [`.github/workflows/release.yml`](.github/workflows/release.yml)，
打 `v*` tag 时自动构建 `rtdo-<arch>-linux` 与 `rtdo-sudod-<arch>-linux`），
可以跳过编译，直接用现成二进制：

```bash
mkdir -p /tmp/rtdo-prebuilt && cd /tmp/rtdo-prebuilt
base=https://github.com/Reimilia617/RTDO-Project/releases/latest/download
curl -fsSLO "$base/rtdo-x86_64-linux"
curl -fsSLO "$base/rtdo-sudod-x86_64-linux"
cd /path/to/RTDO-Project
sudo ./rtdo.sh --install --prebuilt-dir /tmp/rtdo-prebuilt
```

`--prebuilt-dir` 只跳过编译，安装步骤（setuid、配置、PAM、systemd 服务、sudo 拦截）
仍然由 `rtdo.sh` 独家完成，不会出现两套互相打架的安装逻辑。

### 安装内容

| 文件 | 说明 |
|---|---|
| `/usr/local/bin/rtdo` | 主程序（setuid root） |
| `/usr/local/lib/rtdo/rtdo-sudod` | Go 守护进程（也作为 sudo/sudo-force shim） |
| `/etc/rtdo/rtdo.conf` | 默认配置（`rtdo --init` 生成） |
| `/etc/pam.d/rtdo` | PAM 认证服务文件 |
| `/etc/systemd/system/rtdo-sudod.service` | systemd 服务（开机自启） |
| `/usr/bin/sudo` → shim | 原版 sudo 移动为 `/usr/bin/sudo.real` |
| `/usr/bin/sudo-force` | 调用原版 sudo 的入口 |
| `/etc/rtdo/sudo-disabled` | 拦截启用标记 |

## 卸载

```bash
sudo ./rtdo.sh --uninstall
```

会：停止并删除 systemd 服务 → 删除 `/usr/bin/sudo` shim 与 `sudo-force`，把 `/usr/bin/sudo.real` 还原为 `/usr/bin/sudo` → 删除配置、二进制与 PAM 文件。**sudo 完整还原为可用模式。**

> 注意：卸载时若 sudo 已被拦截，脚本内部操作不依赖 sudo（以 root 直接执行）。
> 如果手上只剩被 shim 接管的 `sudo`，可以用 `sudo-force ./rtdo.sh --uninstall`。

## 首次使用

rtdo 认证的是 **root 密码**（不是你自己账户的密码）。

**root 密码未设置或过弱**时（WSL 默认无密码、发行版默认锁定 root、密码等于用户名或其他用户密码），非 root 用户运行 `rtdo <命令>` 会自动进入**强制设置流程**（等价于自动执行 `sudo passwd`），无法跳过：

1. 验证你当前用户的密码（确认操作者身份，最多 3 次机会）；
2. 输入两次新的 root 密码（终端无回显 / 图形弹窗）；
3. 强制校验：不能等于系统内任意用户名、不能与当前用户密码相同、至少 6 个字符、不含 `:`/换行；
4. 写入新密码，然后继续用新密码完成本次认证。

也可以手动设置：

```bash
sudo passwd        # 普通用户执行（先验证你的用户密码）
# 或
su -c passwd       # 切到 root 后执行
```

> 无终端也无图形桌面的环境（如 cron）无法自动引导，rtdo 会拒绝执行并提示手动 `sudo passwd`。

## 禁用 sudo（可选）

**首次运行** `rtdo` 时会询问一次是否禁用 sudo；若已询问过（无论回答 y/n）或已启用拦截，之后不再提示。若首次运行时处于无交互环境，则不会记录，下次有机会再问。

选择「是」后：

- 原版 sudo 移动为 `/usr/bin/sudo.real`；
- `/usr/bin/sudo` 与 `/usr/bin/sudo-force` 变为 shim（Go 守护进程二进制，通过 argv[0] 区分）；
- 注册并启动 systemd 服务 `rtdo-sudod.service`（监听 `/run/rtdo/sudo.sock`）；
- 之后输入 `sudo <命令>`：守护进程拦截并提示：
  > sudo 已被 rtdo 接管：请使用 rtdo \<命令\> 执行；确需原版 sudo 请用 sudo-force \<命令\>
- 确需原版 sudo：`sudo-force <命令>`（例如 `sudo-force apt update`），行为与原版 sudo 完全一致；
- 拦截是 **fail-closed**：守护进程不在时 `sudo` 依然被拦截提示（不会偷偷放行原版 sudo）。

恢复原版 sudo：`sudo ./rtdo.sh --uninstall`（完整还原），或手动 `rm /usr/bin/sudo /usr/bin/sudo-force && mv /usr/bin/sudo.real /usr/bin/sudo`。

> 注：0.1.0 时代的全局别名方案（`/etc/profile.d/rtdo-alias.sh`）已废弃——别名挡不住真实存在的 sudo 二进制（脚本/非交互环境照样绕过）。新方案用文件级 shim + 守护进程，任何调用方式都无法绕过。

## 使用

```bash
# 普通提权：根据环境自动选择交互方式
rtdo nixos-rebuild switch

# 强制放行：跳过所有拦截（黑名单也放行，谨慎使用）
rtdo --force dd if=/dev/zero of=/dev/sda

# 查看当前策略
rtdo --policy

# 生成默认配置（需要 root）
sudo rtdo --init
```

其他参数：

| 参数 | 说明 |
|---|---|
| `--config <路径>` | 指定配置文件，默认 `/etc/rtdo/rtdo.conf`；也可用环境变量 `RTDO_CONF` |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

## 交互流程

| 环境 | 交互方式 |
|---|---|
| 图形桌面（Wayland/X11） | 弹窗显示操作摘要，等待点击「同意/拒绝」；需要 zenity、kdialog 或 yad，按顺序自动选择 |
| 终端会话 | 终端内输出操作摘要，等待输入 y / n（直接回车等于拒绝） |
| 两者都没有 | 无法确认，默认拒绝，记入审计日志 |

图形桌面下输入密码也用弹窗，终端下密码输入不回显。

## 配置文件

默认位置 `/etc/rtdo/rtdo.conf`，TOML 格式：

```toml
trusted_paths = [
    "/etc/nixos",
    "/home/*/.config",
    "/usr/local/bin"
]

blacklist_commands = [
    "dd",
    "mkfs",
    "rm -rf /",
    "chmod -R 777 /"
]

audit_log = "/var/log/rtdo.log"
log_format = "json"
```

- `trusted_paths`：按目录前缀匹配，支持 `*` / `**` / `?` 通配符；命令涉及的所有路径都在其中时自动放行；
- `blacklist_commands`：单命令名按 basename 精确匹配，带参数的命令按「命令名 + 参数前缀」匹配；命中直接拦截，`--force` 可放行；
- `audit_log` / `log_format`：审计日志路径与格式（`json` / `text`）。

## 审计日志

所有提权请求都会记录（时间、用户、命令、是否 `--force`、结果与原因、退出码）。原因包括：`trusted_path`、`user_confirmed`、`user_denied`、`blacklist`、`force`、`auth_failed`、`root_password_setup`、`root_password_setup_failed`、`not_root`、`pam_setup_failed` 等。

```json
{"time":"2026-08-28T19:00:00+08:00","user":"alice","uid":1000,"command":"nixos-rebuild switch","force":false,"result":"allow","reason":"user_confirmed","exit_code":0}
```

## 多语言

输出语言按以下规则自动选择（优先级从高到低）：

| 变量 | 取值 | 语言 |
|---|---|---|
| `RTDO_LANG` | `zh*` | 中文 |
| `RTDO_LANG` | 其他 | English |
| `LC_ALL` / `LC_MESSAGES` / `LANG` | 以 `zh` 开头 | 中文 |
| 其他 | 任意 | English |

```bash
LANG=zh_CN.UTF-8 rtdo --help      # 中文
RTDO_LANG=en rtdo --help          # English
```

## 退出码

| 场景 | 退出码 |
|---|---|
| 帮助 / 版本 / 初始化 / 查看策略成功 | 0 |
| 放行命令执行成功 | 0（或命令自身的退出码） |
| 拒绝 / 拦截 / 认证失败 / 执行失败 | 1 |
| 用法错误（未知选项、缺少命令） | 2 |
| 放行命令被信号终止 | 128 + 信号编号 |

## 安全说明

- 命令不经 shell：参数按字面传给命令，`$HOME`、管道、引号不会被解释；
- 环境清理：安全 PATH、HOME=/root、移除 LD_PRELOAD / LD_LIBRARY_PATH；
- 不含路径的命令在安全 PATH 中解析成绝对路径再执行，防 PATH 劫持；
- 非交互环境默认拒绝；sudo 拦截 fail-closed；
- 黑名单是硬拦截，`--force` 是唯一的例外，用之前想清楚。

## 已知限制

- root 账户被锁定或未设置密码时，首次/每次运行会自动引导设置；无交互环境则拒绝并提示手动 `sudo passwd`；
- 图形弹窗依赖 zenity / kdialog / yad，最小化安装的系统需要自己装一个；
- 路径提取是启发式的，复杂参数（通配符、重定向）可能识别不全，识别不了就进确认流程，不会悄悄放行；
- 只支持 Linux（PAM、systemd、unix socket 都是 Linux 的东西）；
- 弱密码检测会遍历系统用户做 PAM 校验，用户很多的机器上每次认证略有开销。

## 手动构建

```bash
# rtdo（Rust）
cargo build --release        # 产物 target/release/rtdo

# rtdo-sudod（Go）
cd daemon && CGO_ENABLED=0 go build -trimpath -buildvcs=false -ldflags "-s -w" -o rtdo-sudod .
```

## 测试

简单冒烟：`./rtdo --help`、`./rtdo --version`，`sudo ./rtdo --init` 之后 `./rtdo --policy`。

完整流程需要：setuid 安装 + root 密码（可直接用首次运行引导自动设置）+ 一个终端或图形桌面。黑名单拦截可直接用 `rtdo dd if=/dev/zero of=/dev/null bs=1 count=1` 试，应当直接报拦截。

## 代码结构

- `src/main.rs` — 参数解析与主流程
- `src/config.rs` — 配置加载与默认策略生成
- `src/policy.rs` — 策略判定（黑名单 / 信任目录 / 路径提取）
- `src/pam.rs` — PAM 认证（校验 root 密码、修改 root 密码）
- `src/auth.rs` — 密码输入（终端无回显 / 图形密码框）
- `src/interact.rs` — 环境自适应交互（终端 / 弹窗 / 拦截通知）
- `src/audit.rs` — 审计日志（JSON / text）
- `src/exec.rs` — 权限检查与安全执行
- `src/i18n.rs` — 中英双语支持
- `src/setup.rs` — 每次运行引导（root 密码检测与设置、弱密码检测、sudo 检查、守护进程拦截安装）
- `daemon/main.go` — rtdo-sudod：Go 守护进程 + sudo/sudo-force shim
- `rtdo.sh` — 一键安装 / 卸载（支持 `--prebuilt-dir` 跳过编译）
- `Reimilia_Setup/` — 接入 Reimilia 的安装说明（`Build` / `Binary`），见其 [README](Reimilia_Setup/README.md)
- `.github/workflows/release.yml` — 打 `v*` tag 时自动构建 Reimilia Binary 所需的 Release 产物
- `pkg/` — deb / rpm 打包文件

## 许可证与贡献者

MIT License，Copyright (c) 2026 Reimilia617

贡献者：

- **Reimilia617** — 项目发起、需求与产品设计
- **DeepSeek（AI 助手）** — 代码实现、架构设计与文档协作

发现 bug 或有更好的想法，欢迎直接提 issue。

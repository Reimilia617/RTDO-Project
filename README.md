# rtdo — Root Task Do

一个可交互的、环境自适应的提权工具，用来替代传统 sudo。

## 这是什么

rtdo 是一个「带策略的 sudo」。你用它在终端里执行命令时，它会先校验 root 密码，然后根据配置文件决定怎么处理这条命令：

- **直接放行**：命令只碰信任目录里的东西；
- **问你**：拿不准的，让你在终端输入 y/N，或者在桌面上点弹窗；
- **直接拦下**：命中高危黑名单的命令，只有加 `--force` 才放行。

所有过程都会写进审计日志。

它和 sudo 的区别：sudo 验证一次密码之后就放行一切；rtdo 会对每条命令单独做判断，危险操作多一道确认，日常操作尽量不打扰。适合经常要在服务器（比如 NixOS）上以 root 跑命令、又不想让 `sudo !!` 变成习惯性动作的人。

## 设计目标

- 默认拦截：除非用户明确允许，任何提权操作都不会执行。
- 环境自适应：在图形桌面下弹窗提醒，在终端中输出文本提示。
- 策略驱动：通过配置文件定义「信任目录」与「高危操作」，仅对真正危险的操作进行拦截。
- 单文件部署：一个二进制文件 + 一个配置文件，复制到对应目录即可使用。
- 安装需 root：rtdo 本身需要由 root 用户安装到系统目录。

## 工作流程

一次提权请求的完整流程：

1. **权限检查**：进程必须拥有 root 权限（二进制以 setuid root 安装，或直接由 root/sudo 调用）。
2. **加载策略**：读取 `/etc/rtdo/rtdo.conf`。
3. **认证**：普通用户调用时，要求输入 **root 密码**（PAM 校验，不是你自己账户的密码），错误最多重试 3 次；root 直接调用则跳过。若 root 密码尚未设置，首次运行会自动引导设置（见「首次使用：设置 root 密码」）。
4. **策略判定**：
   - 命中黑名单 → 直接拦截，弹窗或终端告知原因；
   - 命令涉及的全部路径都在信任目录内 → 自动放行；
   - 其他情况 → 交互确认（终端 y/N 或图形弹窗「同意/拒绝」）。
5. **执行**：确认后以 root 身份运行命令。命令不经 shell，直接 exec；环境做了清理（安全 PATH、HOME=/root、移除 LD_PRELOAD 等）。

非交互环境（没有终端也没有图形桌面，比如被 cron 调用）默认拒绝。

## 安装

需要 Linux + Rust 工具链 + libpam 开发头文件：

```bash
# Debian/Ubuntu
sudo apt install libpam0g-dev
# Fedora
sudo dnf install pam-devel
# Arch
sudo pacman -S pam

cargo build --release
```

安装到系统目录并设置 setuid：

```bash
sudo cp target/release/rtdo /usr/local/bin/rtdo
sudo chown root:root /usr/local/bin/rtdo
sudo chmod u+s /usr/local/bin/rtdo

sudo mkdir -p /etc/rtdo
sudo rtdo --init
```

`--init` 会生成默认配置 `/etc/rtdo/rtdo.conf`，同时生成 PAM 认证服务文件 `/etc/pam.d/rtdo`（rtdo 靠它校验 root 密码；如果该文件缺失，首次运行时也会自动补建）。

### 安装前置：需要 sudo

rtdo 的安装与日常运维（如手动 `sudo passwd`）依赖 sudo，因此**安装 rtdo 前请先确认 sudo 已安装并正确配置**（当前用户已在 sudo/wheel 组）：

```bash
# Debian/Ubuntu
sudo apt install sudo && sudo usermod -aG sudo <你的用户名>
# Fedora/RHEL
sudo dnf install sudo && sudo usermod -aG wheel <你的用户名>
# Arch
sudo pacman -S sudo && sudo usermod -aG wheel <你的用户名>
# 加入组后需重新登录生效
```

rtdo 会在 `--init` 和首次运行引导时自动检测：**如果检测到 sudo 缺失或当前用户不在 sudo/wheel 组，会明确提示你先安装并正确配置 sudo，再继续安装 rtdo**。

为什么要 setuid：rtdo 需要以 root 身份去校验密码、执行命令，setuid 是让普通用户进入这个流程的标准做法（sudo 自己也是这么装的）。如果只打算给 root 自己用，不设 setuid 直接以 root 调用也行，只是会跳过密码输入。

## 首次使用：设置 root 密码（必读）

rtdo 认证的是 **root 密码**，不是你自己账户的密码。**如果 root 账户没有密码，rtdo 无法工作**——比如 WSL 里 root 默认没有密码、部分发行版默认锁定 root（`/etc/shadow` 中 root 行以 `!` 或 `*` 开头）。

手动设置 root 密码（二选一）：

```bash
# 普通用户执行（sudo 会先验证你的用户密码，再让你设置新的 root 密码）
sudo passwd

# 或先切到 root 用户，再执行 passwd（无参数即修改 root 自己的密码）
su -c passwd
```

**首次运行自动引导**：当检测到 root 密码未设置时，非 root 用户第一次运行 `rtdo <命令>` 会自动进入**强制设置流程**（等价于自动执行 `sudo passwd`），无法跳过：

1. 验证你当前用户的密码（确认操作者身份，最多 3 次机会）；
2. 输入两次新的 root 密码（终端无回显 / 图形弹窗）；
3. 强制校验：新 root 密码**不能与你当前用户的密码相同**，且不能为空、不能过短（至少 6 个字符）、不能等于用户名；
4. 以 root 权限写入新密码，然后继续用刚设置的 root 密码完成本次认证；
5. **提示是否禁用 sudo**：询问是否设置全局别名 `sudo=rtdo`（见下节「禁用 sudo（可选）」）。

> 注意：在既没有终端也没有图形桌面的环境（如 cron 调用）中无法自动引导，rtdo 会拒绝执行并提示你手动运行 `sudo passwd`。

## 禁用 sudo（可选）

设置 root 密码后，rtdo 会询问你是否**禁用 sudo**：即写入全局别名文件 `/etc/profile.d/rtdo-alias.sh`，把所有 `sudo` 命令改为由 rtdo 提权（新终端生效）：

```sh
# /etc/profile.d/rtdo-alias.sh（由 rtdo 生成）
if command -v rtdo >/dev/null 2>&1; then
    alias sudo='rtdo'
fi
```

- 启用后，你在终端里敲 `sudo <命令>` 实际走的是 rtdo 的策略判定与确认流程（而不是 sudo 的全放行）；
- 需要**恢复 sudo**：删除 `/etc/profile.d/rtdo-alias.sh` 并重新登录（或在新终端执行 `unalias sudo`）；
- 也可以在引导时选择「不」，之后随时手动创建该文件启用；
- 该别名只对交互式 shell 生效，不影响脚本、cron 等非交互环境。

## 多语言

rtdo 的所有用户可见输出（帮助、提示、交互、错误）都支持**中英双语**，按以下规则自动选择（优先级从高到低）：

| 变量 | 取值 | 语言 |
|---|---|---|
| `RTDO_LANG` | `zh*` | 中文 |
| `RTDO_LANG` | 其他 | English |
| `LC_ALL` / `LC_MESSAGES` / `LANG` | 以 `zh` 开头 | 中文 |
| `LC_ALL` / `LC_MESSAGES` / `LANG` | 其他（如 `en_US.UTF-8`、`C`） | English |

示例：

```bash
LANG=zh_CN.UTF-8 rtdo --help      # 中文
RTDO_LANG=en rtdo --help          # English
```

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
| `--config <路径>` | 指定配置文件，默认 `/etc/rtdo/rtdo.conf`；也可以用环境变量 `RTDO_CONF` |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

## 交互流程

| 环境 | 交互方式 |
|---|---|
| 图形桌面（Wayland/X11） | 弹窗显示操作摘要，等待点击「同意/拒绝」；需要 zenity、kdialog 或 yad，按顺序自动选择 |
| 终端会话 | 终端内输出操作摘要，等待输入 y / n（直接回车等于拒绝） |
| 两者都没有 | 无法确认，默认拒绝，记入审计日志 |

图形桌面下输入 root 密码也用弹窗（对应工具的密码框），终端下密码输入不回显。

## 配置文件

默认位置 `/etc/rtdo/rtdo.conf`，TOML 格式：

```toml
# 信任目录：命令作用到这些目录内时自动放行，不弹窗
trusted_paths = [
    "/etc/nixos",
    "/home/*/.config",
    "/usr/local/bin"
]

# 高危命令：任何情况下都会拦截，除非使用 --force
blacklist_commands = [
    "dd",
    "mkfs",
    "rm -rf /",
    "chmod -R 777 /"
]

# 审计日志路径
audit_log = "/var/log/rtdo.log"

# 日志格式: "json" 或 "text"
log_format = "json"
```

字段说明：

- `trusted_paths`：按目录前缀匹配。命令涉及的所有路径（从参数里提取）都落在这些目录内时自动放行。支持通配符：`*` 匹配单层路径段内的任意字符，`**` 匹配任意层级，`?` 匹配单个字符。
- `blacklist_commands`：单命令名按 basename 精确匹配（`dd` 命中任意位置调用的 dd）；带参数的命令按前缀匹配（`rm -rf /` 会命中 `rm -rf /tmp` 这类写法）。命中后直接拦截，`--force` 可放行。
- `audit_log`：审计日志路径。
- `log_format`：`json` 或 `text`。

路径提取是启发式的：以 `/`、`.`、`~` 开头或包含 `/` 的参数视为路径（`--foo=/path` 这种选项值也会提取）。提取不到路径的命令（比如 `systemctl status`）一律走确认流程——拿不准就问你，这是有意为之，宁可多问一次也不悄悄放行。

## 审计日志

所有提权请求都会记录，包括被拦截和放行的。每条记录包含：

- 执行时间
- 操作用户（用户名和 UID）
- 完整命令
- 是否使用 `--force`
- 最终结果（allow / deny / blocked）与原因（trusted_path、user_confirmed、user_denied、blacklist、force、auth_failed、root_password_setup、root_password_setup_failed 等）
- 放行命令的退出码

JSON 格式（默认）：

```json
{"time":"2026-08-28T19:00:00+08:00","user":"alice","uid":1000,"command":"nixos-rebuild switch","force":false,"result":"allow","reason":"user_confirmed","exit_code":0}
```

text 格式：

```
[2026-08-28T19:00:00+08:00] user=alice uid=1000 force=false result=blocked reason=blacklist cmd="dd if=/dev/zero of=/dev/sda"
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
- 环境清理：以安全 PATH 运行，HOME/USER/LOGNAME 设为 root，移除 LD_PRELOAD / LD_LIBRARY_PATH；
- 不含路径的命令会在安全 PATH 里解析成绝对路径再执行，防 PATH 劫持；
- 非交互环境默认拒绝；
- 黑名单是硬拦截，`--force` 是唯一的例外，用之前想清楚。

## 已知限制

- root 账户被锁定或未设置密码（`/etc/shadow` 密码字段以 `!`/`*` 开头或为空）时，首次运行会自动引导设置（等价于 `sudo passwd`）；若处于无交互环境（无终端、无图形桌面）则会被拒绝并提示手动执行 `sudo passwd`；
- 图形弹窗依赖 zenity / kdialog / yad，最小化安装的系统需要自己装一个；
- 路径提取是启发式的，复杂参数（通配符、重定向）可能识别不全，识别不了就进确认流程，不会悄悄放行；
- 只支持 Linux（PAM 是 Linux 的东西）。

## 构建

```bash
cargo build --release
```

产物在 `target/release/rtdo`。

## 测试

简单冒烟：`./rtdo --help`、`./rtdo --version`，`sudo ./rtdo --init` 之后 `./rtdo --policy`。

完整流程需要：setuid 安装 + 给 root 设密码（WSL 里 root 默认没有密码，先 `sudo passwd root`；也可以直接运行 `./rtdo <任意命令>`，首次运行引导会自动设置）+ 一个终端或图形桌面。黑名单拦截可以直接用 `rtdo dd if=/dev/zero of=/dev/null bs=1 count=1` 试，应当直接报拦截。

## 贡献

欢迎提 issue 和 PR。代码结构：

- `src/main.rs` — 参数解析与主流程
- `src/config.rs` — 配置加载与默认策略生成
- `src/policy.rs` — 策略判定（黑名单 / 信任目录 / 路径提取）
- `src/pam.rs` — PAM 认证（校验 root 密码）
- `src/auth.rs` — 密码输入（终端无回显 / 图形密码框）
- `src/interact.rs` — 环境自适应交互（终端 / 弹窗 / 拦截通知）
- `src/audit.rs` — 审计日志（JSON / text）
- `src/exec.rs` — 权限检查与安全执行

## 许可证与贡献者

MIT License，Copyright (c) 2026 Reimilia617

贡献者：

- **Reimilia617** — 项目发起、需求与产品设计
- **DeepSeek（AI 助手）** — 代码实现、架构设计与文档协作

这个项目是人和 AI 结对完成的：方向与需求由 Reimilia617 定，实现细节一起推敲。发现 bug 或有更好的想法，欢迎直接提 issue。

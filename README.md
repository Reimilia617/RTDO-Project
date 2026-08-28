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
3. **认证**：普通用户调用时，要求输入 **root 密码**（PAM 校验，不是你自己账户的密码），错误最多重试 3 次；root 直接调用则跳过。
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

为什么要 setuid：rtdo 需要以 root 身份去校验密码、执行命令，setuid 是让普通用户进入这个流程的标准做法（sudo 自己也是这么装的）。如果只打算给 root 自己用，不设 setuid 直接以 root 调用也行，只是会跳过密码输入。

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
- 最终结果（allow / deny / blocked）与原因（trusted_path、user_confirmed、user_denied、blacklist、force、auth_failed 等）
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

- root 账户被锁定（/etc/shadow 密码字段以 `!` 开头）时 PAM 校验会失败，rtdo 不可用——这是预期行为；
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

完整流程需要：setuid 安装 + 给 root 设密码（WSL 里 root 默认没有密码，先 `sudo passwd root`）+ 一个终端或图形桌面。黑名单拦截可以直接用 `rtdo dd if=/dev/zero of=/dev/null bs=1 count=1` 试，应当直接报拦截。

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

//! 首次运行引导：sudo 前置检查、root 密码检测与强制设置、禁用 sudo（全局别名）。
//!
//! 背景：rtdo 认证的是 **root 密码**。很多环境（WSL 默认无密码、部分发行版
//! 默认锁定 root）root 没有可用密码，此时 rtdo 无法工作。本模块在首次运行
//! 时自动检测并强制走一遍等价于 `sudo passwd` 的设置流程：
//!
//! 1. 验证当前用户（调用者）的密码，确认操作者身份（等价于 sudo 的认证步骤）；
//! 2. 让用户输入两次新的 root 密码；
//! 3. 强制约束：新 root 密码不能与当前用户密码相同（也不能为空、过短或等于 root）；
//! 4. 以 root 权限写入新密码（PAM `pam_chauthtok`，失败时回退 `chpasswd`）；
//! 5. 询问是否禁用 sudo（设置全局别名 `sudo=rtdo`）。
//!
//! 另外提供安装前置检查 `check_sudo_ready()`：sudo 缺失或未正确配置时给出提示。

use std::io::Write;
use std::process::{Command, Stdio};

/// sudo→rtdo 全局别名文件（/etc/profile.d 对登录 shell 全局生效）。
pub const SUDO_ALIAS_PATH: &str = "/etc/profile.d/rtdo-alias.sh";

// ---------------------------------------------------------------------------
// sudo 前置检查
// ---------------------------------------------------------------------------

/// sudo 是否可用（在安全 PATH 中查找 sudo 二进制）。
pub fn sudo_available() -> bool {
    crate::exec::find_in_safe_path("sudo").is_some()
}

/// 当前用户是否在 sudo/wheel 组（读 /etc/group）。
fn user_in_sudo_group(username: &str) -> bool {
    let content = match std::fs::read_to_string("/etc/group") {
        Ok(c) => c,
        Err(_) => return false,
    };
    for line in content.lines() {
        let mut parts = line.split(':');
        let gname = match parts.next() {
            Some(g) => g,
            None => continue,
        };
        if gname != "sudo" && gname != "wheel" {
            continue;
        }
        parts.next(); // passwd 字段
        parts.next(); // gid 字段
        let members = parts.next().unwrap_or("");
        if members.split(',').any(|m| m.trim() == username) {
            return true;
        }
    }
    false
}

/// 检查 sudo 是否已安装并正确配置，返回需要展示的提示列表（双语，已格式化）。
///
/// 安装 rtdo 前应先装好并配置 sudo；该检查在 `--init` 与首次运行引导时调用。
pub fn check_sudo_ready() -> Vec<String> {
    let mut hints = Vec::new();
    let user = crate::exec::current_user();
    if !sudo_available() {
        hints.push(crate::t!(
            "警告: 未检测到 sudo。rtdo 的安装与运维（如手动 `sudo passwd`）依赖 sudo，\
             请先安装并正确配置 sudo，再安装 rtdo。",
            "warning: sudo not found. rtdo installation and operations (e.g. manual \
             `sudo passwd`) rely on sudo; please install and configure sudo first, \
             then install rtdo."
        ));
        hints.push(crate::t!(
            "  Debian/Ubuntu: apt install sudo && usermod -aG sudo <用户名>",
            "  Debian/Ubuntu: apt install sudo && usermod -aG sudo <user>"
        ));
        hints.push(crate::t!(
            "  Fedora/RHEL: dnf install sudo && usermod -aG wheel <用户名>",
            "  Fedora/RHEL: dnf install sudo && usermod -aG wheel <user>"
        ));
        hints.push(crate::t!(
            "  Arch: pacman -S sudo && usermod -aG wheel <用户名>",
            "  Arch: pacman -S sudo && usermod -aG wheel <user>"
        ));
        hints.push(crate::t!(
            "  安装后需重新登录使组权限生效。",
            "  Log out and back in for the group change to take effect."
        ));
    } else if user != "root" && !user_in_sudo_group(&user) {
        hints.push(crate::t!(
            "提示: sudo 已安装，但当前用户 {} 不在 sudo/wheel 组，无法使用 sudo。\
             请以 root 执行 `usermod -aG sudo {}`（或 wheel）后重新登录。",
            "note: sudo is installed, but the current user {} is not in the \
             sudo/wheel group and cannot use sudo. Run `usermod -aG sudo {}` \
             (or wheel) as root, then log back in.",
            user,
            user
        ));
    } else {
        hints.push(crate::t!(
            "sudo 已安装并正确配置。",
            "sudo is installed and configured correctly."
        ));
    }
    hints
}

// ---------------------------------------------------------------------------
// root 密码检测与引导设置
// ---------------------------------------------------------------------------

/// root 密码是否已设置（读取 /etc/shadow）。
///
/// 判定规则：root 行的密码字段为空、以 `!` 开头（锁定）或以 `*` 开头（禁用）
/// 都视为“未设置”。读取失败时保守地视为“未设置”，由引导流程给出明确错误。
pub fn root_password_set() -> bool {
    let content = match std::fs::read_to_string("/etc/shadow") {
        Ok(c) => c,
        Err(_) => return false,
    };
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("root:") {
            let hash = rest.split(':').next().unwrap_or("");
            return !(hash.is_empty() || hash.starts_with('!') || hash.starts_with('*'));
        }
    }
    false
}

/// 引导设置 root 密码（等价于自动执行 `sudo passwd`）。
///
/// `current_user` 为调用者用户名（真实 UID 对应用户，绝不可能是 root，
/// 因为 root 直接调用时本流程不会触发）。
///
/// 返回 `Ok(())` 表示 root 密码已成功设置；`Err(msg)` 表示失败原因。
pub fn bootstrap_root_password(current_user: &str) -> Result<(), String> {
    if crate::interact::detect_channel() == crate::interact::Channel::None {
        return Err(crate::t!(
            "root 密码未设置，且当前环境无法交互（非终端且无图形桌面）。\n\
             rtdo: 请手动执行 `sudo passwd`（或在 root 用户下执行 `passwd`）设置 root 密码后重试。",
            "root password is not set, and the environment cannot interact \
             (no terminal and no GUI).\n\
             rtdo: run `sudo passwd` manually (or `passwd` as root) to set the \
             root password, then retry."
        ));
    }

    eprintln!(
        "{}",
        crate::t!(
            "rtdo: 检测到 root 密码未设置，首次运行需要先设置 root 密码。",
            "rtdo: root password is not set; the first run must set it first."
        )
    );
    eprintln!(
        "{}",
        crate::t!(
            "rtdo: 本流程等价于自动执行 `sudo passwd`（先验证你的用户密码，再设置新的 root 密码）。",
            "rtdo: this flow is equivalent to running `sudo passwd` automatically \
             (verify your user password first, then set a new root password)."
        )
    );

    // 第 0 步：sudo 前置提示（sudo 缺失/未配置时提醒先处理好）
    for hint in check_sudo_ready() {
        eprintln!("{}", hint);
    }

    // 第 1 步：验证当前用户密码
    eprintln!(
        "{}",
        crate::t!(
            "rtdo: [1/3] 验证当前用户 {} 的密码…",
            "rtdo: [1/3] verifying password of user {}…",
            current_user
        )
    );
    let user_password = verify_current_user(current_user)?;

    // 第 2 步：读取新的 root 密码（两次输入确认）
    eprintln!(
        "{}",
        crate::t!(
            "rtdo: [2/3] 设置新的 root 密码（两次输入确认）…",
            "rtdo: [2/3] setting a new root password (enter twice)…"
        )
    );
    let (pw, confirm) = read_new_root_password()?;
    if pw != confirm {
        return Err(crate::t!(
            "两次输入的 root 密码不一致，已取消设置。",
            "the two root password entries do not match; setup cancelled."
        ));
    }

    // 第 3 步：约束校验并写入
    eprintln!(
        "{}",
        crate::t!(
            "rtdo: [3/3] 校验并写入 root 密码…",
            "rtdo: [3/3] validating and writing the root password…"
        )
    );
    validate_new_password(&pw, &user_password, current_user)?;
    apply_root_password(&pw)?;

    println!(
        "{}",
        crate::t!("rtdo: root 密码已设置成功。", "rtdo: root password set successfully.")
    );
    Ok(())
}

/// 验证当前用户密码（最多 3 次尝试），成功时返回该密码（供“不能与 root 密码相同”校验）。
fn verify_current_user(user: &str) -> Result<String, String> {
    let mut attempts = 3;
    loop {
        let pw = crate::auth::prompt_password_with_label(&crate::t!(
            "请输入用户 {} 的密码（验证操作者身份）: ",
            "Enter password for user {} (verify identity): ",
            user
        ))
        .map_err(|e| crate::t!("无法获取密码: {}", "cannot read password: {}", e))?;
        match crate::pam::verify_password(user, &pw) {
            Ok(()) => return Ok(pw),
            Err(_) => {
                attempts -= 1;
                if attempts == 0 {
                    return Err(crate::t!(
                        "当前用户 {} 的密码验证失败，已取消设置 root 密码。\n\
                         rtdo: 若你的用户账户没有密码或已被锁定，请先以 root 身份运行 \
                         `passwd {}` 或 `sudo passwd {}` 设置，再重试。",
                        "password verification failed for user {}; root password setup cancelled.\n\
                         rtdo: if your account has no password or is locked, run \
                         `passwd {}` or `sudo passwd {}` as root first, then retry.",
                        user,
                        user,
                        user
                    ));
                }
                eprintln!(
                    "{}",
                    crate::t!(
                        "rtdo: 密码错误（剩余 {} 次机会）",
                        "rtdo: wrong password ({} attempts left)",
                        attempts
                    )
                );
            }
        }
    }
}

/// 让用户输入两次新的 root 密码。
fn read_new_root_password() -> Result<(String, String), String> {
    let pw = crate::auth::prompt_password_with_label(&crate::t!(
        "请输入新的 root 密码: ",
        "Enter new root password: "
    ))
    .map_err(|e| crate::t!("无法获取密码: {}", "cannot read password: {}", e))?;
    let confirm = crate::auth::prompt_password_with_label(&crate::t!(
        "请再次输入新的 root 密码: ",
        "Re-enter new root password: "
    ))
    .map_err(|e| crate::t!("无法获取密码: {}", "cannot read password: {}", e))?;
    Ok((pw, confirm))
}

/// 新 root 密码的强制约束：
/// * 非空；
/// * 长度至少 6 个字符；
/// * 不能包含 `:` 或换行符（避免破坏 shadow/chpasswd 格式）；
/// * 不能等于用户名或 “root”；
/// * **不能与当前用户密码相同**。
fn validate_new_password(pw: &str, user_password: &str, current_user: &str) -> Result<(), String> {
    if pw.is_empty() {
        return Err(crate::t!(
            "root 密码不能为空。",
            "root password must not be empty."
        ));
    }
    if pw.len() < 6 {
        return Err(crate::t!(
            "root 密码长度至少为 6 个字符。",
            "root password must be at least 6 characters."
        ));
    }
    if pw.contains(':') || pw.contains('\n') || pw.contains('\r') {
        return Err(crate::t!(
            "root 密码不能包含 ':' 或换行符。",
            "root password must not contain ':' or newlines."
        ));
    }
    if pw == "root" || pw == current_user {
        return Err(crate::t!(
            "root 密码不能等于用户名。",
            "root password must not equal the username."
        ));
    }
    if pw == user_password {
        return Err(crate::t!(
            "新的 root 密码不能与当前用户 {} 的密码相同。",
            "the new root password must not be the same as the password of user {}.",
            current_user
        ));
    }
    Ok(())
}

/// 以 root 权限写入新的 root 密码（等价于 `sudo passwd root` 的执行步骤）。
///
/// 优先使用 PAM `pam_chauthtok`（进程内完成，不依赖外部工具）；
/// 失败时回退到 `chpasswd`（从 stdin 读取 `root:新密码`）。
fn apply_root_password(new_password: &str) -> Result<(), String> {
    match crate::pam::change_root_password(new_password) {
        Ok(()) => return Ok(()),
        Err(e) => eprintln!(
            "{}",
            crate::t!(
                "rtdo: 警告: PAM 方式设置失败（{}），尝试回退 chpasswd…",
                "rtdo: warning: PAM method failed ({}), falling back to chpasswd…",
                e
            )
        ),
    }
    apply_root_password_chpasswd(new_password)
}

/// 回退方案：通过 `chpasswd` 设置 root 密码。
fn apply_root_password_chpasswd(new_password: &str) -> Result<(), String> {
    let mut child = Command::new("chpasswd")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| crate::t!("无法启动 chpasswd: {}", "cannot start chpasswd: {}", e))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| crate::t!("无法获取 chpasswd 的 stdin", "cannot get chpasswd stdin"))?;
        writeln!(stdin, "root:{}", new_password)
            .map_err(|e| crate::t!("写入 chpasswd 失败: {}", "write to chpasswd failed: {}", e))?;
    }

    let out = child
        .wait_with_output()
        .map_err(|e| crate::t!("chpasswd 执行失败: {}", "chpasswd failed: {}", e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(crate::t!(
            "chpasswd 设置失败: {}",
            "chpasswd failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

// ---------------------------------------------------------------------------
// 禁用 sudo（全局别名 sudo=rtdo）
// ---------------------------------------------------------------------------

/// 是否已安装 sudo→rtdo 全局别名。
pub fn sudo_alias_installed() -> bool {
    std::fs::read_to_string(SUDO_ALIAS_PATH)
        .map(|c| c.contains("alias sudo"))
        .unwrap_or(false)
}

/// 询问是否禁用 sudo：设置全局别名 `sudo=rtdo`（新终端生效）。
///
/// 在首次运行引导设置 root 密码成功后调用；无交互通道时仅打印手动方式提示。
pub fn offer_disable_sudo() -> Result<(), String> {
    if sudo_alias_installed() {
        println!(
            "{}",
            crate::t!(
                "rtdo: 已检测到 sudo→rtdo 全局别名（{}），无需重复设置。",
                "rtdo: sudo→rtdo global alias already installed ({}); nothing to do.",
                SUDO_ALIAS_PATH
            )
        );
        return Ok(());
    }
    if crate::interact::detect_channel() == crate::interact::Channel::None {
        println!(
            "{}",
            crate::t!(
                "rtdo: 当前环境无法交互。如需禁用 sudo（全局别名 sudo=rtdo），可手动创建 {}。",
                "rtdo: no interactive channel. To disable sudo (global alias sudo=rtdo), \
                 create {} manually.",
                SUDO_ALIAS_PATH
            )
        );
        return Ok(());
    }

    let answer = crate::interact::confirm_yes_no(&crate::t!(
        "是否禁用 sudo？将设置全局别名 sudo=rtdo（以后所有 sudo 命令改由 rtdo 提权）[y/N] ",
        "Disable sudo? This sets the global alias sudo=rtdo (all sudo commands will be \
         elevated by rtdo) [y/N] "
    ))?;
    if !answer {
        println!(
            "{}",
            crate::t!(
                "rtdo: 保留 sudo。之后可随时手动创建 {} 来启用 sudo→rtdo 别名。",
                "rtdo: sudo kept. You can create {} manually later to enable the sudo→rtdo alias.",
                SUDO_ALIAS_PATH
            )
        );
        return Ok(());
    }
    install_sudo_alias()
}

/// 写入全局别名文件 `/etc/profile.d/rtdo-alias.sh`。
fn install_sudo_alias() -> Result<(), String> {
    let content = concat!(
        "# Generated by rtdo — disable sudo via global alias (sudo=rtdo)\n",
        "# 由 rtdo 生成 — 通过全局别名禁用 sudo（sudo=rtdo）\n",
        "# To restore sudo: delete this file and log in again, or run `unalias sudo`\n",
        "# 恢复 sudo：删除本文件并重新登录，或执行 `unalias sudo`\n",
        "if command -v rtdo >/dev/null 2>&1; then\n",
        "    alias sudo='rtdo'\n",
        "fi\n",
    );
    std::fs::write(SUDO_ALIAS_PATH, content).map_err(|e| {
        crate::t!(
            "无法写入 {}: {}",
            "cannot write {}: {}",
            SUDO_ALIAS_PATH,
            e
        )
    })?;
    println!(
        "{}",
        crate::t!(
            "rtdo: 已设置全局别名 sudo=rtdo（{}）。新终端生效；恢复 sudo 请删除该文件并重新登录。",
            "rtdo: global alias sudo=rtdo installed ({}). Takes effect in new shells; \
             delete the file and log in again to restore sudo.",
            SUDO_ALIAS_PATH
        )
    );
    Ok(())
}

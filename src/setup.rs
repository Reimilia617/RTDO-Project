//! 首次运行/每次运行引导：sudo 前置检查、root 密码检测与强制设置、禁用 sudo（守护进程拦截）。
//!
//! 背景：rtdo 认证的是 **root 密码**。很多环境（WSL 默认无密码、部分发行版
//! 默认锁定 root）root 没有可用密码，此时 rtdo 无法工作。本模块在运行时
//! 自动检测并强制走一遍等价于 `sudo passwd` 的设置流程：
//!
//! 1. 验证当前用户（调用者）的密码，确认操作者身份（等价于 sudo 的认证步骤）；
//! 2. 让用户输入两次新的 root 密码；
//! 3. 强制约束：新 root 密码不能等于系统内任意用户名或任意用户的密码
//!    （也不能为空、过短、含 `:`/换行）；
//! 4. 以 root 权限写入新密码（PAM `pam_chauthtok`，失败时回退 `chpasswd`）；
//! 5. 询问是否禁用 sudo：安装 Go 守护进程（rtdo-sudod，systemd 管理）拦截 sudo，
//!    原版 sudo 移动为 `/usr/bin/sudo.real`，可用 `sudo-force` 调用。
//!
//! 每次运行时（非 root 调用）都会检查：
//! * root 密码缺失或过弱（等于任意用户名/任意用户密码）→ 强制重新设置；
//! * sudo 未安装 → 提示先安装并配置 sudo；
//! * 尚未禁用 sudo → 询问是否启用守护进程拦截（已启用则跳过）。

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// 路径常量
// ---------------------------------------------------------------------------

/// sudo 拦截守护进程二进制（由 rtdo.sh --install 或 rtdo 安装）。
pub const DAEMON_BIN_PATH: &str = "/usr/local/lib/rtdo/rtdo-sudod";
/// 原版 sudo 备份位置（被拦截后移到这里）。
pub const REAL_SUDO_PATH: &str = "/usr/bin/sudo.real";
/// sudo shim（拦截入口）。
pub const SHIM_SUDO_PATH: &str = "/usr/bin/sudo";
/// sudo-force shim（调用原版 sudo 的入口）。
pub const SHIM_SUDO_FORCE_PATH: &str = "/usr/bin/sudo-force";
/// systemd 服务单元文件。
pub const SYSTEMD_UNIT_PATH: &str = "/etc/systemd/system/rtdo-sudod.service";
/// 标记文件：存在即表示已启用 sudo 拦截。
pub const SUDO_DISABLED_MARKER: &str = "/etc/rtdo/sudo-disabled";
/// 旧版本遗留的全局别名文件（不再使用，安装拦截时清理）。
pub const LEGACY_ALIAS_PATH: &str = "/etc/profile.d/rtdo-alias.sh";

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
/// 安装 rtdo 前应先装好并配置 sudo；该检查在 `--init`、首次引导与每次运行时调用。
pub fn check_sudo_ready() -> Vec<String> {
    let mut hints = Vec::new();
    let user = crate::exec::current_user();
    if !sudo_available() {
        hints.push(crate::t!(
            "警告: 未检测到 sudo。rtdo 的安装与运维（如手动 `sudo passwd`）依赖 sudo，\
             请先安装并正确配置 sudo，再进行操作。",
            "warning: sudo not found. rtdo installation and operations (e.g. manual \
             `sudo passwd`) rely on sudo; please install and configure sudo first, \
             then proceed."
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
// 系统用户与 root 密码检测
// ---------------------------------------------------------------------------

/// 读取系统内所有用户名（/etc/passwd 第一字段）。
fn system_usernames() -> Vec<String> {
    let content = match std::fs::read_to_string("/etc/passwd") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| l.split(':').next())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

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

/// root 密码是否“过弱”：等于系统内任意一个用户名，或等于任意一个用户的密码。
///
/// 用途：认证通过后若发现 root 密码过弱，强制重新设置（等价于再次跳出设置界面）。
pub fn root_password_weak(pw: &str) -> bool {
    let usernames = system_usernames();
    if usernames.iter().any(|u| u == pw) {
        return true;
    }
    for u in &usernames {
        if u == "root" {
            continue;
        }
        if crate::pam::verify_password(u, pw).is_ok() {
            return true;
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
            "root 密码未设置或过弱，且当前环境无法交互（非终端且无图形桌面）。\n\
             rtdo: 请手动执行 `sudo passwd`（或在 root 用户下执行 `passwd`）设置 root 密码后重试。",
            "root password is missing or weak, and the environment cannot interact \
             (no terminal and no GUI).\n\
             rtdo: run `sudo passwd` manually (or `passwd` as root) to set the \
             root password, then retry."
        ));
    }

    eprintln!(
        "{}",
        crate::t!(
            "rtdo: 检测到 root 密码未设置或过弱，需要重新设置 root 密码。",
            "rtdo: the root password is missing or weak; it must be reset."
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
    validate_new_password(&pw, &user_password)?;
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
/// * **不能等于系统内任意一个用户名**；
/// * **不能与当前用户密码相同**。
fn validate_new_password(pw: &str, user_password: &str) -> Result<(), String> {
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
    if system_usernames().iter().any(|u| u == pw) {
        return Err(crate::t!(
            "root 密码不能等于系统内任意一个用户名。",
            "the root password must not equal any system username."
        ));
    }
    if pw == user_password {
        return Err(crate::t!(
            "新的 root 密码不能与当前用户的密码相同。",
            "the new root password must not be the same as the current user's password."
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
// 禁用 sudo（Go 守护进程拦截 + systemd）
// ---------------------------------------------------------------------------

/// 是否已启用 sudo 拦截（标记文件存在）。
pub fn sudo_interception_installed() -> bool {
    Path::new(SUDO_DISABLED_MARKER).exists()
}

/// 询问是否禁用 sudo：安装 Go 守护进程并拦截 sudo（原版 sudo 移动为 sudo.real，
/// 可用 `sudo-force` 调用）。
///
/// 每次运行（非 root 调用）都会检查；已启用则跳过。
pub fn offer_disable_sudo() -> Result<(), String> {
    if sudo_interception_installed() {
        return Ok(());
    }
    if crate::interact::detect_channel() == crate::interact::Channel::None {
        println!(
            "{}",
            crate::t!(
                "rtdo: 当前环境无法交互。如需禁用 sudo（守护进程拦截），请以 root 运行 \
                 `curl -fsSL https://raw.githubusercontent.com/Reimilia617/RTDO-Project/main/rtdo.sh | sh -s -- --install`。",
                "rtdo: no interactive channel. To disable sudo (daemon interception), run \
                 `curl -fsSL https://raw.githubusercontent.com/Reimilia617/RTDO-Project/main/rtdo.sh | sh -s -- --install` as root."
            )
        );
        return Ok(());
    }

    let answer = crate::interact::confirm_yes_no(&crate::t!(
        "是否禁用 sudo？将安装守护进程拦截 sudo（原版 sudo 移动为 sudo.real，\
         确需使用请用 sudo-force；systemd 服务 rtdo-sudod 管理）[y/N] ",
        "Disable sudo? This installs a daemon that intercepts sudo (the original sudo \
         moves to sudo.real; use sudo-force if you really need it; managed by the \
         rtdo-sudod systemd service) [y/N] "
    ))?;
    if !answer {
        println!(
            "{}",
            crate::t!(
                "rtdo: 保留 sudo。之后可随时以 root 运行 rtdo.sh --install 启用拦截。",
                "rtdo: sudo kept. You can run `rtdo.sh --install` as root later to enable interception."
            )
        );
        return Ok(());
    }
    install_sudo_interception()
}

/// 安装 sudo 拦截：
/// 1. 原版 sudo 移动为 `/usr/bin/sudo.real`；
/// 2. 守护进程二进制复制为 `/usr/bin/sudo`（shim，被拦截）与 `/usr/bin/sudo-force`；
/// 3. 写入 systemd 单元并启用启动服务；
/// 4. 写入标记文件；清理旧版别名文件。
pub fn install_sudo_interception() -> Result<(), String> {
    if !crate::exec::is_root() {
        return Err(crate::t!(
            "需要 root 权限以安装 sudo 拦截。",
            "root permission needed to install the sudo interception."
        ));
    }
    if !Path::new(DAEMON_BIN_PATH).is_file() {
        return Err(crate::t!(
            "未找到守护进程二进制 {}。请先通过 rtdo.sh --install 安装（会编译并放置 rtdo-sudod）。",
            "daemon binary {} not found. Please install via `rtdo.sh --install` first \
             (it builds and places rtdo-sudod).",
            DAEMON_BIN_PATH
        ));
    }

    // 1. 备份原版 sudo
    if Path::new(SHIM_SUDO_PATH).exists() && !Path::new(REAL_SUDO_PATH).exists() {
        std::fs::rename(SHIM_SUDO_PATH, REAL_SUDO_PATH).map_err(|e| {
            crate::t!(
                "无法移动 {} 到 {}: {}",
                "cannot move {} to {}: {}",
                SHIM_SUDO_PATH,
                REAL_SUDO_PATH,
                e
            )
        })?;
    }

    // 2. 安装 shim（守护进程二进制以不同 argv[0] 扮演 sudo / sudo-force）
    for p in [SHIM_SUDO_PATH, SHIM_SUDO_FORCE_PATH] {
        std::fs::copy(DAEMON_BIN_PATH, p).map_err(|e| {
            crate::t!("无法安装 {}: {}", "cannot install {}: {}", p, e)
        })?;
        let _ = std::fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(0o755));
    }

    // 3. systemd 单元 + 启用服务
    std::fs::write(SYSTEMD_UNIT_PATH, systemd_unit_content()).map_err(|e| {
        crate::t!(
            "无法写入 {}: {}",
            "cannot write {}: {}",
            SYSTEMD_UNIT_PATH,
            e
        )
    })?;
    run_systemctl(&["daemon-reload"])?;
    if let Err(e) = run_systemctl(&["enable", "--now", "rtdo-sudod.service"]) {
        // 非 systemd 环境（如容器）不阻塞，但给出警告
        eprintln!(
            "{}",
            crate::t!(
                "rtdo: 警告: 无法启用 systemd 服务（{}）。请手动执行 systemctl enable --now rtdo-sudod。",
                "rtdo: warning: cannot enable systemd service ({}). Run \
                 `systemctl enable --now rtdo-sudod` manually.",
                e
            )
        );
    }

    // 4. 标记 + 清理旧别名
    if let Some(dir) = Path::new(SUDO_DISABLED_MARKER).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(SUDO_DISABLED_MARKER, "rtdo sudo interception enabled\n").map_err(|e| {
        crate::t!(
            "无法写入标记文件 {}: {}",
            "cannot write marker {}: {}",
            SUDO_DISABLED_MARKER,
            e
        )
    })?;
    let _ = std::fs::remove_file(LEGACY_ALIAS_PATH);

    println!(
        "{}",
        crate::t!(
            "rtdo: 已启用 sudo 拦截。`sudo` 现在会被守护进程拦截并提示改用 rtdo；\
             确需原版 sudo 请使用 `sudo-force <命令>`（systemd 服务: rtdo-sudod）。",
            "rtdo: sudo interception enabled. `sudo` is now intercepted by the daemon \
             (use rtdo instead); for the original sudo, use `sudo-force <cmd>` \
             (systemd service: rtdo-sudod)."
        )
    );
    Ok(())
}

/// systemd 单元内容。
fn systemd_unit_content() -> String {
    format!(
        "[Unit]\n\
         Description=rtdo sudo interception daemon (rtdo-sudod)\n\
         Documentation=https://github.com/Reimilia617/RTDO-Project\n\
         After=local-fs.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} serve\n\
         RuntimeDirectory=rtdo\n\
         RuntimeDirectoryMode=0755\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        DAEMON_BIN_PATH
    )
}

/// 执行 systemctl 命令（静默失败时返回 Err）。
fn run_systemctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("systemctl: {}", e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

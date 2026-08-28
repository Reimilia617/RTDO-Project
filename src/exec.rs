//! 权限检查与以 root 身份执行命令。

use std::process::{Command, ExitStatus};

/// 进程当前有效 UID 是否为 root（setuid root 安装时为 0）。
pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// 真实 UID（调用者身份；setuid 时与有效 UID 不同）。
pub fn real_uid() -> u32 {
    unsafe { libc::getuid() }
}

/// 当前用户（真实 UID）的用户名，取自 /etc/passwd（getpwuid_r）。
pub fn current_user() -> String {
    let uid = real_uid();
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let mut buf = vec![0u8 as libc::c_char; 16384];
        let rc = libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result);
        if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
            let name = std::ffi::CStr::from_ptr(pwd.pw_name)
                .to_string_lossy()
                .into_owned();
            if !name.is_empty() {
                return name;
            }
        }
    }
    format!("uid-{}", uid)
}

/// 以 root 运行命令时使用的安全 PATH。
const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// 解析命令：含 `/` 的直接使用；否则在安全 PATH 中查找绝对路径（防止 PATH 劫持）。
fn resolve_command(cmd: &str) -> String {
    if cmd.contains('/') {
        return cmd.to_string();
    }
    for dir in SAFE_PATH.split(':') {
        let p = std::path::Path::new(dir).join(cmd);
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    cmd.to_string()
}

/// 以 root 身份执行命令，返回子进程退出码。
///
/// 安全措施：
/// * 补全真实身份为 root（setuid 下 setuid(0)/setgid(0)）；
/// * 使用安全 PATH 解析命令绝对路径，不经过 shell 解释；
/// * 清理环境：HOME=/root、USER/LOGNAME=root，移除 LD_PRELOAD / LD_LIBRARY_PATH。
pub fn run_as_root(argv: &[String]) -> i32 {
    unsafe {
        libc::setgid(0);
        libc::setuid(0);
    }

    let resolved = resolve_command(&argv[0]);
    let mut cmd = Command::new(&resolved);
    cmd.args(&argv[1..]);
    cmd.env("PATH", SAFE_PATH)
        .env("HOME", "/root")
        .env("USER", "root")
        .env("LOGNAME", "root")
        .env_remove("LD_PRELOAD")
        .env_remove("LD_LIBRARY_PATH");

    match cmd.status() {
        Ok(status) => propagate(status),
        Err(e) => {
            eprintln!("rtdo: 无法执行 {}: {}", resolved, e);
            1
        }
    }
}

/// 将子进程退出状态转为退出码（信号终止返回 128 + 信号编号）。
fn propagate(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            eprintln!("rtdo: 命令被信号 {} 终止", sig);
            return 128 + sig;
        }
    }
    1
}

//! 环境自适应交互：
//! * 图形桌面（Wayland/X11）：弹窗显示操作摘要，等待用户点击“同意/拒绝”。
//! * 终端会话：终端内输出操作摘要，等待用户输入 y / n。
//! * 两者皆无：无法交互（调用方决定默认拒绝）。

use std::io::{IsTerminal, Write};
use std::process::Command;

/// 交互通道。
#[derive(Debug, PartialEq, Eq)]
pub enum Channel {
    /// 终端会话（/dev/tty 可用或 stdin 是终端）。
    Terminal,
    /// 图形桌面（有显示环境且存在 zenity/kdialog/yad）。
    Gui,
    /// 无可用的交互通道。
    None,
}

/// 确认结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Confirm {
    Yes,
    No,
}

/// 是否处于图形桌面环境（Wayland / X11）。
pub fn has_display() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
}

/// 在 PATH 中查找可用的对话框工具。
pub fn find_tool(names: &[&str]) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in names {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// /dev/tty 是否可用（存在控制终端）。
pub fn tty_available() -> bool {
    let c = match std::ffi::CString::new("/dev/tty") {
        Ok(c) => c,
        Err(_) => return false,
    };
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR) };
    if fd >= 0 {
        unsafe { libc::close(fd) };
        true
    } else {
        false
    }
}

/// 检测当前交互通道。
pub fn detect_channel() -> Channel {
    if std::io::stdin().is_terminal() || tty_available() {
        Channel::Terminal
    } else if has_display() && find_tool(&["zenity", "kdialog", "yad"]).is_some() {
        Channel::Gui
    } else {
        Channel::None
    }
}

/// 请求用户确认（终端 y/N 或 弹窗 同意/拒绝）。
pub fn confirm(cmd_display: &str, args_display: &str) -> Result<Confirm, String> {
    match detect_channel() {
        Channel::Terminal => confirm_terminal(cmd_display, args_display),
        Channel::Gui => confirm_gui(cmd_display, args_display),
        Channel::None => Err(crate::t!(
            "无法交互：既非终端也无可用的图形对话框，已默认拒绝（可使用 --force 跳过确认）",
            "cannot interact: no terminal or GUI dialog available; denied by default (use --force to skip confirmation)"
        )),
    }
}

/// 终端内确认（通用 y/N 读取，prompt 由调用方提供）。
pub fn confirm_yes_no(prompt: &str) -> Result<bool, String> {
    if !tty_available() && !std::io::stdin().is_terminal() {
        return Err(crate::t!(
            "无法交互：既非终端也无可用的图形对话框",
            "cannot interact: no terminal or GUI dialog available"
        ));
    }
    let mut err = std::io::stderr();
    let _ = write!(err, "{}", prompt);
    let _ = err.flush();
    let line = read_line_interactive();
    let t = line.trim().to_lowercase();
    Ok(t == "y" || t == "yes")
}

/// 终端内确认。
fn confirm_terminal(cmd_display: &str, args_display: &str) -> Result<Confirm, String> {
    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "{}",
        crate::t!("rtdo: 请求以 root 身份执行命令", "rtdo: request to run command as root")
    );
    let _ = writeln!(
        err,
        "{}",
        crate::t!("  命令: {}", "  command: {}", cmd_display)
    );
    if !args_display.is_empty() {
        let _ = writeln!(
            err,
            "{}",
            crate::t!("  参数: {}", "  args: {}", args_display)
        );
    }
    let _ = writeln!(
        err,
        "{}",
        crate::t!("  用户: {}", "  user: {}", crate::exec::current_user())
    );
    let _ = write!(
        err,
        "{}",
        crate::t!("允许执行？[y/N] ", "Allow? [y/N] ")
    );
    let _ = err.flush();

    let line = read_line_interactive();
    let t = line.trim().to_lowercase();
    Ok(if t == "y" || t == "yes" {
        Confirm::Yes
    } else {
        Confirm::No
    })
}

/// 图形弹窗确认（zenity / kdialog / yad）。
fn confirm_gui(cmd_display: &str, args_display: &str) -> Result<Confirm, String> {
    let mut text = format!(
        "{}\n\n{}: {}\n",
        crate::t!(
            "rtdo 请求以 root 身份执行:",
            "rtdo requests to run as root:"
        ),
        crate::t!("命令", "command"),
        cmd_display
    );
    if !args_display.is_empty() {
        text.push_str(&format!(
            "{}: {}\n",
            crate::t!("参数", "args"),
            args_display
        ));
    }
    text.push_str(&format!(
        "\n{}: {}\n\n{}",
        crate::t!("用户", "user"),
        crate::exec::current_user(),
        crate::t!("是否允许？", "Allow?")
    ));

    if let Some(z) = find_tool(&["zenity"]) {
        let st = Command::new(z)
            .arg("--question")
            .arg("--title")
            .arg(crate::t!("rtdo — 提权确认", "rtdo — privilege confirmation"))
            .arg("--text")
            .arg(&text)
            .arg("--ok-label")
            .arg(crate::t!("同意", "Allow"))
            .arg("--cancel-label")
            .arg(crate::t!("拒绝", "Deny"))
            .status();
        if let Ok(s) = st {
            return Ok(if s.success() { Confirm::Yes } else { Confirm::No });
        }
    }
    if let Some(k) = find_tool(&["kdialog"]) {
        let st = Command::new(k)
            .arg("--yesno")
            .arg(&text)
            .arg("--title")
            .arg(crate::t!("rtdo — 提权确认", "rtdo — privilege confirmation"))
            .status();
        if let Ok(s) = st {
            return Ok(if s.success() { Confirm::Yes } else { Confirm::No });
        }
    }
    if let Some(y) = find_tool(&["yad"]) {
        let st = Command::new(y)
            .arg("--question")
            .arg("--title")
            .arg(crate::t!("rtdo — 提权确认", "rtdo — privilege confirmation"))
            .arg("--text")
            .arg(&text)
            .arg("--button")
            .arg(format!("{}:0", crate::t!("同意", "Allow")))
            .arg("--button")
            .arg(format!("{}:1", crate::t!("拒绝", "Deny")))
            .status();
        if let Ok(s) = st {
            return Ok(if s.success() { Confirm::Yes } else { Confirm::No });
        }
    }
    Err(crate::t!(
        "图形环境未检测到可用的对话框工具 (zenity/kdialog/yad)",
        "no dialog tool found in GUI environment (zenity/kdialog/yad)"
    ))
}

/// 通知用户命令已被拦截（黑名单）。
/// GUI 环境弹信息窗，终端环境打印文本，两者皆无时打印到 stderr。
pub fn notify_blocked(cmd_display: &str, args_display: &str, reason: &str) {
    match detect_channel() {
        Channel::Terminal => {
            eprintln!(
                "{}",
                crate::t!("rtdo: 已拦截命令", "rtdo: command blocked")
            );
            eprintln!(
                "{}",
                crate::t!("  命令: {}", "  command: {}", cmd_display)
            );
            if !args_display.is_empty() {
                eprintln!(
                    "{}",
                    crate::t!("  参数: {}", "  args: {}", args_display)
                );
            }
            eprintln!(
                "{}",
                crate::t!("  原因: {}", "  reason: {}", reason)
            );
            eprintln!(
                "{}",
                crate::t!(
                    "提示: 如确需执行，请使用 `rtdo --force ...`（风险自负）",
                    "hint: if you really need to run it, use `rtdo --force ...` (at your own risk)"
                )
            );
        }
        Channel::Gui => {
            let mut text = format!(
                "{}\n\n{}: {}\n",
                crate::t!("rtdo 已拦截高危命令:", "rtdo blocked a dangerous command:"),
                crate::t!("命令", "command"),
                cmd_display
            );
            if !args_display.is_empty() {
                text.push_str(&format!(
                    "{}: {}\n",
                    crate::t!("参数", "args"),
                    args_display
                ));
            }
            text.push_str(&format!(
                "\n{}: {}\n\n{}",
                crate::t!("原因", "reason"),
                reason,
                crate::t!(
                    "如需强制放行请使用 --force。",
                    "use --force to force-run it."
                )
            ));
            if let Some(z) = find_tool(&["zenity"]) {
                let _ = Command::new(z)
                    .arg("--info")
                    .arg("--title")
                    .arg(crate::t!("rtdo — 已拦截", "rtdo — blocked"))
                    .arg("--text")
                    .arg(&text)
                    .status();
            } else if let Some(k) = find_tool(&["kdialog"]) {
                let _ = Command::new(k)
                    .arg("--msgbox")
                    .arg(&text)
                    .arg("--title")
                    .arg(crate::t!("rtdo — 已拦截", "rtdo — blocked"))
                    .status();
            } else if let Some(y) = find_tool(&["yad"]) {
                let _ = Command::new(y)
                    .arg("--info")
                    .arg("--title")
                    .arg(crate::t!("rtdo — 已拦截", "rtdo — blocked"))
                    .arg("--text")
                    .arg(&text)
                    .status();
            }
        }
        Channel::None => {
            eprintln!(
                "{}",
                crate::t!(
                    "rtdo: 已拦截命令 {}（{}）",
                    "rtdo: blocked command {} ({})",
                    cmd_display,
                    reason
                )
            );
        }
    }
}

/// 从 /dev/tty（优先）或 stdin 读取一行。
fn read_line_interactive() -> String {
    let c = match std::ffi::CString::new("/dev/tty") {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR) };
    if fd >= 0 {
        let mut s = String::new();
        let mut buf = [0u8; 256];
        loop {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            let chunk = &buf[..n as usize];
            if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
                s.push_str(&String::from_utf8_lossy(&chunk[..pos]));
                break;
            }
            s.push_str(&String::from_utf8_lossy(chunk));
        }
        unsafe { libc::close(fd) };
        s
    } else {
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
        s
    }
}

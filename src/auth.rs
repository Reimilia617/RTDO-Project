//! 认证：交互式获取 root 密码（终端无回显 / 图形密码框）。

use std::ffi::CString;

/// 交互式获取 root 密码（默认提示文案）。
pub fn prompt_password() -> Result<String, String> {
    prompt_password_with_label(&crate::t!("请输入 root 密码: ", "Enter root password: "))
}

/// 交互式获取密码，`label` 为提示文字（终端显示在提示符，图形对话框用作标题/文案）。
///
/// 优先级：终端（/dev/tty）-> 图形密码框（zenity/kdialog/yad）-> 失败。
pub fn prompt_password_with_label(label: &str) -> Result<String, String> {
    if crate::interact::tty_available() {
        read_password_tty(label)
    } else if crate::interact::has_display() {
        prompt_password_gui(label).ok_or_else(|| {
            crate::t!(
                "无法获取密码：未检测到可用的对话框工具 (zenity/kdialog/yad)",
                "cannot read password: no dialog tool found (zenity/kdialog/yad)"
            )
        })
    } else {
        Err(crate::t!(
            "无法进行密码输入：非终端且无图形环境（已默认拒绝）",
            "cannot prompt for password: no terminal and no GUI (denied by default)"
        ))
    }
}

/// 终端内无回显读取密码（通过 /dev/tty，避免 setuid 下 stdin 被重定向）。
fn read_password_tty(label: &str) -> Result<String, String> {
    let tty = CString::new("/dev/tty")
        .map_err(|_| crate::t!("路径含 NUL", "path contains NUL"))?;
    let fd = unsafe { libc::open(tty.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(crate::t!(
            "无法打开 /dev/tty：需要交互式终端",
            "cannot open /dev/tty: an interactive terminal is required"
        ));
    }

    let mut orig: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut orig) } != 0 {
        unsafe { libc::close(fd) };
        return Err(crate::t!("tcgetattr 失败", "tcgetattr failed"));
    }
    let guard = TtyRestore { fd, term: orig };

    let mut noecho = orig;
    noecho.c_lflag &= !(libc::ECHO as libc::tcflag_t);
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &noecho) } != 0 {
        return Err(crate::t!("tcsetattr 失败", "tcsetattr failed"));
    }

    unsafe {
        libc::write(fd, label.as_ptr() as *const libc::c_void, label.len());
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(fd, b.as_mut_ptr() as *mut libc::c_void, 1) };
        if n <= 0 {
            break;
        }
        if b[0] == b'\n' || b[0] == b'\r' {
            break;
        }
        bytes.push(b[0]);
    }
    unsafe {
        libc::write(fd, b"\n".as_ptr() as *const libc::c_void, 1);
    }
    drop(guard); // 恢复回显并关闭 fd
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 终端回显恢复守卫：任何路径退出都保证恢复 ECHO 并关闭 fd。
struct TtyRestore {
    fd: i32,
    term: libc::termios,
}

impl Drop for TtyRestore {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.term);
            libc::close(self.fd);
        }
    }
}

/// 图形密码框：zenity --password / kdialog --password / yad --entry --hide-text。
fn prompt_password_gui(label: &str) -> Option<String> {
    if let Some(z) = crate::interact::find_tool(&["zenity"]) {
        let out = std::process::Command::new(z)
            .arg("--password")
            .arg("--title")
            .arg(label)
            .output()
            .ok()?;
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    if let Some(k) = crate::interact::find_tool(&["kdialog"]) {
        let out = std::process::Command::new(k)
            .arg("--password")
            .arg(label)
            .output()
            .ok()?;
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    if let Some(y) = crate::interact::find_tool(&["yad"]) {
        let out = std::process::Command::new(y)
            .arg("--entry")
            .arg("--hide-text")
            .arg("--title")
            .arg(crate::t!("rtdo — 认证", "rtdo — authentication"))
            .arg("--text")
            .arg(label)
            .output()
            .ok()?;
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    None
}

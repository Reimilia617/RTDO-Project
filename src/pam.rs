//! PAM 认证：校验 root 密码（服务名 rtdo，配置位于 /etc/pam.d/rtdo）。
//!
//! 链接需求：libpam（Debian/Ubuntu: libpam0g-dev；Fedora: pam-devel；Arch: pam）。
//!
//! 首次使用时自动生成最小 PAM 服务文件 `/etc/pam.d/rtdo`：
//! ```text
//! auth required pam_unix.so
//! account required pam_unix.so
//! password required pam_unix.so
//! ```
//! 该链不包含 pam_rootok，避免 setuid 进程在认证环节被直接放行。
//! `password` 段用于首次运行引导中修改 root 密码（等价于 `sudo passwd` 的
//! pam_chauthtok 流程）；旧版生成的文件缺少该段，会在 ensure_pam_service 中自动补齐。

use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;

pub const PAM_SERVICE_PATH: &str = "/etc/pam.d/rtdo";
const PAM_SERVICE_CONTENT: &str = "auth required pam_unix.so\naccount required pam_unix.so\npassword required pam_unix.so\n";

const PAM_SUCCESS: c_int = 0;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

/// 确保 PAM 服务文件存在且完整（不存在或缺少 auth/account/password 段时自动创建/补齐）。
pub fn ensure_pam_service() -> Result<(), String> {
    if let Some(content) = std::fs::read_to_string(PAM_SERVICE_PATH).ok() {
        if content.contains("auth") && content.contains("account") && content.contains("password") {
            return Ok(());
        }
    }
    if !crate::exec::is_root() {
        return Err(crate::t!(
            "需要 root 权限以创建/更新 PAM 服务文件 {}（请先以 root/sudo 运行一次）",
            "root permission needed to create/update PAM service file {} (run once as root/sudo)",
            PAM_SERVICE_PATH
        ));
    }
    std::fs::write(PAM_SERVICE_PATH, PAM_SERVICE_CONTENT).map_err(|e| {
        crate::t!(
            "无法写入 PAM 服务文件 {}: {}",
            "cannot write PAM service file {}: {}",
            PAM_SERVICE_PATH,
            e
        )
    })
}

/// 通过 PAM 校验 root 密码。
pub fn verify_root_password(password: &str) -> Result<(), String> {
    verify_password("root", password)
}

/// 通过 PAM 校验指定用户的密码（用于验证调用者身份等）。
pub fn verify_password(user: &str, password: &str) -> Result<(), String> {
    let service = CString::new("rtdo")
        .map_err(|_| crate::t!("服务名含 NUL", "service name contains NUL"))?;
    let user = CString::new(user)
        .map_err(|_| crate::t!("用户名含 NUL", "username contains NUL"))?;
    let pw = CString::new(password)
        .map_err(|_| crate::t!("密码含 NUL", "password contains NUL"))?;

    let conv = PamConv {
        conv: Some(conv_func),
        appdata_ptr: pw.as_ptr() as *mut c_void,
    };

    let mut handle: *mut PamHandle = ptr::null_mut();
    let rc = unsafe { pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) };
    if rc != PAM_SUCCESS {
        return Err(crate::t!(
            "pam_start 失败 (PAM 错误码 {})",
            "pam_start failed (PAM error {})",
            rc
        ));
    }

    let auth = unsafe { pam_authenticate(handle, 0) };
    let acct = unsafe { pam_acct_mgmt(handle, 0) };
    unsafe { pam_end(handle, auth) };

    if auth != PAM_SUCCESS {
        return Err(crate::t!("密码错误", "wrong password"));
    }
    if acct != PAM_SUCCESS {
        return Err(crate::t!(
            "账户状态异常 (PAM 错误码 {})",
            "account state error (PAM error {})",
            acct
        ));
    }
    Ok(())
}

/// 通过 PAM 修改 root 密码（等价于 `sudo passwd root` 的核心步骤）。
///
/// 调用前进程必须拥有 root 权限（setuid root 时 euid 已为 0）；此处同时把
/// 真实 UID 置为 0（become_real_root），使 pam_unix 不再要求旧密码，直接
/// 通过会话函数收两次新密码并写入 /etc/shadow。
pub fn change_root_password(new_password: &str) -> Result<(), String> {
    crate::exec::become_real_root();

    let service = CString::new("rtdo")
        .map_err(|_| crate::t!("服务名含 NUL", "service name contains NUL"))?;
    let user = CString::new("root")
        .map_err(|_| crate::t!("用户名含 NUL", "username contains NUL"))?;
    let pw = CString::new(new_password)
        .map_err(|_| crate::t!("密码含 NUL", "password contains NUL"))?;

    let conv = PamConv {
        conv: Some(conv_func),
        appdata_ptr: pw.as_ptr() as *mut c_void,
    };

    let mut handle: *mut PamHandle = ptr::null_mut();
    let rc = unsafe { pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) };
    if rc != PAM_SUCCESS {
        return Err(crate::t!(
            "pam_start 失败 (PAM 错误码 {})",
            "pam_start failed (PAM error {})",
            rc
        ));
    }

    let rc = unsafe { pam_chauthtok(handle, 0) };
    unsafe { pam_end(handle, rc) };

    if rc != PAM_SUCCESS {
        return Err(crate::t!(
            "pam_chauthtok 失败 (PAM 错误码 {})",
            "pam_chauthtok failed (PAM error {})",
            rc
        ));
    }
    Ok(())
}

// ---- PAM FFI ----

/// struct pam_message
#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

/// struct pam_response
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

/// struct pam_conv
#[repr(C)]
struct PamConv {
    conv: Option<
        unsafe extern "C" fn(
            num_msg: c_int,
            msg: *const *mut PamMessage,
            resp: *mut *mut PamResponse,
            appdata: *mut c_void,
        ) -> c_int,
    >,
    appdata_ptr: *mut c_void,
}

/// 不透明句柄 pam_handle_t
#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        pamh: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_chauthtok(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut PamHandle, status: c_int) -> c_int;
}

/// 会话函数：把预先提供的密码返回给 PAM。
///
/// 内存说明：响应数组与字符串均用 libc 分配（calloc/strdup），
/// 释放交由 PAM 库处理；若库不释放则产生少量泄漏（进程短生命周期，可接受），
/// 但绝不会二次释放导致未定义行为。
unsafe extern "C" fn conv_func(
    num_msg: c_int,
    msgs: *const *mut PamMessage,
    resp_out: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if resp_out.is_null() {
        return PAM_CONV_ERR;
    }
    let n = num_msg as usize;
    let responses = libc::calloc(n, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if responses.is_null() {
        return PAM_CONV_ERR;
    }
    *resp_out = responses;

    let password = appdata as *const c_char;
    let empty: [libc::c_char; 1] = [0 as libc::c_char];

    for i in 0..n {
        let item = responses.add(i);
        (*item).resp = ptr::null_mut();
        (*item).resp_retcode = 0;
        if msgs.is_null() {
            continue;
        }
        let msg = *msgs.add(i);
        if msg.is_null() {
            continue;
        }
        match (*msg).msg_style {
            PAM_PROMPT_ECHO_OFF => {
                (*item).resp = libc::strdup(password);
            }
            PAM_PROMPT_ECHO_ON => {
                (*item).resp = libc::strdup(empty.as_ptr());
            }
            _ => {
                // PAM_ERROR_MSG / PAM_TEXT_INFO：resp 保持 NULL
            }
        }
    }
    PAM_SUCCESS
}

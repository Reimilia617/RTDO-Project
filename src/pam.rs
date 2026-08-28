//! PAM 认证：校验 root 密码（服务名 rtdo，配置位于 /etc/pam.d/rtdo）。
//!
//! 链接需求：libpam（Debian/Ubuntu: libpam0g-dev；Fedora: pam-devel；Arch: pam）。
//!
//! 首次使用时自动生成最小 PAM 服务文件 `/etc/pam.d/rtdo`：
//! ```text
//! auth required pam_unix.so
//! account required pam_unix.so
//! ```
//! 该链不包含 pam_rootok，避免 setuid 进程在认证环节被直接放行。

use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr;

pub const PAM_SERVICE_PATH: &str = "/etc/pam.d/rtdo";
const PAM_SERVICE_CONTENT: &str = "auth required pam_unix.so\naccount required pam_unix.so\n";

const PAM_SUCCESS: c_int = 0;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

/// 确保 PAM 服务文件存在（不存在且为 root 时自动创建）。
pub fn ensure_pam_service() -> Result<(), String> {
    if std::path::Path::new(PAM_SERVICE_PATH).exists() {
        return Ok(());
    }
    if !crate::exec::is_root() {
        return Err(format!(
            "需要 root 权限以创建 PAM 服务文件 {}（请先以 root/sudo 运行一次）",
            PAM_SERVICE_PATH
        ));
    }
    std::fs::write(PAM_SERVICE_PATH, PAM_SERVICE_CONTENT)
        .map_err(|e| format!("无法创建 PAM 服务文件 {}: {}", PAM_SERVICE_PATH, e))
}

/// 通过 PAM 校验 root 密码。
pub fn verify_root_password(password: &str) -> Result<(), String> {
    let service = CString::new("rtdo").map_err(|_| "服务名含 NUL".to_string())?;
    let user = CString::new("root").map_err(|_| "用户名含 NUL".to_string())?;
    let pw = CString::new(password).map_err(|_| "密码含 NUL".to_string())?;

    let conv = PamConv {
        conv: Some(conv_func),
        appdata_ptr: pw.as_ptr() as *mut c_void,
    };

    let mut handle: *mut PamHandle = ptr::null_mut();
    let rc = unsafe { pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) };
    if rc != PAM_SUCCESS {
        return Err(format!("pam_start 失败 (PAM 错误码 {})", rc));
    }

    let auth = unsafe { pam_authenticate(handle, 0) };
    let acct = unsafe { pam_acct_mgmt(handle, 0) };
    unsafe { pam_end(handle, auth) };

    if auth != PAM_SUCCESS {
        return Err("密码错误".to_string());
    }
    if acct != PAM_SUCCESS {
        return Err(format!("账户状态异常 (PAM 错误码 {})", acct));
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

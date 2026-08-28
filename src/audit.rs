//! 审计日志：所有提权请求（被拦截与放行的操作）追加写入日志文件，
//! 支持 JSON（默认）与纯文本两种格式。

use serde::Serialize;

/// 一条审计记录。
#[derive(Serialize)]
pub struct AuditEntry {
    /// 执行时间（RFC3339）。
    pub time: String,
    /// 操作用户。
    pub user: String,
    /// 用户 UID。
    pub uid: u32,
    /// 完整命令。
    pub command: String,
    /// 是否使用 --force。
    pub force: bool,
    /// 最终结果：allow / deny / blocked。
    pub result: String,
    /// 原因：trusted_path / user_confirmed / user_denied / blacklist / force / auth_failed / ...
    pub reason: String,
    /// 退出码（仅放行并执行完毕时存在）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl AuditEntry {
    /// 构造一条记录（结果已知但尚未执行）。
    pub fn new(command: &str, force: bool, result: &str, reason: &str) -> Self {
        Self {
            time: chrono::Utc::now().to_rfc3339(),
            user: crate::exec::current_user(),
            uid: crate::exec::real_uid(),
            command: command.to_string(),
            force,
            result: result.to_string(),
            reason: reason.to_string(),
            exit_code: None,
        }
    }

    /// 构造一条已执行的记录（含退出码）。
    pub fn with_exit(
        command: &str,
        force: bool,
        result: &str,
        reason: &str,
        exit_code: i32,
    ) -> Self {
        let mut e = Self::new(command, force, result, reason);
        e.exit_code = Some(exit_code);
        e
    }
}

/// 追加写入一条审计记录。
pub fn log(entry: &AuditEntry, log_path: &str, format: &str) {
    let line = match format {
        "text" => format_text(entry),
        _ => serde_json::to_string(entry).unwrap_or_default(),
    };
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        Ok(mut f) => {
            use std::io::Write;
            if let Err(e) = writeln!(f, "{}", line) {
                eprintln!(
                    "{}",
                    crate::t!(
                        "rtdo: 警告: 写入审计日志失败: {}",
                        "rtdo: warning: failed to write audit log: {}",
                        e
                    )
                );
            }
        }
        Err(e) => eprintln!(
            "{}",
            crate::t!(
                "rtdo: 警告: 无法打开审计日志 {}: {}",
                "rtdo: warning: cannot open audit log {}: {}",
                log_path,
                e
            )
        ),
    }
}

/// 纯文本格式。
fn format_text(e: &AuditEntry) -> String {
    let escaped = e.command.replace('\\', "\\\\").replace('"', "\\\"");
    let mut s = format!(
        "[{}] user={} uid={} force={} result={} reason={}",
        e.time, e.user, e.uid, e.force, e.result, e.reason
    );
    if let Some(code) = e.exit_code {
        s.push_str(&format!(" exit={}", code));
    }
    s.push_str(&format!(" cmd=\"{}\"", escaped));
    s
}

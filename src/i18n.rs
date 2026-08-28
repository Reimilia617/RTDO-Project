//! 轻量中英双语（i18n）支持。
//!
//! 语言选择规则（优先级从高到低）：
//! 1. 环境变量 `RTDO_LANG`：`zh*` → 中文，其他 → 英文；
//! 2. `LC_ALL` / `LC_MESSAGES` / `LANG`：以 `zh` 开头 → 中文，否则英文；
//! 3. 默认英文。
//!
//! 用法（配合 `t!` 宏）：
//! ```rust
//! println!("{}", crate::t!("中文消息", "English message"));
//! println!("{}", crate::t!("欢迎 {} 用户", "Welcome, user {}", name));
//! ```

use std::sync::OnceLock;

/// 当前是否为中文环境。
pub fn is_zh() -> bool {
    static ZH: OnceLock<bool> = OnceLock::new();
    *ZH.get_or_init(detect_zh)
}

fn detect_zh() -> bool {
    if let Ok(v) = std::env::var("RTDO_LANG") {
        return v.trim().to_lowercase().starts_with("zh");
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_lowercase();
            if !v.is_empty() {
                return v.starts_with("zh");
            }
        }
    }
    false
}

/// 按当前语言返回消息。
pub fn pick<'a>(zh: &'a str, en: &'a str) -> &'a str {
    if is_zh() {
        zh
    } else {
        en
    }
}

/// 用参数依次替换消息中的 `{}` 占位符（多余的占位符保留原样）。
fn fill(s: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(s.len() + args.len() * 4);
    let mut rest = s;
    let mut i = 0;
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if i < args.len() {
            out.push_str(args[i]);
            i += 1;
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

pub fn fmt0(zh: &str, en: &str) -> String {
    pick(zh, en).to_string()
}

pub fn fmt1(zh: &str, en: &str, a: &str) -> String {
    fill(pick(zh, en), &[a])
}

pub fn fmt2(zh: &str, en: &str, a: &str, b: &str) -> String {
    fill(pick(zh, en), &[a, b])
}

pub fn fmt3(zh: &str, en: &str, a: &str, b: &str, c: &str) -> String {
    fill(pick(zh, en), &[a, b, c])
}

/// 双语消息宏：`t!("中文", "English")` 或 `t!("中文 {}", "English {}", 参数)`。
#[macro_export]
macro_rules! t {
    ($zh:literal, $en:literal) => {
        $crate::i18n::fmt0($zh, $en)
    };
    ($zh:literal, $en:literal, $a:expr) => {
        $crate::i18n::fmt1($zh, $en, &format!("{}", $a))
    };
    ($zh:literal, $en:literal, $a:expr, $b:expr) => {
        $crate::i18n::fmt2($zh, $en, &format!("{}", $a), &format!("{}", $b))
    };
    ($zh:literal, $en:literal, $a:expr, $b:expr, $c:expr) => {
        $crate::i18n::fmt3(
            $zh,
            $en,
            &format!("{}", $a),
            &format!("{}", $b),
            &format!("{}", $c),
        )
    };
}

//! 策略判定：信任目录自动放行、高危黑名单拦截、路径提取与通配符匹配。

use crate::config::Config;

/// 判定结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 命中黑名单，直接拦截（仅 --force 可放行）。
    BlockBlacklist,
    /// 命令涉及的全部路径位于信任目录内，自动放行。
    AllowTrusted,
    /// --force 强制放行。
    AllowForce,
    /// 需要交互确认。
    Prompt,
}

/// 对命令进行分类。
pub fn classify(cmd: &str, args: &[String], cfg: &Config) -> Verdict {
    if is_blacklisted(cmd, args, &cfg.blacklist_commands) {
        return Verdict::BlockBlacklist;
    }
    let paths = extract_paths(cmd, args);
    if !paths.is_empty() && paths.iter().all(|p| is_trusted(p, &cfg.trusted_paths)) {
        return Verdict::AllowTrusted;
    }
    Verdict::Prompt
}

/// 判断命令是否命中黑名单。
///
/// 匹配规则：
/// * 条目为单个命令名（无空格）时，按命令 basename 精确匹配；
/// * 条目带参数时，按“命令名 + 参数前缀”匹配（如 `rm -rf /` 命中 `rm -rf /...`）。
fn is_blacklisted(cmd: &str, args: &[String], blacklist: &[String]) -> bool {
    let argv0 = std::path::Path::new(cmd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cmd.to_string());

    for entry in blacklist {
        let tokens: Vec<&str> = entry.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let first_ok = argv0 == tokens[0] || cmd == tokens[0];
        if !first_ok {
            continue;
        }
        if tokens.len() == 1 {
            return true;
        }
        // 剩余 token 按参数前缀匹配。
        // 前缀以 / 结尾时视为路径前缀，允许延续（如 "rm -rf /" 命中 rm -rf /tmp）；
        // 否则要求单词边界（如 "chmod -R 777" 不命中 "chmod -R 7777"）。
        let joined = args.join(" ");
        let rest = tokens[1..].join(" ");
        if joined == rest {
            return true;
        }
        if joined.len() > rest.len() && joined.starts_with(&rest) {
            let next = joined.as_bytes()[rest.len()];
            if rest.ends_with('/') || next == b' ' {
                return true;
            }
        }
    }
    false
}

/// 从命令行中启发式提取“可能被操作”的路径参数。
///
/// 规则：以 `/`、`.`、`~` 开头或包含 `/` 的参数视为路径；
/// `--foo=/path` 形式的选项值也会被提取。
/// 无法识别的命令一律进入确认流程（默认拦截，保证安全）。
fn extract_paths(cmd: &str, args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if looks_like_path(cmd) {
        out.push(cmd.to_string());
    }
    for a in args {
        if let Some(rest) = a.strip_prefix("--") {
            if let Some((_, v)) = rest.split_once('=') {
                if looks_like_path(v) {
                    out.push(v.to_string());
                }
            }
        } else if looks_like_path(a) {
            out.push(a.clone());
        }
    }
    out
}

/// 判断字符串是否“看起来像路径”。
fn looks_like_path(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.starts_with('/') || s.starts_with('.') || s.starts_with('~') {
        return true;
    }
    s.contains('/')
}

/// 判断路径是否位于某个信任目录内（支持 *、**、? 通配符）。
fn is_trusted(path: &str, trusted: &[String]) -> bool {
    trusted.iter().any(|t| glob_match(t, path))
}

/// 判断 path 是否以 pattern 开头（按路径段比较），支持通配符。
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat_segs: Vec<&str> = pattern.split('/').collect();
    let txt_segs: Vec<&str> = path.split('/').collect();
    segments_prefix_match(&pat_segs, &txt_segs)
}

/// 模式段必须是文本段的前缀；`**` 段可跨任意层级匹配。
fn segments_prefix_match(pat: &[&str], txt: &[&str]) -> bool {
    if pat.is_empty() {
        return true;
    }
    if pat[0] == "**" {
        for skip in 0..=txt.len() {
            if segments_prefix_match(&pat[1..], &txt[skip..]) {
                return true;
            }
        }
        false
    } else {
        if txt.is_empty() {
            return false;
        }
        if seg_match(pat[0], txt[0]) {
            segments_prefix_match(&pat[1..], &txt[1..])
        } else {
            false
        }
    }
}

/// 单段匹配：`*` 匹配任意字符序列，`?` 匹配单个字符（不跨 `/`）。
fn seg_match(pat: &str, seg: &str) -> bool {
    if !pat.contains('*') && !pat.contains('?') {
        return pat == seg;
    }
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = seg.chars().collect();
    wild_match(&p, &t)
}

/// 经典带回溯的通配符匹配（* 匹配任意长度，? 匹配单个字符）。
fn wild_match(p: &[char], t: &[char]) -> bool {
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star = usize::MAX; // 最近一个 * 在 p 中的位置
    let mut mark = 0usize; // 与 star 对应的 t 位置
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

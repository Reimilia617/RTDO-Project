//! 配置加载与默认策略生成。
//!
//! 配置文件为 TOML 格式，默认路径 `/etc/rtdo/rtdo.conf`，
//! 可通过 `--config <路径>` 或环境变量 `RTDO_CONF` 覆盖。

use serde::Deserialize;
use std::process::ExitCode;

pub const DEFAULT_CONFIG_DIR: &str = "/etc/rtdo";
pub const DEFAULT_CONFIG_PATH: &str = "/etc/rtdo/rtdo.conf";

/// rtdo 配置。
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// 信任目录：命令作用到这些目录内时自动放行，不弹窗。
    pub trusted_paths: Vec<String>,
    /// 高危命令：任何情况下都会拦截，除非使用 --force。
    pub blacklist_commands: Vec<String>,
    /// 审计日志路径。
    pub audit_log: String,
    /// 日志格式："json" 或 "text"。
    pub log_format: String,
}

impl Config {
    /// 规范化：填充默认值并校验。
    fn normalize(&mut self) -> Result<(), String> {
        if self.audit_log.trim().is_empty() {
            self.audit_log = "/var/log/rtdo.log".to_string();
        }
        let lf = self.log_format.trim().to_lowercase();
        if lf.is_empty() {
            self.log_format = "json".to_string();
        } else if lf != "json" && lf != "text" {
            return Err(format!(
                "log_format 只能是 json 或 text，当前: {}",
                self.log_format
            ));
        } else {
            self.log_format = lf;
        }
        Ok(())
    }
}

/// 解析配置文件路径：命令行指定优先，否则使用默认路径。
fn config_path(override_path: Option<&str>) -> String {
    match override_path {
        Some(p) => p.to_string(),
        None => std::env::var("RTDO_CONF").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string()),
    }
}

/// 加载配置。
pub fn load(override_path: Option<&str>) -> Result<Config, String> {
    let path = config_path(override_path);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("无法读取配置文件 {}: {}", path, e))?;
    let mut cfg: Config =
        toml::from_str(&content).map_err(|e| format!("配置文件 {} 解析失败: {}", path, e))?;
    cfg.normalize()?;
    Ok(cfg)
}

/// 默认配置文件内容（与 README 示例一致）。
pub fn default_content() -> String {
    r#"# rtdo 配置文件（TOML 格式）
# 信任目录：命令作用到这些目录内时自动放行，不弹窗。
# 支持通配符: * 匹配单个路径段内的任意字符, ** 匹配任意层级的路径段, ? 匹配单个字符。
trusted_paths = [
    "/etc/nixos",
    "/home/*/.config",
    "/usr/local/bin"
]

# 高危命令：任何情况下都会拦截，除非使用 --force。
# 单条命令名按 basename 精确匹配（如 "dd" 匹配任意位置调用的 dd）；
# 带参数的命令按“命令名 + 参数前缀”匹配（如 "rm -rf /" 匹配 rm -rf /...）。
blacklist_commands = [
    "dd",
    "mkfs",
    "rm -rf /",
    "chmod -R 777 /"
]

# 审计日志路径
audit_log = "/var/log/rtdo.log"

# 日志格式: "json" 或 "text"
log_format = "json"
"#
    .to_string()
}

/// `rtdo --init`：生成默认配置与 PAM 认证服务文件（需要 root）。
pub fn init(override_path: Option<&str>) -> ExitCode {
    let path = config_path(override_path);

    if !crate::exec::is_root() {
        eprintln!("rtdo: --init 需要 root 权限，请使用 `sudo rtdo --init`。");
        return ExitCode::from(1);
    }

    // 创建配置目录
    if let Some(dir) = std::path::Path::new(&path).parent() {
        if !dir.as_os_str().is_empty() && !dir.exists() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("rtdo: 无法创建目录 {}: {}", dir.display(), e);
                return ExitCode::from(1);
            }
        }
    }

    if std::path::Path::new(&path).exists() {
        eprintln!("rtdo: 配置文件已存在，未覆盖: {}", path);
    } else {
        match std::fs::write(&path, default_content()) {
            Ok(()) => println!("rtdo: 已生成默认配置: {}", path),
            Err(e) => {
                eprintln!("rtdo: 无法写入配置文件 {}: {}", path, e);
                return ExitCode::from(1);
            }
        }
    }

    // PAM 认证服务（rtdo 依赖它校验 root 密码）
    match crate::pam::ensure_pam_service() {
        Ok(()) => println!("rtdo: PAM 认证服务已就绪 (/etc/pam.d/rtdo)"),
        Err(e) => eprintln!("rtdo: 警告: {}", e),
    }

    ExitCode::SUCCESS
}

/// `rtdo --policy`：打印当前策略。
pub fn print_policy(override_path: Option<&str>) -> ExitCode {
    match load(override_path) {
        Ok(cfg) => {
            println!("rtdo 策略（{}）", config_path(override_path));
            println!("信任目录:");
            if cfg.trusted_paths.is_empty() {
                println!("  （无，所有命令都需要确认）");
            } else {
                for t in &cfg.trusted_paths {
                    println!("  {}", t);
                }
            }
            println!("高危命令:");
            if cfg.blacklist_commands.is_empty() {
                println!("  （无）");
            } else {
                for b in &cfg.blacklist_commands {
                    println!("  {}", b);
                }
            }
            println!("审计日志: {}", cfg.audit_log);
            println!("日志格式: {}", cfg.log_format);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("rtdo: {}", e);
            eprintln!("提示: 可运行 `sudo rtdo --init` 生成默认配置。");
            ExitCode::from(1)
        }
    }
}

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
            return Err(crate::t!(
                "log_format 只能是 json 或 text，当前: {}",
                "log_format must be json or text, got: {}",
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
    let content = std::fs::read_to_string(&path).map_err(|e| {
        crate::t!(
            "无法读取配置文件 {}: {}",
            "cannot read config file {}: {}",
            path,
            e
        )
    })?;
    let mut cfg: Config = toml::from_str(&content).map_err(|e| {
        crate::t!(
            "配置文件 {} 解析失败: {}",
            "failed to parse config {}: {}",
            path,
            e
        )
    })?;
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

/// `rtdo --init`：检查 sudo 前置条件、生成默认配置与 PAM 认证服务文件（需要 root）。
pub fn init(override_path: Option<&str>) -> ExitCode {
    let path = config_path(override_path);

    if !crate::exec::is_root() {
        eprintln!(
            "{}",
            crate::t!(
                "rtdo: --init 需要 root 权限，请使用 `sudo rtdo --init`。",
                "rtdo: --init requires root; run `sudo rtdo --init`."
            )
        );
        return ExitCode::from(1);
    }

    // 安装前置检查：sudo 是否已安装并正确配置（提示先装好 sudo 再安装 rtdo）
    eprintln!("{}", crate::t!("检查 sudo 前置条件…", "Checking sudo prerequisite…"));
    for hint in crate::setup::check_sudo_ready() {
        eprintln!("{}", hint);
    }

    // 创建配置目录
    if let Some(dir) = std::path::Path::new(&path).parent() {
        if !dir.as_os_str().is_empty() && !dir.exists() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!(
                    "{}",
                    crate::t!(
                        "rtdo: 无法创建目录 {}: {}",
                        "rtdo: cannot create directory {}: {}",
                        dir.display(),
                        e
                    )
                );
                return ExitCode::from(1);
            }
        }
    }

    if std::path::Path::new(&path).exists() {
        eprintln!(
            "{}",
            crate::t!(
                "rtdo: 配置文件已存在，未覆盖: {}",
                "rtdo: config file already exists, not overwritten: {}",
                path
            )
        );
    } else {
        match std::fs::write(&path, default_content()) {
            Ok(()) => println!(
                "{}",
                crate::t!(
                    "rtdo: 已生成默认配置: {}",
                    "rtdo: default config written: {}",
                    path
                )
            ),
            Err(e) => {
                eprintln!(
                    "{}",
                    crate::t!(
                        "rtdo: 无法写入配置文件 {}: {}",
                        "rtdo: cannot write config file {}: {}",
                        path,
                        e
                    )
                );
                return ExitCode::from(1);
            }
        }
    }

    // PAM 认证服务（rtdo 依赖它校验 root 密码）
    match crate::pam::ensure_pam_service() {
        Ok(()) => println!(
            "{}",
            crate::t!(
                "rtdo: PAM 认证服务已就绪 (/etc/pam.d/rtdo)",
                "rtdo: PAM auth service ready (/etc/pam.d/rtdo)"
            )
        ),
        Err(e) => eprintln!(
            "{}",
            crate::t!("rtdo: 警告: {}", "rtdo: warning: {}", e)
        ),
    }

    ExitCode::SUCCESS
}

/// `rtdo --policy`：打印当前策略。
pub fn print_policy(override_path: Option<&str>) -> ExitCode {
    match load(override_path) {
        Ok(cfg) => {
            println!(
                "{}",
                crate::t!("rtdo 策略（{}）", "rtdo policy ({})", config_path(override_path))
            );
            println!("{}", crate::t!("信任目录:", "trusted paths:"));
            if cfg.trusted_paths.is_empty() {
                println!(
                    "{}",
                    crate::t!("  （无，所有命令都需要确认）", "  (none — every command needs confirmation)")
                );
            } else {
                for t in &cfg.trusted_paths {
                    println!("  {}", t);
                }
            }
            println!("{}", crate::t!("高危命令:", "blacklist:"));
            if cfg.blacklist_commands.is_empty() {
                println!("{}", crate::t!("  （无）", "  (none)"));
            } else {
                for b in &cfg.blacklist_commands {
                    println!("  {}", b);
                }
            }
            println!(
                "{}",
                crate::t!("审计日志: {}", "audit log: {}", cfg.audit_log)
            );
            println!(
                "{}",
                crate::t!("日志格式: {}", "log format: {}", cfg.log_format)
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("rtdo: {}", e);
            eprintln!(
                "{}",
                crate::t!(
                    "提示: 可运行 `sudo rtdo --init` 生成默认配置。",
                    "hint: run `sudo rtdo --init` to generate the default config."
                )
            );
            ExitCode::from(1)
        }
    }
}

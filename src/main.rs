//! rtdo — Root Task Do
//!
//! 一个可交互的、环境自适应的提权工具，用于替代传统 sudo。
//!
//! 设计目标：
//! * 默认拦截：除非用户明确允许，任何提权操作都不会执行。
//! * 环境自适应：图形桌面下弹窗提醒，终端中输出文本提示。
//! * 策略驱动：通过配置文件定义“信任目录”与“高危操作”，仅对真正危险的操作进行拦截。
//! * 单文件部署：一个二进制文件 + 一个配置文件，复制到对应目录即可使用。
//! * 安装需 root：rtdo 需要由 root 用户安装到系统目录（setuid root）。
//!
//! 多语言：输出根据 `RTDO_LANG` / `LC_ALL` / `LC_MESSAGES` / `LANG` 自动选择中文或英文。

mod audit;
mod auth;
mod config;
mod exec;
mod i18n;
mod interact;
mod pam;
mod policy;
mod setup;

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 解析后的命令行参数。
struct Args {
    force: bool,
    config: Option<String>,
    action: Action,
}

enum Action {
    /// `rtdo --init`：生成默认配置与 PAM 认证服务。
    Init,
    /// `rtdo --policy`：查看当前策略。
    Policy,
    /// `rtdo [--force] <命令> [参数...]`：提权执行。
    Run(Vec<String>),
}

fn print_usage() {
    println!(
        "{}",
        crate::t!(
            "rtdo {} — Root Task Do：交互式、环境自适应的提权工具（替代 sudo）",
            "rtdo {} — Root Task Do: interactive, environment-adaptive privilege elevation tool (sudo alternative)",
            VERSION
        )
    );
    println!();
    println!(
        "{}",
        crate::t!("用法:", "usage:")
    );
    println!(
        "{}",
        crate::t!(
            "  rtdo [选项] <命令> [参数...]   以 root 身份执行命令（自动拦截 / 交互确认）",
            "  rtdo [options] <command> [args...]   run command as root (auto-block / interactive confirm)"
        )
    );
    println!(
        "{}",
        crate::t!(
            "  rtdo --force <命令> [参数...]  强制放行，跳过所有拦截（仍需认证）",
            "  rtdo --force <command> [args...]  force-run, skip all blocks (auth still required)"
        )
    );
    println!(
        "{}",
        crate::t!("  rtdo --policy                  查看当前策略", "  rtdo --policy                  show current policy")
    );
    println!(
        "{}",
        crate::t!(
            "  rtdo --init                    生成默认配置文件（需要 root）",
            "  rtdo --init                    generate default config (requires root)"
        )
    );
    println!(
        "{}",
        crate::t!("  rtdo --help                    显示帮助", "  rtdo --help                    show help")
    );
    println!(
        "{}",
        crate::t!("  rtdo --version                 显示版本", "  rtdo --version                 show version")
    );
    println!();
    println!(
        "{}",
        crate::t!("选项:", "options:")
    );
    println!(
        "{}",
        crate::t!(
            "  --force          跳过拦截（黑名单与确认），仍需认证",
            "  --force          skip blocks (blacklist & confirmation); auth still required"
        )
    );
    println!(
        "{}",
        crate::t!(
            "  --config <路径>  指定配置文件（默认 {}/rtdo.conf）",
            "  --config <path>  config file (default {}/rtdo.conf)",
            config::DEFAULT_CONFIG_DIR
        )
    );
    println!(
        "{}",
        crate::t!("  -h, --help       显示帮助", "  -h, --help       show help")
    );
    println!(
        "{}",
        crate::t!("  -V, --version    显示版本", "  -V, --version    show version")
    );
    println!();
    println!(
        "{}",
        crate::t!(
            "首次使用: rtdo 认证的是 root 密码。若 root 未设置密码（如 WSL、默认锁定 root 的发行版），",
            "first run: rtdo authenticates against the root password. If root has no password (e.g. WSL,"
        )
    );
    println!(
        "{}",
        crate::t!(
            "          首次运行会自动引导设置（等价于 `sudo passwd`：先验证当前用户密码，再设置新的 root 密码）。",
            "          distros that lock root by default), the first run guides you through setting one"
        )
    );
    println!(
        "{}",
        crate::t!(
            "          也可手动执行 `sudo passwd` 或在 root 用户下执行 `passwd`。",
            "          (equivalent to `sudo passwd`). You can also run `sudo passwd` manually, or `passwd` as root."
        )
    );
    println!(
        "{}",
        crate::t!(
            "多语言: 输出语言由 RTDO_LANG / LC_ALL / LC_MESSAGES / LANG 自动决定（zh* 为中文，其他为英文）。",
            "language: output language follows RTDO_LANG / LC_ALL / LC_MESSAGES / LANG (zh* → Chinese, otherwise English)."
        )
    );
}

/// 解析命令行参数。
fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut force = false;
    let mut config: Option<String> = None;
    let mut command: Vec<String> = Vec::new();
    let mut action: Option<Action> = None;
    let mut in_command = false;
    let mut i = 0;

    while i < raw.len() {
        let a = &raw[i];
        if !in_command {
            match a.as_str() {
                "--force" => force = true,
                "--config" => {
                    i += 1;
                    if i >= raw.len() {
                        return Err(crate::t!(
                            "--config 后缺少路径参数",
                            "--config requires a path argument"
                        ));
                    }
                    config = Some(raw[i].clone());
                }
                "--policy" => action = Some(Action::Policy),
                "--init" => action = Some(Action::Init),
                "--" => in_command = true,
                _ if a.starts_with('-') && a.len() > 1 => {
                    return Err(crate::t!("未知选项: {}", "unknown option: {}", a));
                }
                _ => {
                    // 第一个非选项参数开始，后续全部作为命令参数
                    in_command = true;
                    command.push(a.clone());
                }
            }
        } else {
            command.push(a.clone());
        }
        i += 1;
    }

    let action = match action {
        Some(a) => {
            if !command.is_empty() {
                return Err(crate::t!(
                    "--init / --policy 不能与要执行的命令同时使用",
                    "--init/--policy cannot be combined with a command"
                ));
            }
            a
        }
        None => {
            if command.is_empty() {
                return Err(crate::t!(
                    "缺少要执行的命令",
                    "missing command to run"
                ));
            }
            Action::Run(command)
        }
    };

    Ok(Args { force, config, action })
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();

    if raw.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return ExitCode::SUCCESS;
    }
    if raw.iter().any(|a| a == "-V" || a == "--version") {
        println!("rtdo {}", VERSION);
        return ExitCode::SUCCESS;
    }

    let args = match parse_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rtdo: {}", e);
            eprintln!(
                "{}",
                crate::t!("用法: rtdo --help", "usage: rtdo --help")
            );
            return ExitCode::from(2);
        }
    };

    match args.action {
        Action::Init => config::init(args.config.as_deref()),
        Action::Policy => config::print_policy(args.config.as_deref()),
        Action::Run(argv) => run_command(&argv, args.force, args.config.as_deref()),
    }
}

/// 提权执行主流程：
/// 配置加载 -> root 检查 -> 策略判定 -> （黑名单直接拦截 / 非交互提前拒绝）-> 认证 -> 交互确认 -> 以 root 执行。
fn run_command(argv: &[String], force: bool, config_path: Option<&str>) -> ExitCode {
    // 1. 加载配置（同时提供审计日志路径）
    let cfg = match config::load(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rtdo: {}", e);
            eprintln!(
                "{}",
                crate::t!(
                    "提示: 可运行 `sudo rtdo --init` 生成默认配置",
                    "hint: run `sudo rtdo --init` to generate the default config"
                )
            );
            return ExitCode::from(1);
        }
    };

    let cmd_display = argv[0].clone();
    let args_display = argv[1..].join(" ");
    let full_cmd = argv.join(" ");

    // 2. 权限检查：必须为 root（setuid root 或直接以 root/sudo 运行）
    if !exec::is_root() {
        eprintln!(
            "{}",
            crate::t!(
                "rtdo: 必须以 root 权限运行。请先安装为 setuid root:",
                "rtdo: must run with root privileges. Install it setuid root first:"
            )
        );
        eprintln!("    sudo cp target/release/rtdo /usr/local/bin/rtdo");
        eprintln!("    sudo chown root:root /usr/local/bin/rtdo");
        eprintln!("    sudo chmod u+s /usr/local/bin/rtdo");
        eprintln!(
            "{}",
            crate::t!(
                "  或通过 root / sudo 直接调用。",
                "  or invoke it directly as root / via sudo."
            )
        );
        audit::log(
            &audit::AuditEntry::new(&full_cmd, force, "blocked", "not_root"),
            &cfg.audit_log,
            &cfg.log_format,
        );
        return ExitCode::from(1);
    }

    // 3. 策略判定（--force 直接放行，跳过所有拦截）
    let verdict = if force {
        policy::Verdict::AllowForce
    } else {
        policy::classify(&argv[0], &argv[1..], &cfg)
    };

    // 3a. 黑名单：直接拦截并通知（无论有无桌面环境），不需要先要密码
    if verdict == policy::Verdict::BlockBlacklist {
        interact::notify_blocked(
            &cmd_display,
            &args_display,
            &crate::t!("命中高危命令黑名单", "matches dangerous command blacklist"),
        );
        audit::log(
            &audit::AuditEntry::new(&full_cmd, force, "blocked", "blacklist"),
            &cfg.audit_log,
            &cfg.log_format,
        );
        return ExitCode::from(1);
    }

    // 3b. 需要确认但无任何交互通道：提前默认拒绝（避免无谓的密码输入）
    if verdict == policy::Verdict::Prompt && interact::detect_channel() == interact::Channel::None {
        eprintln!(
            "{}",
            crate::t!(
                "rtdo: 无法交互确认（既非终端也无可用的图形对话框），已默认拒绝；如确需执行请使用 --force。",
                "rtdo: cannot confirm interactively (no terminal or GUI dialog); denied by default; use --force if you must."
            )
        );
        audit::log(
            &audit::AuditEntry::new(&full_cmd, force, "deny", "no_interactive_channel"),
            &cfg.audit_log,
            &cfg.log_format,
        );
        return ExitCode::from(1);
    }

    // 4. 认证：非 root 直接调用时，必须验证 root 密码
    if exec::real_uid() != 0 {
        if let Err(e) = pam::ensure_pam_service() {
            eprintln!("rtdo: {}", e);
            audit::log(
                &audit::AuditEntry::new(&full_cmd, force, "blocked", "pam_setup_failed"),
                &cfg.audit_log,
                &cfg.log_format,
            );
            return ExitCode::from(1);
        }

        // 4a. 首次运行引导：root 密码未设置（/etc/shadow 为空、! 锁定或 * 禁用）时，
        //     强制进入设置流程（等价于自动执行 `sudo passwd`）：
        //     验证当前用户密码 -> 输入两次新的 root 密码 -> 校验新密码不与当前用户密码相同 -> 写入
        //     -> 询问是否禁用 sudo（全局别名 sudo=rtdo）。
        if !setup::root_password_set() {
            match setup::bootstrap_root_password(&exec::current_user()) {
                Ok(()) => {
                    println!(
                        "{}",
                        crate::t!(
                            "rtdo: root 密码设置完成，请使用新的 root 密码完成本次认证。",
                            "rtdo: root password set. Please use the new root password to authenticate."
                        )
                    );
                    // 提示是否禁用 sudo（设置全局别名 sudo=rtdo）
                    if let Err(e) = setup::offer_disable_sudo() {
                        eprintln!(
                            "{}",
                            crate::t!(
                                "rtdo: 警告: 设置 sudo→rtdo 别名失败: {}",
                                "rtdo: warning: failed to set sudo→rtdo alias: {}",
                                e
                            )
                        );
                    }
                    audit::log(
                        &audit::AuditEntry::new(&full_cmd, force, "allow", "root_password_setup"),
                        &cfg.audit_log,
                        &cfg.log_format,
                    );
                }
                Err(e) => {
                    eprintln!("rtdo: {}", e);
                    audit::log(
                        &audit::AuditEntry::new(
                            &full_cmd,
                            force,
                            "blocked",
                            "root_password_setup_failed",
                        ),
                        &cfg.audit_log,
                        &cfg.log_format,
                    );
                    return ExitCode::from(1);
                }
            }
        }

        let mut attempts = 3;
        loop {
            let password = match auth::prompt_password() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("rtdo: {}", e);
                    audit::log(
                        &audit::AuditEntry::new(&full_cmd, force, "blocked", "auth_unavailable"),
                        &cfg.audit_log,
                        &cfg.log_format,
                    );
                    return ExitCode::from(1);
                }
            };
            match pam::verify_root_password(&password) {
                Ok(()) => break,
                Err(e) => {
                    attempts -= 1;
                    if attempts == 0 {
                        eprintln!(
                            "{}",
                            crate::t!(
                                "rtdo: 认证失败，已拒绝执行。",
                                "rtdo: authentication failed; execution denied."
                            )
                        );
                        audit::log(
                            &audit::AuditEntry::new(&full_cmd, force, "deny", "auth_failed"),
                            &cfg.audit_log,
                            &cfg.log_format,
                        );
                        return ExitCode::from(1);
                    }
                    eprintln!(
                        "{}",
                        crate::t!(
                            "rtdo: {}（剩余 {} 次机会）",
                            "rtdo: {} ({} attempts left)",
                            e,
                            attempts
                        )
                    );
                }
            }
        }
    }

    // 5. 按判定结果处理
    match verdict {
        policy::Verdict::AllowTrusted => {
            let code = exec::run_as_root(argv);
            audit::log(
                &audit::AuditEntry::with_exit(&full_cmd, force, "allow", "trusted_path", code),
                &cfg.audit_log,
                &cfg.log_format,
            );
            ExitCode::from(code as u8)
        }
        policy::Verdict::AllowForce => {
            let code = exec::run_as_root(argv);
            audit::log(
                &audit::AuditEntry::with_exit(&full_cmd, force, "allow", "force", code),
                &cfg.audit_log,
                &cfg.log_format,
            );
            ExitCode::from(code as u8)
        }
        policy::Verdict::Prompt => match interact::confirm(&cmd_display, &args_display) {
            Ok(interact::Confirm::Yes) => {
                let code = exec::run_as_root(argv);
                audit::log(
                    &audit::AuditEntry::with_exit(&full_cmd, force, "allow", "user_confirmed", code),
                    &cfg.audit_log,
                    &cfg.log_format,
                );
                ExitCode::from(code as u8)
            }
            Ok(interact::Confirm::No) => {
                audit::log(
                    &audit::AuditEntry::new(&full_cmd, force, "deny", "user_denied"),
                    &cfg.audit_log,
                    &cfg.log_format,
                );
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("rtdo: {}", e);
                audit::log(
                    &audit::AuditEntry::new(&full_cmd, force, "deny", "no_interactive_channel"),
                    &cfg.audit_log,
                    &cfg.log_format,
                );
                ExitCode::from(1)
            }
        },
        policy::Verdict::BlockBlacklist => unreachable!("黑名单已在前面处理"),
    }
}

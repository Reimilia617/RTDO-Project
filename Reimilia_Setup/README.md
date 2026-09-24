# Reimilia_Setup —— RTDO-Project 的接入说明

这个目录让 RTDO-Project 能被 [Reimilia](https://github.com/Reimilia617/Reimilia) 部署。
Reimilia 拉取本项目后会找到本目录，按下面的文件决定怎么装。

## 目录内容

| 文件 | 作用 |
|---|---|
| `Build` | 源码编译安装（Rust + Go），**默认方式**，不依赖是否已发布产物 |
| `Binary` | 预编译二进制安装（可选加速），从 GitHub Release 下载产物 |

两个文件都是 Reimilia 的 md 描述格式，格式说明见
[Reimilia 文档 · md 格式](https://github.com/Reimilia617/Reimilia/blob/master/docs/md-format.md)。

## 怎么部署

在 Reimilia 里（`reimilia` 启动 WebUI，或 `reimilia --cli` 走终端）：

```bash
# 终端：交互选择
reimilia --cli

# 终端：直接指定项目，默认按 INSTALL_ORDER 尝试（当前配置为 Build → Binary）
reimilia --cli RTDO-Project --yes

# 强制源码编译
reimilia --cli RTDO-Project --kind build --yes

# 强制预编译（需要 Release 里已有产物）
reimilia --cli RTDO-Project --kind binary --yes
```

WebUI 里选好「安装方式」后点「部署」即可，右侧会实时滚动日志。

## 两个文件都做了什么

**Build**

1. 复用 Reimilia 已经克隆好的源码（`$REIMILIA_WORKDIR`），不再重复 `git clone`；
2. 检查源码完整性（`Cargo.toml`、`daemon/`）与编译工具链（`cargo`、`go`），
   缺什么就明确报出来，并提示可以改用 `--kind binary`；
3. 通过 `reimilia_elevate ./rtdo.sh --install` 提权一次，
   由 `rtdo.sh` 完成编译、安装、注册 systemd 服务、拦截 sudo；
4. 以普通用户身份校验 `/usr/local/bin/rtdo --version`。

**Binary**

1. 按 `uname -m` 拼出产物名（`rtdo-x86_64-linux` / `rtdo-sudod-x86_64-linux`）；
2. 从 `releases/latest/download/` 下载，失败会提示改用 `--kind build`；
3. 校验下载内容是不是 ELF（防止把 404 错误页当二进制装进去）；
4. 同样交给 `rtdo.sh --install --prebuilt-dir <目录>` 完成安装。

## 为什么两个文件都调用 rtdo.sh

安装要动 `/usr/local/bin`（setuid root）、`/etc/pam.d`、`/etc/systemd/system`，
还要把 `/usr/bin/sudo` 换成 shim —— 这套逻辑在 `rtdo.sh` 里已经写好且有
`--uninstall` 对应还原。如果 Reimilia 的安装脚本自己再实现一遍，
两份逻辑迟早会走偏。所以：

* `Build` = 让 `rtdo.sh` 自己编译；
* `Binary` = 把产物准备好，用 `rtdo.sh --prebuilt-dir` 跳过编译。

安装逻辑始终只有一份。

## 注意：提权只能做一次

`rtdo.sh --install` 会把 `/usr/bin/sudo` 替换成 rtdo 的 shim（原版移到
`/usr/bin/sudo.real`）。**安装完成之后再调用 `sudo` 就会被 rtdo 自己拦截**，
所以两个脚本都只在安装那一步提权一次，之后的校验用普通用户执行。
Reimilia 侧也做了对应处理：探测提权工具时会识别 `/usr/bin/sudo.real`
是否存在，存在则优先用 `sudo-force`。

需要 root 的步骤在 WebUI 模式下会在**启动 Reimilia 的那个终端**里提示输入密码
（`sudo` 直接读写 `/dev/tty`）。如果没有终端，请改用 `reimilia --cli`。

## 卸载 / 重装

```bash
# 卸载（会完整还原 sudo 为可用状态）
sudo ./rtdo.sh --uninstall

# 或者已经被拦截、sudo 用不了时：
sudo-force ./rtdo.sh --uninstall
```

## 关于签名

Reimilia README 里描述的 GPG 签名校验机制**目前还没有实现**（Beta 阶段，
私钥也没有合适的存放位置），所以本目录暂时不提供 `.sig` 文件，
Reimilia 也不会因为没有签名而拒绝部署。等 Reimilia 真正实现之后，
本目录需要补上 `Build.sig` 与 `Binary.sig`。

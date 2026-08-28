// rtdo-sudod — rtdo sudo 拦截守护进程 + sudo/sudo-force shim。
//
// 两种运行模式（同一二进制）：
//   * `rtdo-sudod serve`：守护进程。监听 Unix socket `/run/rtdo/sudo.sock`，
//     由 systemd 服务 rtdo-sudod.service 管理；记录拦截/放行日志（stderr → journal）。
//   * 被调用为 `sudo` / `sudo-force`（argv[0] 判断）：shim。
//       - sudo：向守护进程查询，默认被拦截并提示改用 rtdo（fail-closed）；
//       - sudo-force：通知守护进程后直接执行原版 sudo（/usr/bin/sudo.real）。
//
// 协议（换行分隔文本）：
//   "check"            -> "block <message>"（默认拦截）
//   "force <cmdline>"  -> "allow"（sudo-force，仅记录日志）
//   "status"           -> "enabled"
//
// 构建：CGO_ENABLED=0 go build -trimpath -o rtdo-sudod .

package main

import (
	"bufio"
	"fmt"
	"log"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const (
	sockDir  = "/run/rtdo"
	sockPath = "/run/rtdo/sudo.sock"

	// 原版 sudo 被拦截后移动到的位置（由 rtdo.sh / rtdo 安装时设置）。
	realSudo = "/usr/bin/sudo.real"

	// 拦截提示（中英双语一行）。
	blockMsg = "sudo 已被 rtdo 接管：请使用 rtdo <命令> 执行；确需原版 sudo 请用 sudo-force <命令> / sudo is managed by rtdo: use `rtdo <cmd>`, or `sudo-force <cmd>` for the original sudo."
)

func main() {
	if len(os.Args) > 1 && os.Args[1] == "serve" {
		serve()
		return
	}
	shim()
}

// serve：守护进程模式（systemd Type=simple）。
func serve() {
	if err := os.MkdirAll(sockDir, 0o755); err != nil {
		log.Fatalf("cannot create %s: %v", sockDir, err)
	}
	_ = os.Remove(sockPath)
	l, err := net.Listen("unix", sockPath)
	if err != nil {
		log.Fatalf("listen %s: %v", sockPath, err)
	}
	defer l.Close()
	// 任何用户（shim 以调用者身份运行）都能连接查询
	_ = os.Chmod(sockPath, 0o666)
	log.Printf("rtdo-sudod listening on %s", sockPath)

	for {
		c, err := l.Accept()
		if err != nil {
			log.Printf("accept: %v", err)
			continue
		}
		go handleConn(c)
	}
}

func handleConn(c net.Conn) {
	defer c.Close()
	sc := bufio.NewScanner(c)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		switch {
		case line == "check":
			log.Printf("intercept: sudo usage blocked")
			fmt.Fprintf(c, "block %s\n", blockMsg)
		case strings.HasPrefix(line, "force "):
			log.Printf("sudo-force allowed: %s", strings.TrimPrefix(line, "force "))
			fmt.Fprintln(c, "allow")
		case line == "status":
			fmt.Fprintln(c, "enabled")
		default:
			fmt.Fprintf(c, "block %s\n", blockMsg)
		}
	}
}

// shim：被调用为 sudo / sudo-force 时的行为。
func shim() {
	name := filepath.Base(os.Args[0])
	args := os.Args[1:]
	isForce := name == "sudo-force"

	conn, err := net.Dial("unix", sockPath)
	if err != nil {
		// 守护进程不可用：fail-closed（sudo 已禁用，一律拦截提示）
		os.Stderr.WriteString(blockMsg + "\n")
		os.Exit(1)
	}
	defer conn.Close()

	if isForce {
		fmt.Fprintf(conn, "force %s\n", strings.Join(args, " "))
		execRealSudo(args)
	}

	fmt.Fprintln(conn, "check")
	reply, _ := bufio.NewReader(conn).ReadString('\n')
	if strings.HasPrefix(reply, "allow") {
		execRealSudo(args)
	}
	msg := strings.TrimSpace(strings.TrimPrefix(reply, "block "))
	if msg == "" {
		msg = blockMsg
	}
	os.Stderr.WriteString(msg + "\n")
	os.Exit(1)
}

// execRealSudo：执行原版 sudo（sudo.real），继承 stdio。
func execRealSudo(args []string) {
	if _, err := os.Stat(realSudo); err != nil {
		os.Stderr.WriteString(fmt.Sprintf(
			"rtdo: 未找到原版 sudo（%s）。请先安装 sudo 或运行 rtdo.sh --uninstall 还原。\n",
			realSudo,
		))
		os.Exit(1)
	}
	cmd := exec.Command(realSudo, args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	code := 1
	if err := cmd.Run(); err != nil {
		if ee, ok := err.(*exec.ExitError); ok {
			code = ee.ExitCode()
		}
	}
	os.Exit(code)
}

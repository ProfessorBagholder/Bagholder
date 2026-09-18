//go:build windows

package app

import "os/exec"

func detachProcess(cmd *exec.Cmd) {}

func terminateProcess(cmd *exec.Cmd) {
	if cmd != nil && cmd.Process != nil {
		_ = cmd.Process.Kill()
	}
}

func isDarwin() bool { return false }

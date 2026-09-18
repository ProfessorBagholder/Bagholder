//go:build !windows

package notify

func windowsRegister(icon string) error { return nil }

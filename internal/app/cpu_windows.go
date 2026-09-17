//go:build windows

package app

import (
	"syscall"
	"time"
)

func cpuClock() float64 {
	var creation, exit, kernel, user syscall.Filetime
	h, err := syscall.GetCurrentProcess()
	if err != nil {
		return float64(time.Now().UnixNano()) / 1e9
	}
	if err := syscall.GetProcessTimes(h, &creation, &exit, &kernel, &user); err != nil {
		return float64(time.Now().UnixNano()) / 1e9
	}
	return float64(kernel.Nanoseconds()+user.Nanoseconds()) / 1e9
}

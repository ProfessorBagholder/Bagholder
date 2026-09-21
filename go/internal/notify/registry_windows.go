//go:build windows

package notify

import (
	"os"

	"golang.org/x/sys/windows/registry"
)

func windowsRegister(icon string) error {
	key, _, err := registry.CreateKey(registry.CURRENT_USER, `Software\Classes\AppUserModelId\`+WindowsAppID, registry.SET_VALUE)
	if err != nil {
		return err
	}
	defer key.Close()
	if err := key.SetStringValue("DisplayName", AppName); err != nil {
		return err
	}
	if _, err := os.Stat(icon); err == nil {
		if err := key.SetStringValue("IconUri", icon); err != nil {
			return err
		}
	}
	return nil
}

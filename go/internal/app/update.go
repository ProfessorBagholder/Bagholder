package app

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

var versionRE = regexp.MustCompile(`^v?(\d+)\.(\d+)\.(\d+)$`)

func ParseVersion(tag string) ([3]int, bool) {
	m := versionRE.FindStringSubmatch(strings.TrimSpace(tag))
	if m == nil {
		return [3]int{}, false
	}
	var out [3]int
	for i := 0; i < 3; i++ {
		out[i], _ = strconv.Atoi(m[i+1])
	}
	return out, true
}

func versionNewer(a, b [3]int) bool {
	for i := 0; i < 3; i++ {
		if a[i] != b[i] {
			return a[i] > b[i]
		}
	}
	return false
}

func BinaryAssetName(tag string) string {
	name := "bagholder-" + tag + "-" + runtime.GOOS + "-" + runtime.GOARCH
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	return name
}

func releaseAssets(rel map[string]any) map[string]any {
	tag := py.S(rel["tag_name"])
	found := map[string]string{}
	assets, _ := rel["assets"].([]any)
	for _, raw := range assets {
		a, _ := raw.(map[string]any)
		found[py.S(a["name"])] = py.S(a["browser_download_url"])
	}
	stem := BinaryAssetName(tag)
	if found[stem] != "" && found[stem+".sha256"] != "" {
		return map[string]any{"binary": found[stem], "sha": found[stem+".sha256"]}
	}
	return map[string]any{}
}

func (a *App) checkForUpdate(now time.Time) map[string]any {
	if now.IsZero() {
		now = time.Now().UTC()
	}
	record := map[string]any{"checkedAt": py.Stamp(now), "ok": false, "latest": "", "url": RepoURL + "/releases/latest", "updateAvailable": false}
	rel := a.ws.HTTPJSON(http.MethodGet, ReleaseURL, nil, map[string]string{"Accept": "application/vnd.github+json", "User-Agent": "Bagholder/" + AppVersion}, 30*time.Second)
	if rel != nil && rel["_http_status"] == nil {
		if latest, ok := ParseVersion(py.S(rel["tag_name"])); ok {
			current, _ := ParseVersion(AppVersion)
			record["ok"] = true
			record["latest"] = py.S(rel["tag_name"])
			if u := py.S(rel["html_url"]); u != "" {
				record["url"] = u
			}
			record["updateAvailable"] = versionNewer(latest, current)
			record["assets"] = releaseAssets(rel)
			if truthy(record["updateAvailable"]) {
				body := "Update from the header."
				if a.cfg.UpdatesOff {
					body = "Pull the new image."
				}
				a.notify.Emit("updates", "update:"+py.S(record["latest"]), "Bagholder "+py.S(record["latest"])+" is available", body, nil)
			}
		}
	}
	if raw, err := json.Marshal(record); err == nil {
		a.st.SetMeta("update_check", string(raw))
	}
	return record
}

func (a *App) updateStatus() map[string]any {
	raw := a.st.GetMeta("update_check")
	if raw == "" {
		return map[string]any{}
	}
	var rec map[string]any
	if err := json.Unmarshal([]byte(raw), &rec); err != nil || rec == nil {
		return map[string]any{}
	}
	return rec
}

func (a *App) updateMode() string {
	if st, err := os.Stat(filepath.Join(a.cfg.AppDir, ".git")); err == nil && st != nil && which("git") != "" {
		return "git"
	}
	return "release"
}

func (a *App) git(args ...string) (string, string, int, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = a.cfg.AppDir
	var out, errb strings.Builder
	cmd.Stdout, cmd.Stderr = &out, &errb
	done := make(chan error, 1)
	if err := cmd.Start(); err != nil {
		return "", "", -1, err
	}
	go func() { done <- cmd.Wait() }()
	select {
	case err := <-done:
		code := 0
		if err != nil {
			var ee *exec.ExitError
			if errors.As(err, &ee) {
				code = ee.ExitCode()
			} else {
				return out.String(), errb.String(), -1, err
			}
		}
		return out.String(), errb.String(), code, nil
	case <-time.After(120 * time.Second):
		_ = cmd.Process.Kill()
		return out.String(), errb.String(), -1, errors.New("git timed out")
	}
}

func (a *App) gitUpdateReady() (bool, string) {
	out, _, _, err := a.git("status", "--porcelain")
	if err != nil {
		return false, "git: " + err.Error()
	}
	if strings.TrimSpace(out) != "" {
		return false, "This copy is a git checkout with local changes; pull it yourself."
	}
	out, _, _, err = a.git("rev-parse", "--abbrev-ref", "HEAD")
	if err != nil {
		return false, "git: " + err.Error()
	}
	if strings.TrimSpace(out) != "master" {
		return false, "This copy is a git checkout on another branch; pull it yourself."
	}
	if which("go") == "" {
		return false, "This copy is a git checkout without the Go toolchain; build it yourself."
	}
	return true, ""
}

func (a *App) canUpdate(rec map[string]any) bool {
	if rec == nil {
		rec = a.updateStatus()
	}
	if !truthy(rec["updateAvailable"]) || a.cfg.UpdatesOff {
		return false
	}
	if a.updateMode() == "git" {
		ok, _ := a.gitUpdateReady()
		return ok
	}
	assets, _ := rec["assets"].(map[string]any)
	return len(assets) > 0
}

func (a *App) download(rawURL, dest string, maxBytes int64) error {
	req, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return err
	}
	req.Header.Set("User-Agent", "Bagholder/"+AppVersion)
	req.Header.Set("Accept", "application/octet-stream")
	resp, err := a.ws.Do(req, 0)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
		return errors.New("HTTP " + strconv.Itoa(resp.StatusCode))
	}
	f, err := os.Create(dest)
	if err != nil {
		return err
	}
	defer f.Close()
	n, err := io.Copy(f, io.LimitReader(resp.Body, maxBytes+1))
	if err != nil {
		return err
	}
	if n > maxBytes {
		return errors.New("release archive is larger than expected")
	}
	return nil
}

func (a *App) previousPath() string {
	return filepath.Join(a.cfg.Home, "previous", filepath.Base(a.cfg.Exe))
}

func copyFile(src, dst string, mode os.FileMode) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
		return err
	}
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, mode)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}
	return out.Close()
}

func (a *App) checkBinary(path, tag, what string) error {
	_ = os.Chmod(path, 0o755)
	cmd := exec.Command(path, "--version")
	out, err := cmd.Output()
	if err != nil {
		return errors.New("the " + what + " did not run")
	}
	got := strings.TrimSpace(string(out))
	if got == "" {
		return errors.New("the " + what + " did not run")
	}
	if tag == "" {
		return nil
	}
	want := strings.TrimPrefix(tag, "v")
	if !strings.HasSuffix(got, want) {
		return errors.New("the " + what + " reports version " + got + ", not " + want)
	}
	return nil
}

func (a *App) installBinary(staged, tag string) error {
	previous := a.previousPath()
	_ = os.RemoveAll(filepath.Dir(previous))
	if err := copyFile(a.cfg.Exe, previous, 0o755); err != nil {
		return err
	}
	if err := replaceExecutable(staged, a.cfg.Exe); err != nil {
		a.rollback()
		return err
	}
	return os.WriteFile(filepath.Join(a.cfg.Home, "update-pending"), []byte(tag), 0o644)
}

func replaceExecutable(staged, target string) error {
	_ = os.Chmod(staged, 0o755)
	if isWindows() {
		old := target + ".old"
		_ = os.Remove(old)
		if err := os.Rename(target, old); err != nil {
			return err
		}
		if err := os.Rename(staged, target); err != nil {
			_ = os.Rename(old, target)
			return err
		}
		return nil
	}
	return os.Rename(staged, target)
}

func (a *App) rollback() bool {
	previous := a.previousPath()
	if !isFile(previous) {
		return false
	}
	if isWindows() {
		_ = os.Rename(a.cfg.Exe, a.cfg.Exe+".old")
	}
	if err := copyFile(previous, a.cfg.Exe, 0o755); err != nil {
		return false
	}
	_ = os.RemoveAll(filepath.Dir(previous))
	return true
}

func (a *App) requestRestart() {
	a.mu.Lock()
	a.exitCode = RestartCode
	a.mu.Unlock()
	go func() {
		time.Sleep(500 * time.Millisecond)
		a.setStop()
		a.shutdownServer()
	}()
}

func (a *App) setUpdating(msg string) {
	a.mu.Lock()
	a.state.updating = msg
	a.mu.Unlock()
}

func (a *App) performUpdate(tag string, rec map[string]any) {
	err := func() error {
		staging := filepath.Join(a.cfg.Home, "staging")
		_ = os.RemoveAll(staging)
		if err := os.MkdirAll(staging, 0o755); err != nil {
			return err
		}
		defer os.RemoveAll(staging)
		staged := filepath.Join(staging, filepath.Base(a.cfg.Exe))
		check, what := tag, "downloaded release"
		if a.updateMode() == "git" {
			check, what = "", "build"
		}
		if check == "" {
			a.setUpdating("Updating to " + tag + "…")
			ok, why := a.gitUpdateReady()
			if !ok {
				return errors.New(why)
			}
			out, errb, code, err := a.git("pull", "--ff-only")
			if err != nil {
				return errors.New("git pull failed: " + err.Error())
			}
			if code != 0 {
				msg := strings.TrimSpace(errb)
				if msg == "" {
					msg = strings.TrimSpace(out)
				}
				return errors.New("git pull failed: " + cutStr(msg, 200))
			}
			a.setUpdating("Building " + tag + "…")
			build := exec.Command("go", "build", "-o", staged, "./cmd/bagholder")
			build.Dir = a.cfg.AppDir
			if outb, err := build.CombinedOutput(); err != nil {
				return errors.New("go build failed: " + cutStr(strings.TrimSpace(string(outb)), 200))
			}
		} else {
			assets, _ := rec["assets"].(map[string]any)
			if len(assets) == 0 {
				return errors.New("This release has no downloadable binary for " + runtime.GOOS + "/" + runtime.GOARCH + ".")
			}
			a.setUpdating("Downloading " + tag + "…")
			shaPath := filepath.Join(staging, "release.sha256")
			if err := a.download(py.S(assets["binary"]), staged, UpdateMaxBytes); err != nil {
				return err
			}
			if err := a.download(py.S(assets["sha"]), shaPath, 4096); err != nil {
				return err
			}
			shaText, err := os.ReadFile(shaPath)
			if err != nil {
				return err
			}
			want := strings.ToLower(strings.TrimSpace(strings.Fields(string(shaText) + " ")[0]))
			data, err := os.ReadFile(staged)
			if err != nil {
				return err
			}
			sum := sha256.Sum256(data)
			if want != hex.EncodeToString(sum[:]) {
				return errors.New("The download did not match the release's checksum.")
			}
		}
		a.setUpdating("Installing " + tag + "…")
		if err := a.checkBinary(staged, check, what); err != nil {
			return err
		}
		return a.installBinary(staged, tag)
	}()
	if err != nil {
		a.mu.Lock()
		a.state.updating = ""
		a.state.updateError = "Update failed: " + err.Error()
		a.mu.Unlock()
		a.logf("bagholder update failed: %s\n", err.Error())
		return
	}
	a.setUpdating("Restarting…")
	a.logf("bagholder update: %s installed, restarting\n", tag)
	a.requestRestart()
}

func (a *App) startUpdate() map[string]any {
	if a.cfg.UpdatesOff {
		return map[string]any{"ok": false, "error": UpdatesOffMessage}
	}
	rec := a.updateStatus()
	a.mu.Lock()
	if a.state.updating != "" {
		a.mu.Unlock()
		return map[string]any{"ok": true}
	}
	if a.state.syncing {
		a.mu.Unlock()
		return map[string]any{"ok": false, "error": "Wait for the sync to finish, then update."}
	}
	latest := py.S(rec["latest"])
	if !truthy(rec["updateAvailable"]) || latest == "" {
		a.mu.Unlock()
		return map[string]any{"ok": false, "error": "No update to install."}
	}
	if !a.canUpdate(rec) {
		a.mu.Unlock()
		if a.updateMode() == "git" {
			_, why := a.gitUpdateReady()
			return map[string]any{"ok": false, "error": why}
		}
		return map[string]any{"ok": false, "error": "This release has no downloadable binary for " + runtime.GOOS + "/" + runtime.GOARCH + "."}
	}
	a.state.updateError = ""
	a.state.updating = "Updating to " + latest + "…"
	a.mu.Unlock()
	go a.performUpdate(latest, rec)
	return map[string]any{"ok": true}
}

func (a *App) checkForUpdateIfDue(now time.Time) map[string]any {
	if now.IsZero() {
		now = time.Now().UTC()
	}
	rec := a.updateStatus()
	if last, ok := parseStamp(py.S(rec["checkedAt"])); ok && now.Sub(last) < UpdateCheckHours*time.Hour {
		return rec
	}
	return a.checkForUpdate(now)
}

func Supervise(home string, healthySec int) int {
	exe, err := os.Executable()
	if err != nil {
		return 1
	}
	marker := filepath.Join(home, "update-pending")
	sigs := make(chan os.Signal, 2)
	signal.Notify(sigs, os.Interrupt, syscall.SIGTERM)
	defer signal.Stop(sigs)
	for {
		child := exec.Command(exe, os.Args[1:]...)
		child.Env = append(os.Environ(), "BAGHOLDER_CHILD=1")
		child.Stdin, child.Stdout, child.Stderr = os.Stdin, os.Stdout, os.Stderr
		if err := child.Start(); err != nil {
			return 1
		}
		_, statErr := os.Stat(marker)
		pending := statErr == nil
		done := make(chan error, 1)
		go func() { done <- child.Wait() }()
		var waitErr error
		interrupted := false
		if pending {
			select {
			case waitErr = <-done:
			case <-time.After(time.Duration(healthySec) * time.Second):
				_ = os.Remove(marker)
				_ = os.RemoveAll(filepath.Join(home, "previous"))
				pending = false
				select {
				case waitErr = <-done:
				case <-sigs:
					interrupted = true
				}
			case <-sigs:
				interrupted = true
			}
		} else {
			select {
			case waitErr = <-done:
			case <-sigs:
				interrupted = true
			}
		}
		if interrupted {
			terminateProcess(child)
			select {
			case <-done:
			case <-time.After(10 * time.Second):
			}
			return 0
		}
		code := 0
		if waitErr != nil {
			var ee *exec.ExitError
			if errors.As(waitErr, &ee) {
				code = ee.ExitCode()
			} else {
				code = 1
			}
		}
		if code == RestartCode {
			continue
		}
		if pending && code != 0 {
			_ = os.Remove(marker)
			previous := filepath.Join(home, "previous", filepath.Base(exe))
			if isFile(previous) {
				if isWindows() {
					_ = os.Rename(exe, exe+".old")
				}
				if copyFile(previous, exe, 0o755) == nil {
					_ = os.RemoveAll(filepath.Join(home, "previous"))
					os.Stderr.WriteString("bagholder update: the new version did not start; the previous one is back\n")
					continue
				}
			}
		}
		return code
	}
}

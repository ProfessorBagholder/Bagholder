package notify

import (
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var Kinds = []string{"fills", "problems", "connection", "updates", "releases", "disclosures"}
var ReleaseScopes = []string{"releasesHeld", "releasesWatched", "releasesAll"}
var DisclosureScopes = []string{"disclosuresHeld", "disclosuresWatched", "disclosuresAll"}
var SettingKeys = append(append([]string{"fills", "problems", "connection", "updates"}, ReleaseScopes...), DisclosureScopes...)

const (
	SettingsKey  = "notify_settings"
	HeartbeatSec = 15.0
	ModeEnv      = "BAGHOLDER_NOTIFY"
	AppName      = "Bagholder"
	MacBundleID  = "com.bagholder.notifier"
	Watermark    = "notify_seen:"
)

type job struct {
	row     store.Notification
	channel string
}

type Notifier struct {
	Store   *store.Store
	URL     string
	Icon    string
	mu      sync.Mutex
	cond    *sync.Cond
	queue   chan job
	started bool
	winReg  bool
}

func New(st *store.Store) *Notifier {
	n := &Notifier{Store: st, URL: "http://127.0.0.1:8765/", queue: make(chan job, 256)}
	n.cond = sync.NewCond(&n.mu)
	return n
}

func (n *Notifier) Configure(url, icon string) {
	if url != "" {
		n.URL = url
	}
	if icon != "" {
		n.Icon = icon
	}
}

func (n *Notifier) Settings() map[string]bool {
	raw := map[string]any{}
	if v := n.Store.GetMeta(SettingsKey); v != "" {
		var parsed any
		if json.Unmarshal([]byte(v), &parsed) == nil {
			if m, ok := parsed.(map[string]any); ok {
				raw = m
			}
		}
	}
	out := map[string]bool{}
	for _, k := range SettingKeys {
		out[k] = truthy(raw[k])
	}
	return out
}

func truthy(v any) bool {
	switch x := v.(type) {
	case nil:
		return false
	case bool:
		return x
	case float64:
		return x != 0
	case string:
		return x != ""
	case []any:
		return len(x) > 0
	case map[string]any:
		return len(x) > 0
	}
	return true
}

func (n *Notifier) DisclosureScopes() map[string]bool {
	on := n.Settings()
	out := map[string]bool{}
	for _, k := range DisclosureScopes {
		if on[k] {
			out[strings.ToLower(k[len("disclosures"):])] = true
		}
	}
	return out
}

func (n *Notifier) ReleaseScopes() map[string]bool {
	on := n.Settings()
	out := map[string]bool{}
	for _, k := range ReleaseScopes {
		if on[k] {
			out[strings.ToLower(k[len("releases"):])] = true
		}
	}
	return out
}

func (n *Notifier) KindOn(kind string) bool {
	if kind == "disclosures" {
		return len(n.DisclosureScopes()) > 0
	}
	if kind == "releases" {
		return len(n.ReleaseScopes()) > 0
	}
	return n.Settings()[kind]
}

func (n *Notifier) SetSettings(patch map[string]any) map[string]bool {
	cur := n.Settings()
	for k, v := range patch {
		if b, ok := v.(bool); ok && py.Contains(SettingKeys, k) {
			cur[k] = b
		}
	}
	b, _ := json.Marshal(cur)
	n.Store.SetMeta(SettingsKey, string(b))
	return cur
}

func (n *Notifier) Status() map[string]any {
	out := map[string]any{}
	for k, v := range n.Settings() {
		out[k] = v
	}
	out["native"] = n.NativeChannel()
	out["unread"] = n.Store.UnreadNotifications()
	return out
}

func which(name string) string {
	p, err := exec.LookPath(name)
	if err != nil {
		return ""
	}
	return p
}

func (n *Notifier) NativeChannel() string {
	mode := strings.ToLower(strings.TrimSpace(os.Getenv(ModeEnv)))
	if mode == "browser" || mode == "off" || mode == "0" || mode == "none" {
		return ""
	}
	switch runtime.GOOS {
	case "darwin":
		if which("osascript") != "" {
			return "mac"
		}
		return ""
	case "windows":
		if which("powershell") != "" || which("pwsh") != "" {
			return "windows"
		}
		return ""
	}
	if which("notify-send") != "" && (os.Getenv("DISPLAY") != "" || os.Getenv("WAYLAND_DISPLAY") != "") {
		return "linux"
	}
	return ""
}

type Stamped[T any] struct {
	When  string
	Ident string
	Item  T
}

func FreshSince[T any](st *store.Store, stream string, items []T, at func(T) string, ident func(T) string, seen func(T) bool) []T {
	key := Watermark + stream
	raw := st.GetMeta(key)
	mark, shownRaw, _ := strings.Cut(raw, "|")
	shown := map[string]bool{}
	for _, x := range strings.Split(shownRaw, ",") {
		if x != "" {
			shown[x] = true
		}
	}
	stamped := make([]Stamped[T], 0, len(items))
	newest := ""
	for _, i := range items {
		s := Stamped[T]{When: at(i), Ident: ident(i), Item: i}
		stamped = append(stamped, s)
		if s.When > newest {
			newest = s.When
		}
	}
	remember := func(top string) {
		atTop := map[string]bool{}
		for _, s := range stamped {
			if s.When == top {
				atTop[s.Ident] = true
			}
		}
		if top == mark {
			for k := range shown {
				atTop[k] = true
			}
		}
		ids := make([]string, 0, len(atTop))
		for k := range atTop {
			ids = append(ids, k)
		}
		sort.Strings(ids)
		st.SetMeta(key, top+"|"+strings.Join(ids, ","))
	}
	if raw == "" {
		if newest != "" {
			remember(newest)
		}
		return []T{}
	}
	out := []T{}
	for _, s := range stamped {
		if (s.When > mark || (s.When == mark && !shown[s.Ident])) && !(seen != nil && seen(s.Item)) {
			out = append(out, s.Item)
		}
	}
	if len(out) > 0 || newest > mark {
		top := newest
		if mark > top {
			top = mark
		}
		remember(top)
	}
	return out
}

func (n *Notifier) Emit(kind, key, title, body string, extra map[string]any) *store.Notification {
	if !py.Contains(Kinds, kind) || !n.KindOn(kind) {
		return nil
	}
	return n.post(kind, key, title, body, extra)
}

func (n *Notifier) TestNotification() *store.Notification {
	stamp := time.Now().UTC().Format("20060102150405.000000")
	stamp = strings.Replace(stamp, ".", "", 1)
	return n.post("test", "test:"+stamp, AppName, "Notifications reach you here.", nil)
}

func (n *Notifier) post(kind, key, title, body string, extra map[string]any) *store.Notification {
	channel := n.NativeChannel()
	row := n.Store.AddNotification(kind, key, title, body, extra, channel != "")
	if row == nil {
		return nil
	}
	if channel != "" {
		n.enqueue(*row, channel)
	}
	n.mu.Lock()
	n.cond.Broadcast()
	n.mu.Unlock()
	return row
}

func (n *Notifier) Wake() {
	n.mu.Lock()
	n.cond.Broadcast()
	n.mu.Unlock()
}

func (n *Notifier) enqueue(row store.Notification, channel string) {
	n.mu.Lock()
	if !n.started {
		n.started = true
		go n.work()
	}
	n.mu.Unlock()
	n.queue <- job{row, channel}
}

func (n *Notifier) work() {
	for j := range n.queue {
		ok := false
		func() {
			defer func() {
				if e := recover(); e != nil {
					fmt.Fprintf(os.Stderr, "bagholder notify: %v\n", e)
				}
			}()
			ok = n.Deliver(j.channel, j.row.Title, j.row.Body)
		}()
		if !ok {
			fmt.Fprintf(os.Stderr, "bagholder notify: %s not shown (%s)\n", j.row.Title, j.channel)
		}
	}
}

func (n *Notifier) Deliver(channel, title, body string) bool {
	switch channel {
	case "mac":
		return n.macDeliver(title, body)
	case "windows":
		return n.windowsDeliver(title, body)
	case "linux":
		return n.linuxDeliver(title, body)
	}
	return false
}

const MacScript = `on run
	set t to system attribute "BAGHOLDER_TITLE"
	if t is "" then
		open location "%s"
	else
		display notification (system attribute "BAGHOLDER_BODY") with title t
	end if
end run
`

func (n *Notifier) macScript() string { return fmt.Sprintf(MacScript, n.URL) }

func (n *Notifier) MacAppPath() string {
	return filepath.Join(n.Store.Home(), AppName+".app")
}

func (n *Notifier) macStamp() string {
	h := sha1.New()
	h.Write([]byte(n.macScript()))
	if data, err := os.ReadFile(n.Icon); err == nil {
		h.Write(data)
	}
	return hex.EncodeToString(h.Sum(nil))
}

func (n *Notifier) MacApp() string {
	app := n.MacAppPath()
	stamp := filepath.Join(app, "Contents", "Resources", "bagholder.stamp")
	want := n.macStamp()
	if _, err := os.Stat(filepath.Join(app, "Contents", "MacOS", "applet")); err == nil {
		if data, err := os.ReadFile(stamp); err == nil && string(data) == want {
			return app
		}
	}
	built, err := n.macBuild(app, want)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder notify: the notifier app could not be built: %s\n", err)
		return ""
	}
	return built
}

func run(timeout time.Duration, env []string, name string, args ...string) ([]byte, error) {
	cmd := exec.Command(name, args...)
	if env != nil {
		cmd.Env = append(os.Environ(), env...)
	}
	done := make(chan struct{})
	var out []byte
	var err error
	go func() {
		out, err = cmd.CombinedOutput()
		close(done)
	}()
	select {
	case <-done:
		return out, err
	case <-time.After(timeout):
		if cmd.Process != nil {
			cmd.Process.Kill()
		}
		<-done
		return out, fmt.Errorf("timed out")
	}
}

func (n *Notifier) macBuild(app, stamp string) (string, error) {
	if which("osacompile") == "" {
		return "", nil
	}
	work, err := os.MkdirTemp("", "bagholder-notifier-")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(work)
	script := filepath.Join(work, "notifier.applescript")
	if err := os.WriteFile(script, []byte(n.macScript()), 0o644); err != nil {
		return "", err
	}
	built := filepath.Join(work, AppName+".app")
	if out, err := run(60*time.Second, nil, "osacompile", "-o", built, script); err != nil {
		return "", fmt.Errorf("osacompile: %s", strings.TrimSpace(string(out)))
	}
	plist := filepath.Join(built, "Contents", "Info.plist")
	if _, err := run(30*time.Second, nil, "plutil", "-replace", "CFBundleIdentifier", "-string", MacBundleID, plist); err != nil {
		return "", err
	}
	if _, err := run(30*time.Second, nil, "plutil", "-replace", "CFBundleDisplayName", "-string", AppName, plist); err != nil {
		return "", err
	}
	if icns := n.macIcon(work); icns != "" {
		res := filepath.Join(built, "Contents", "Resources")
		copyFile(icns, filepath.Join(res, "applet.icns"))
		os.Remove(filepath.Join(res, "Assets.car"))
		run(30*time.Second, nil, "plutil", "-remove", "CFBundleIconName", plist)
	}
	if err := os.WriteFile(filepath.Join(built, "Contents", "Resources", "bagholder.stamp"), []byte(stamp), 0o644); err != nil {
		return "", err
	}
	if which("codesign") != "" {
		run(60*time.Second, nil, "codesign", "--force", "--sign", "-", built)
	}
	os.RemoveAll(app)
	os.MkdirAll(filepath.Dir(app), 0o755)
	if err := os.Rename(built, app); err != nil {
		if err := copyTree(built, app); err != nil {
			return "", err
		}
	}
	return app, nil
}

func copyFile(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.Create(dst)
	if err != nil {
		return err
	}
	defer out.Close()
	_, err = io.Copy(out, in)
	return err
}

func copyTree(src, dst string) error {
	return filepath.Walk(src, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(src, path)
		target := filepath.Join(dst, rel)
		if info.IsDir() {
			return os.MkdirAll(target, info.Mode())
		}
		if err := copyFile(path, target); err != nil {
			return err
		}
		return os.Chmod(target, info.Mode())
	})
}

func (n *Notifier) macIcon(work string) string {
	if which("sips") == "" || which("iconutil") == "" {
		return ""
	}
	if _, err := os.Stat(n.Icon); err != nil {
		return ""
	}
	iconset := filepath.Join(work, "icon.iconset")
	os.Mkdir(iconset, 0o755)
	sizes := []struct {
		size  int
		names []string
	}{{16, []string{"icon_16x16.png"}}, {32, []string{"icon_16x16@2x.png", "icon_32x32.png"}}, {64, []string{"icon_32x32@2x.png"}},
		{128, []string{"icon_128x128.png"}}, {256, []string{"icon_128x128@2x.png", "icon_256x256.png"}}, {512, []string{"icon_256x256@2x.png", "icon_512x512.png"}}}
	for _, s := range sizes {
		first := filepath.Join(iconset, s.names[0])
		if _, err := run(30*time.Second, nil, "sips", "-z", strconv.Itoa(s.size), strconv.Itoa(s.size), n.Icon, "--out", first); err != nil {
			return ""
		}
		for _, other := range s.names[1:] {
			copyFile(first, filepath.Join(iconset, other))
		}
	}
	icns := filepath.Join(work, "icon.icns")
	if _, err := run(30*time.Second, nil, "iconutil", "-c", "icns", iconset, "-o", icns); err != nil {
		return ""
	}
	if _, err := os.Stat(icns); err != nil {
		return ""
	}
	return icns
}

func (n *Notifier) macDeliver(title, body string) bool {
	if app := n.MacApp(); app != "" {
		out, err := run(60*time.Second, nil, "open", "-n", "-W", "--env", "BAGHOLDER_TITLE="+title, "--env", "BAGHOLDER_BODY="+body, app)
		if err == nil {
			return true
		}
		msg := strings.TrimSpace(string(out))
		if msg == "" {
			msg = err.Error()
		}
		fmt.Fprintf(os.Stderr, "bagholder notify: the notifier app refused: %s\n", msg)
	}
	_, err := run(60*time.Second, []string{"BAGHOLDER_TITLE=" + title, "BAGHOLDER_BODY=" + body}, "osascript", "-e", `display notification (system attribute "BAGHOLDER_BODY") with title (system attribute "BAGHOLDER_TITLE")`)
	return err == nil
}

const WindowsAppID = AppName

const WindowsScript = `$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml('<toast activationType="protocol" launch="__URL__"><visual><binding template="ToastGeneric"><text></text><text></text></binding></visual></toast>')
$t = $xml.GetElementsByTagName('text')
$t.Item(0).AppendChild($xml.CreateTextNode($env:BAGHOLDER_TITLE)) | Out-Null
$t.Item(1).AppendChild($xml.CreateTextNode($env:BAGHOLDER_BODY)) | Out-Null
$toast = New-Object Windows.UI.Notifications.ToastNotification $xml
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('__APP__').Show($toast)
`

func (n *Notifier) WindowsScript() string {
	return strings.ReplaceAll(strings.ReplaceAll(WindowsScript, "__URL__", n.URL), "__APP__", WindowsAppID)
}

func (n *Notifier) windowsDeliver(title, body string) bool {
	n.mu.Lock()
	registered := n.winReg
	n.mu.Unlock()
	if !registered {
		if err := windowsRegister(n.Icon); err != nil {
			fmt.Fprintf(os.Stderr, "bagholder notify: app id not registered: %s\n", err)
		} else {
			n.mu.Lock()
			n.winReg = true
			n.mu.Unlock()
		}
	}
	shell := which("powershell")
	if shell == "" {
		shell = which("pwsh")
	}
	if shell == "" {
		shell = "powershell"
	}
	_, err := run(60*time.Second, []string{"BAGHOLDER_TITLE=" + title, "BAGHOLDER_BODY=" + body}, shell, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", n.WindowsScript())
	return err == nil
}

func (n *Notifier) linuxDeliver(title, body string) bool {
	args := []string{"--app-name=" + AppName}
	if _, err := os.Stat(n.Icon); err == nil {
		args = append(args, "--icon="+n.Icon)
	}
	_, err := run(60*time.Second, nil, "notify-send", append(args, title, body)...)
	return err == nil
}

func (n *Notifier) Stream(w io.Writer, flush func(), after *int64, alive func() bool, heartbeat float64) {
	if heartbeat <= 0 {
		heartbeat = HeartbeatSec
	}
	last := n.Store.LatestNotificationID()
	if after != nil {
		last = *after
	}
	if _, err := io.WriteString(w, ": bagholder\n\n"); err != nil {
		return
	}
	flush()
	for alive() {
		n.mu.Lock()
		rows := n.Store.ListNotifications(last, "", false, 0, false)
		if len(rows) == 0 {
			timer := time.AfterFunc(time.Duration(heartbeat*float64(time.Second)), func() {
				n.mu.Lock()
				n.cond.Broadcast()
				n.mu.Unlock()
			})
			n.cond.Wait()
			timer.Stop()
			n.mu.Unlock()
			if _, err := io.WriteString(w, ": ping\n\n"); err != nil {
				return
			}
			flush()
			continue
		}
		n.mu.Unlock()
		for _, r := range rows {
			if r.ID > last {
				last = r.ID
			}
			b, _ := json.Marshal(r)
			if _, err := fmt.Fprintf(w, "id: %d\ndata: %s\n\n", r.ID, b); err != nil {
				return
			}
		}
		flush()
	}
}

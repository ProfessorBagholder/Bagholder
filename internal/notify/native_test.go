package notify

import (
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"testing"
	"time"
)

const stub = `#!/bin/sh
{
printf '%s\000' "$0" "$@"
printf '\001%s\000%s\000\002' "$BAGHOLDER_TITLE" "$BAGHOLDER_BODY"
} >> "$BAGHOLDER_FAKE_LOG"
`

var tools = []string{"open", "osascript", "osacompile", "plutil", "sips", "iconutil", "codesign", "powershell", "notify-send"}

type call struct {
	argv  []string
	title string
	body  string
}

func (c call) tool() string { return filepath.Base(c.argv[0]) }

func (c call) words() []string { return append([]string{c.tool()}, c.argv[1:]...) }

func fakeTools(t *testing.T) string {
	t.Helper()
	if runtime.GOOS == "windows" {
		t.Skip("the native tools are stubbed with shell scripts")
	}
	dir := t.TempDir()
	for _, name := range tools {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(stub), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	log := filepath.Join(dir, "calls.log")
	t.Setenv("BAGHOLDER_FAKE_LOG", log)
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	return log
}

func calls(log string) []call {
	raw, err := os.ReadFile(log)
	if err != nil {
		return nil
	}
	out := []call{}
	for _, rec := range strings.Split(string(raw), "\x02") {
		if rec == "" {
			continue
		}
		argv, env, _ := strings.Cut(rec, "\x01")
		c := call{argv: strings.Split(strings.TrimSuffix(argv, "\x00"), "\x00")}
		if e := strings.Split(strings.TrimSuffix(env, "\x00"), "\x00"); len(e) > 1 {
			c.title, c.body = e[0], e[1]
		}
		out = append(out, c)
	}
	return out
}

func waitCalls(t *testing.T, log string, want int) []call {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for {
		got := calls(log)
		if len(got) >= want || time.Now().After(deadline) {
			return got
		}
		time.Sleep(20 * time.Millisecond)
	}
}

func lastCall(t *testing.T, log string) call {
	t.Helper()
	got := calls(log)
	if len(got) == 0 {
		t.Fatal("nothing was run")
	}
	return got[len(got)-1]
}

func fakeMacApp(t *testing.T, n *Notifier) string {
	t.Helper()
	app := n.MacAppPath()
	for _, d := range []string{filepath.Join(app, "Contents", "MacOS"), filepath.Join(app, "Contents", "Resources")} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(app, "Contents", "MacOS", "applet"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(app, "Contents", "Resources", "bagholder.stamp"), []byte(n.macStamp()), 0o644); err != nil {
		t.Fatal(err)
	}
	return app
}

func favicon(t *testing.T) string {
	t.Helper()
	p, err := filepath.Abs(filepath.Join("..", "..", "favicon.png"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(p); err != nil {
		t.Fatal(err)
	}
	return p
}

func desktop() string {
	switch runtime.GOOS {
	case "darwin":
		return "mac"
	case "windows":
		return "windows"
	}
	return "linux"
}

func delivered(c call) [3]string {
	switch c.tool() {
	case "notify-send":
		return [3]string{"linux", c.argv[len(c.argv)-2], c.argv[len(c.argv)-1]}
	case "open":
		title, body := "", ""
		for _, a := range c.argv {
			if v, ok := strings.CutPrefix(a, "BAGHOLDER_TITLE="); ok {
				title = v
			}
			if v, ok := strings.CutPrefix(a, "BAGHOLDER_BODY="); ok {
				body = v
			}
		}
		return [3]string{"mac", title, body}
	case "osascript":
		return [3]string{"mac", c.title, c.body}
	case "powershell", "pwsh":
		return [3]string{"windows", c.title, c.body}
	}
	return [3]string{"", c.title, c.body}
}

func TestPostedByTheServerARowIsStoredSeenAndHandedToTheSystem(t *testing.T) {
	n, st := setUp(t)
	n.SetSettings(map[string]any{"fills": true})
	log := fakeTools(t)
	fakeMacApp(t, n)
	t.Setenv(ModeEnv, "")
	t.Setenv("DISPLAY", ":0")
	row := n.Emit("fills", "order:1:filled", "Order filled · QNC", "Bought 5 at 1.75", nil)
	got := waitCalls(t, log, 1)
	if row == nil || row.SeenAt == "" {
		t.Fatalf("the server shows it: no page shows it too: %+v", row)
	}
	have := [][3]string{}
	for _, c := range got {
		have = append(have, delivered(c))
	}
	if want := [][3]string{{desktop(), "Order filled · QNC", "Bought 5 at 1.75"}}; !reflect.DeepEqual(have, want) {
		t.Fatalf("calls: %v", have)
	}
	if rows := st.ListNotifications(0, "", true, 0, false); len(rows) != 0 {
		t.Fatalf("nothing left for a page: %+v", rows)
	}
}

func TestEachSystemIsAskedInItsOwnWords(t *testing.T) {
	n, _ := setUp(t)
	n.Configure("http://127.0.0.1:8799/", favicon(t))
	log := fakeTools(t)
	app := fakeMacApp(t, n)
	if !n.Deliver("mac", "Stopped out · QNC", "Sold 5 at 1.64") {
		t.Fatal("mac: not taken")
	}
	want := []string{"open", "-n", "-W", "--env", "BAGHOLDER_TITLE=Stopped out · QNC", "--env", "BAGHOLDER_BODY=Sold 5 at 1.64", app}
	if got := lastCall(t, log).words(); !reflect.DeepEqual(got, want) {
		t.Fatalf("the applet reads its words from the environment; a click on the banner runs it without any and it opens the app: %q", got)
	}
	if !strings.Contains(n.macScript(), `open location "http://127.0.0.1:8799/"`) {
		t.Fatal(n.macScript())
	}
	if err := os.RemoveAll(app); err != nil {
		t.Fatal(err)
	}
	if !n.Deliver("mac", "T", "B") {
		t.Fatal("mac without the applet: not taken")
	}
	if c := lastCall(t, log); c.tool() != "osascript" || c.title != "T" || c.body != "B" {
		t.Fatalf("without the applet, the system's plain notification: %q %q %q", c.words(), c.title, c.body)
	}
	if !n.Deliver("windows", "T", "B") {
		t.Fatal("windows: not taken")
	}
	c := lastCall(t, log)
	cmd := c.words()
	if c.tool() != "powershell" || !reflect.DeepEqual(cmd[1:7], []string{"-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden"}) || c.title != "T" {
		t.Fatalf("windows: %q %q", cmd, c.title)
	}
	if !strings.Contains(cmd[len(cmd)-1], "CreateToastNotifier('Bagholder')") {
		t.Fatal(cmd[len(cmd)-1])
	}
	if !strings.Contains(cmd[len(cmd)-1], `launch="http://127.0.0.1:8799/"`) {
		t.Fatal("a click on the toast opens the app: " + cmd[len(cmd)-1])
	}
	if !n.Deliver("linux", "T", "B") {
		t.Fatal("linux: not taken")
	}
	cmd = lastCall(t, log).words()
	if got := append(append([]string{}, cmd[:2]...), cmd[len(cmd)-2:]...); !reflect.DeepEqual(got, []string{"notify-send", "--app-name=Bagholder", "T", "B"}) {
		t.Fatalf("linux: %q", cmd)
	}
	if !strings.HasPrefix(cmd[2], "--icon=") {
		t.Fatalf("linux icon: %q", cmd)
	}
	if n.Deliver("", "T", "B") {
		t.Fatal("no channel: taken")
	}
}

func TestTheMacAppletIsBuiltOnceUnderBagholdersNameAndIcon(t *testing.T) {
	if runtime.GOOS != "darwin" || which("osacompile") == "" {
		t.Skip("the applet is built with macOS's own tools")
	}
	n, st := setUp(t)
	n.Configure("http://127.0.0.1:8799/", favicon(t))
	app := n.MacApp()
	if app == "" {
		t.Fatal("no applet")
	}
	if want := filepath.Join(st.Home(), "Bagholder.app"); app != want {
		t.Fatalf("app: %q", app)
	}
	out, _ := exec.Command("plutil", "-p", filepath.Join(app, "Contents", "Info.plist")).Output()
	plist := string(out)
	if !strings.Contains(plist, `"CFBundleName" => "Bagholder"`) {
		t.Fatal(plist)
	}
	if !strings.Contains(plist, `"CFBundleIdentifier" => "com.bagholder.notifier"`) {
		t.Fatal(plist)
	}
	if strings.Contains(plist, "CFBundleIconName") {
		t.Fatal("the stock asset catalogue gives way to the app's own icon file")
	}
	res := filepath.Join(app, "Contents", "Resources")
	if icns, err := os.Stat(filepath.Join(res, "applet.icns")); err != nil || icns.Size() <= 10000 {
		t.Fatal("the icon built from the favicon")
	}
	if _, err := os.Stat(filepath.Join(res, "Assets.car")); err == nil {
		t.Fatal("Assets.car")
	}
	if _, err := os.Stat(filepath.Join(res, "Scripts")); err != nil || !strings.Contains(n.macScript(), `open location "http://127.0.0.1:8799/"`) {
		t.Fatal(n.macScript())
	}
	stamp, err := os.ReadFile(filepath.Join(res, "bagholder.stamp"))
	if err != nil {
		t.Fatal(err)
	}
	applet := filepath.Join(app, "Contents", "MacOS", "applet")
	before, err := os.Stat(applet)
	if err != nil {
		t.Fatal(err)
	}
	if again := n.MacApp(); again != app {
		t.Fatalf("already built: not built again: %q", again)
	}
	if after, err := os.Stat(applet); err != nil || !after.ModTime().Equal(before.ModTime()) {
		t.Fatal("already built: not built again")
	}
	n.Configure("http://127.0.0.1:8800/", "")
	if n.macStamp() == string(stamp) {
		t.Fatal("a new address means a new applet")
	}
}

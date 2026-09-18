package app

import (
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

func pageHTML(t *testing.T) string {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "..", "..", "ledger.html"))
	if err != nil {
		t.Fatal(err)
	}
	return string(raw)
}

var scriptRE = regexp.MustCompile(`(?s)<script>(.*?)</script>`)

func TestEveryScriptOnThePageParses(t *testing.T) {
	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("node is needed to parse the page's script")
	}
	scripts := scriptRE.FindAllStringSubmatch(pageHTML(t), -1)
	if len(scripts) == 0 {
		t.Fatal("the page carries its script inline")
	}
	for i, m := range scripts {
		path := filepath.Join(t.TempDir(), "script.js")
		if err := os.WriteFile(path, []byte(m[1]), 0o600); err != nil {
			t.Fatal(err)
		}
		out, err := exec.Command(node, "--check", path).CombinedOutput()
		if err != nil {
			text := string(out)
			if len(text) > 2000 {
				text = text[:2000]
			}
			t.Errorf("script %d does not parse:\n%s", i, text)
		}
	}
}

func TestNoTitleAttributeAnywhereOnThePage(t *testing.T) {
	found := regexp.MustCompile(` title=\\?["']`).FindAllString(pageHTML(t), -1)
	if len(found) != 0 {
		t.Errorf("nothing on the page gets a browser tooltip: %v", found)
	}
}

func TestThePageAndTheServerAgreeOnTheProtocol(t *testing.T) {
	m := regexp.MustCompile(`const PROTOCOL = "([^"]*)"`).FindStringSubmatch(pageHTML(t))
	if m == nil {
		t.Fatal("the page carries no PROTOCOL constant")
	}
	if m[1] != Protocol {
		t.Errorf("ledger.html PROTOCOL = %q, app.Protocol = %q", m[1], Protocol)
	}
}

// The page the binary carries is the page it serves. A file beside the binary must not
// shadow it: that is how a release ends up serving a page its server never shipped with,
// which is the mismatch PROTOCOL exists to catch.
func TestAFileBesideTheBinaryDoesNotShadowTheEmbeddedPage(t *testing.T) {
	a := newTestApp(t)
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "ledger.html"), []byte("<!doctype html>stale"), 0o600); err != nil {
		t.Fatal(err)
	}
	a.cfg.AppDir = dir
	a.staticCache = nil
	if e, ok := a.staticFile("ledger.html"); ok && strings.Contains(string(e.data), "stale") {
		t.Error("a file in the app's own directory was served in place of the embedded page")
	}
	a.cfg.PageDir = dir
	a.staticCache = nil
	e, ok := a.staticFile("ledger.html")
	if !ok || !strings.Contains(string(e.data), "stale") {
		t.Error("BAGHOLDER_PAGE_DIR did not replace the page")
	}
}

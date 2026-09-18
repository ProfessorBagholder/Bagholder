package app

import (
	"encoding/json"
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

// The page the binary carries is the page it serves, and nothing replaces it. A file beside
// the binary must not shadow it: that is how a release ends up serving a page its server never
// shipped with, which is the mismatch PROTOCOL exists to catch.
func TestNothingOnDiskShadowsTheEmbeddedPage(t *testing.T) {
	a := newTestApp(t)
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "ledger.html"), []byte("<!doctype html>stale"), 0o600); err != nil {
		t.Fatal(err)
	}
	a.cfg.AppDir = dir
	a.staticCache = nil
	if e, ok := a.staticFile("ledger.html"); ok && strings.Contains(string(e.data), "stale") {
		t.Error("a file on disk was served in place of the page the binary carries")
	}
}

// The page, the Python app and the Rust port are one product: a Go binary serving the
// repository's page must carry the same version and speak the same protocol.
func TestTheGoPortCarriesTheSameVersionAndProtocol(t *testing.T) {
	root := filepath.Join("..", "..", "..")
	raw, err := os.ReadFile(filepath.Join(root, "python", "bagholder.py"))
	if err != nil {
		t.Skip("the Python app is not in this checkout")
	}
	find := func(re, what string) string {
		m := regexp.MustCompile(re).FindSubmatch(raw)
		if m == nil {
			t.Fatalf("python/bagholder.py has no %s", what)
		}
		return string(m[1])
	}
	if want := find(`(?m)^APP_VERSION = "([^"]+)"`, "APP_VERSION"); want != AppVersion {
		t.Errorf("AppVersion = %q, the Python app says %q", AppVersion, want)
	}
	if want := find(`(?m)^PROTOCOL = "([^"]+)"`, "PROTOCOL"); want != Protocol {
		t.Errorf("Protocol = %q, the Python app says %q", Protocol, want)
	}
}

// A quote tick moves only what is priced off the open positions, so only=live carries
// those sections and leaves the rest of the book on the page.
func TestOnlyLiveCarriesThePricedSectionsAndNothingElse(t *testing.T) {
	whole := []byte(`{"ok":true,"today":"2026-09-18","currency":"CAD","market":{},"positions":[],"positionsSummary":{},"portfolio":{},"markets":{},"trades":[],"equity":[],"kpi":{}}`)
	cut, err := liveOnly(whole, false)
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]any{}
	if err := json.Unmarshal(cut, &got); err != nil {
		t.Fatal(err)
	}
	for _, k := range []string{"ok", "today", "currency", "market", "positions", "positionsSummary", "portfolio"} {
		if _, ok := got[k]; !ok {
			t.Errorf("only=live dropped %q", k)
		}
	}
	for _, k := range []string{"trades", "equity", "kpi", "markets"} {
		if _, ok := got[k]; ok {
			t.Errorf("only=live carried %q, which a price tick cannot move", k)
		}
	}
	withMarkets, err := liveOnly(whole, true)
	if err != nil {
		t.Fatal(err)
	}
	got = map[string]any{}
	_ = json.Unmarshal(withMarkets, &got)
	if _, ok := got["markets"]; !ok {
		t.Error("markets=1 did not add the markets view")
	}
}

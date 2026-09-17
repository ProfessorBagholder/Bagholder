package app

import (
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"testing"
)

func pageHTML(t *testing.T) string {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "..", "ledger.html"))
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

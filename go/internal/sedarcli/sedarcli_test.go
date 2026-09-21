package sedarcli

import (
	"bytes"
	"encoding/json"
	"net"
	"strings"
	"testing"
)

func closedProxy(t *testing.T) {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := "http://" + l.Addr().String()
	l.Close()
	t.Setenv("HTTPS_PROXY", addr)
	t.Setenv("HTTP_PROXY", addr)
}

func TestHelpNamesTheCommands(t *testing.T) {
	var out, errOut bytes.Buffer
	if code := Main([]string{"--help"}, &out, &errOut); code != 2 {
		t.Errorf("exit code = %d, want 2", code)
	}
	if !strings.Contains(errOut.String(), "filings") {
		t.Errorf("help on stderr does not name filings: %q", errOut.String())
	}
	if !strings.Contains(errOut.String(), "resolve") {
		t.Errorf("help on stderr does not name resolve: %q", errOut.String())
	}
}

func TestAMissingDependencyIsReportedAsJSONNotATraceback(t *testing.T) {
	closedProxy(t)
	var out, errOut bytes.Buffer
	if code := Main([]string{"filings", "Shopify"}, &out, &errOut); code != 1 {
		t.Errorf("exit code = %d, want 1", code)
	}
	var payload map[string]any
	if err := json.Unmarshal(out.Bytes(), &payload); err != nil {
		t.Fatalf("stdout is not JSON: %v: %q", err, out.String())
	}
	if ok, _ := payload["ok"].(bool); ok {
		t.Errorf("ok = %v, want false", payload["ok"])
	}
	msg, _ := payload["error"].(string)
	if msg == "" {
		t.Errorf("error is empty: %v", payload)
	}
}

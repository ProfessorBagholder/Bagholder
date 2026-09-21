package app

import "testing"

func TestTheStartupLineIsAClickableURL(t *testing.T) {
	for _, bind := range []string{"127.0.0.1", "0.0.0.0"} {
		if got, want := startupLine(bind, 8799), "http://127.0.0.1:8799"; got != want {
			t.Errorf("startupLine(%q) = %q, want %q", bind, got, want)
		}
	}
}

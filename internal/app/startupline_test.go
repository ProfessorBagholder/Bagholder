package app

import "testing"

func TestTheStartupLineOnLoopback(t *testing.T) {
	if got, want := startupLine("127.0.0.1", 8765), "http://127.0.0.1:8765"; got != want {
		t.Errorf("startupLine = %q, want %q", got, want)
	}
}

func TestTheStartupLineDoesNotPromiseAPortItCannotKnow(t *testing.T) {
	got := startupLine("0.0.0.0", 8765)
	if got == "http://127.0.0.1:8765" {
		t.Error("a container published under another port was told to open 127.0.0.1:8765, which is not where it answers")
	}
	if want := "0.0.0.0:8765"; got != want {
		t.Errorf("startupLine = %q, want %q", got, want)
	}
}

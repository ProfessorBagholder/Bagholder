package py

import "testing"

func TestRound(t *testing.T) {
	cases := []struct {
		x    float64
		n    int
		want float64
	}{{2.675, 2, 2.67}, {0.125, 2, 0.12}, {0.375, 2, 0.38}, {1.005, 2, 1.0}, {2.5, 0, 2}, {3.5, 0, 4}, {-0.5, 0, -0}, {157.125, 2, 157.12}}
	for _, c := range cases {
		if got := Round(c.x, c.n); got != c.want {
			t.Errorf("Round(%v, %d) = %v, want %v", c.x, c.n, got, c.want)
		}
	}
	if RoundInt(2.5) != 2 || RoundInt(3.5) != 4 || RoundInt(-2.5) != -2 {
		t.Error("RoundInt is not banker's rounding")
	}
}

func TestG(t *testing.T) {
	cases := map[float64]string{5: "5", 2.5: "2.5", 1234567: "1.23457e+06", 0.0001: "0.0001", 0.00001: "1e-05", 100: "100", 1.75: "1.75", 0.1 + 0.2: "0.3"}
	for x, want := range cases {
		if got := G(x); got != want {
			t.Errorf("G(%v) = %q, want %q", x, got, want)
		}
	}
}

func TestRepr(t *testing.T) {
	cases := map[float64]string{1: "1.0", 65000: "65000.0", 0.1: "0.1", 1e16: "1e+16", 1.5e-7: "1.5e-07", 123456789.123: "123456789.123"}
	for x, want := range cases {
		if got := Repr(x); got != want {
			t.Errorf("Repr(%v) = %q, want %q", x, got, want)
		}
	}
}

func TestParseISO(t *testing.T) {
	for _, s := range []string{"2026-09-02T09:30:00-04:00", "2026-09-02T09:30:00Z", "2026-09-02", "2026-09-02T09:30", "2026-09-02T09:30:00.123+00:00", "2026-09-02T09:30:00.000Z", "2026-09-02 09:30:00"} {
		if _, _, ok := ParseISO(s); !ok {
			t.Errorf("ParseISO(%q) failed", s)
		}
	}
	tm, naive, ok := ParseISO("2026-09-02T09:30:00-04:00")
	if !ok || naive || tm.UTC().Hour() != 13 {
		t.Errorf("offset not honoured: %v %v %v", tm, naive, ok)
	}
	if _, _, ok := ParseISO("nonsense"); ok {
		t.Error("nonsense parsed")
	}
}

func TestText(t *testing.T) {
	if CollapseSpace(" a   b\n\tc ") != " a b c " {
		t.Errorf("CollapseSpace: %q", CollapseSpace(" a   b\n\tc "))
	}
	if Title("aUG") != "Aug" {
		t.Error("Title")
	}
	if Commas(1500000) != "1,500,000" || Commas(999) != "999" || Commas(-1234) != "-1,234" {
		t.Error("Commas")
	}
	if got := Lines("a\r\nb\nc"); len(got) != 3 || got[1] != "b" {
		t.Errorf("Lines: %v", got)
	}
}

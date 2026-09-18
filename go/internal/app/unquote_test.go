package app

import "testing"

func TestPyUnquoteMatchesPythonUnquote(t *testing.T) {
	cases := map[string]string{
		"%7B%22access_token%22%3A%22a%22%7D": `{"access_token":"a"}`,
		`{"a":"50% off"}`:                    `{"a":"50% off"}`,
		"100%25":                             "100%",
		"a%":                                 "a%",
		"a%z9":                               "a%z9",
		"a+b":                                "a+b",
		"%E2%82%AC":                          "€",
	}
	for in, want := range cases {
		if got := pyUnquote(in); got != want {
			t.Errorf("pyUnquote(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestATokenSurvivesAStrayPercent(t *testing.T) {
	raw := `%7B%22access_token%22%3A%22tok%22%2C%22name%22%3A%22100%2525%20Equity%22%7D`
	got := jsonWithAccessToken(raw)
	if got == nil {
		t.Fatalf("no token parsed from %q", raw)
	}
	if got["access_token"] != "tok" {
		t.Errorf("access_token = %v, want tok", got["access_token"])
	}
}

package app

import "testing"

func TestSafeURLKeepsTheAddressAndDropsTheQuery(t *testing.T) {
	cases := map[string]string{
		"https://my.wealthsimple.com/app/login?code=secret&state=abc": "https://my.wealthsimple.com/app/login",
		"https://my.wealthsimple.com/app/login":                       "https://my.wealthsimple.com/app/login",
		"about:blank":                                                 "about:blank",
		"https://x.test/a/b#tok":                                      "https://x.test/a/b",
	}
	for in, want := range cases {
		if got := safeURL(in); got != want {
			t.Errorf("safeURL(%q) = %q, want %q", in, got, want)
		}
	}
}

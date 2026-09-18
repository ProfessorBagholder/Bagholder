package app

import "testing"

func acceptable(body map[string]any, refused any) bool {
	return body != nil && truthy(body["access_token"]) && body["refresh_token"] != refused
}

func TestASessionWithoutARefreshTokenIsNotCaptured(t *testing.T) {
	var refused any
	body := map[string]any{"access_token": "partial"}
	if acceptable(body, refused) {
		t.Error("an access token with no refresh token was accepted; the reference waits for one, so the login window would close before the second factor")
	}
}

func TestAFullSessionIsCaptured(t *testing.T) {
	var refused any
	body := map[string]any{"access_token": "a", "refresh_token": "r"}
	if !acceptable(body, refused) {
		t.Error("a complete session was not accepted")
	}
}

func TestARefusedRefreshTokenIsNotRetried(t *testing.T) {
	body := map[string]any{"access_token": "a", "refresh_token": "r"}
	var refused any = "r"
	if acceptable(body, refused) {
		t.Error("the refresh token Wealthsimple refused was offered again")
	}
	body["refresh_token"] = "r2"
	if !acceptable(body, refused) {
		t.Error("a different refresh token was not accepted")
	}
}

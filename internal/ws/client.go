package ws

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const (
	OAuth          = "https://api.production.wealthsimple.com/v1/oauth/v2"
	GraphQLURL     = "https://my.wealthsimple.com/graphql"
	GraphQLVersion = "12"
	WSClient       = "@wealthsimple/wealthsimple"
	LoginURL       = "https://my.wealthsimple.com/app/login"
	RefusedLogin   = "Saved login refused. Connect Wealthsimple again."
)

var Queries = map[string]string{
	"FetchAccountCurrentMarginBuyingPowerV2": QFetchAccountMarginBuyingPower,
	"FetchSecurities":                        QFetchSecurities,
	"IdentityHistoricalFinancialsQuery":      QIdentityHistoricalFinancials,
	"FetchAccountHistoricalFinancials":       QFetchAccountHistoricalFinancials,
	"FetchAllAccountFinancials":              QFetchAllAccountFinancials,
	"FetchActivityFeedItems":                 QFetchActivityFeedItems,
	"FetchAccountsWithBalance":               QFetchAccountsWithBalance,
	"FetchSecuritySearchResult":              QFetchSecuritySearchResult,
	"FetchSecurity":                          QFetchSecurity,
	"FetchSecuritiesSummary":                 QFetchSecuritiesSummary,
	"FetchSecurityMarketData":                QFetchSecurityMarketData,
	"FetchTradingBalanceBuyingPower":         QFetchTradingBalanceBuyingPower,
	"SoOrdersOrderCreate":                    QSoOrdersOrderCreate,
	"FetchSoOrdersExtendedOrder":             QFetchSoOrdersExtendedOrder,
	"OrderServiceExtendedOrderFeed":          QOrderServiceExtendedOrderFeed,
	"SoOrdersOrderCancel":                    QSoOrdersOrderCancel,
	"SoOrdersOrderModify":                    QSoOrdersOrderModify,
}

var ErrNotAuthorized = errors.New("not authorized")

type Client struct {
	Files     *Files
	HTTP      *http.Client
	OnError   func(msg string)
	refreshMu sync.Mutex
	refused   string
	mu        sync.Mutex
}

func NewClient(home string) *Client {
	transport := market.NewHTTPClient().Transport
	return &Client{Files: &Files{Home: home}, HTTP: &http.Client{Transport: transport, Timeout: 120 * time.Second}}
}

func (c *Client) setError(msg string) {
	if c.OnError != nil {
		c.OnError(msg)
	}
}

func (c *Client) ResetRefused() {
	c.mu.Lock()
	c.refused = ""
	c.mu.Unlock()
}

func bodyText(raw []byte) string {
	if len(raw) == 0 {
		return ""
	}
	if len(raw) >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
		raw = market.GunzipIfNeeded(raw)
	}
	return strings.ToValidUTF8(string(raw), "�")
}

func (c *Client) HTTPJSON(method, rawURL string, body any, headers map[string]string, timeout time.Duration) map[string]any {
	hdrs := map[string]string{"Accept": "application/json"}
	if ua := c.Files.CachedUserAgent(); ua != "" {
		hdrs["User-Agent"] = ua
	}
	for k, v := range headers {
		hdrs[k] = v
	}
	var reader io.Reader
	if body != nil {
		data, err := json.Marshal(body)
		if err != nil {
			return map[string]any{"error": "encode", "_http_status": 0}
		}
		reader = bytes.NewReader(data)
		hdrs["Content-Type"] = "application/json"
	}
	req, err := http.NewRequest(method, rawURL, reader)
	if err != nil {
		return map[string]any{"error": "request", "_http_status": 0}
	}
	for k, v := range hdrs {
		req.Header.Set(k, v)
	}
	client := *c.HTTP
	if timeout > 0 {
		client.Timeout = timeout
	}
	resp, err := client.Do(req)
	if err != nil {
		return map[string]any{"error": "url_error", "_http_status": 0, "_error": err.Error()}
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return map[string]any{"error": "url_error", "_http_status": 0, "_error": err.Error()}
	}
	text := bodyText(raw)
	if resp.StatusCode >= 400 {
		var parsed map[string]any
		if text != "" {
			if err := json.Unmarshal([]byte(text), &parsed); err != nil || parsed == nil {
				parsed = map[string]any{"error": fmt.Sprintf("http_%d", resp.StatusCode)}
			}
		} else {
			parsed = map[string]any{}
		}
		parsed["_http_status"] = float64(resp.StatusCode)
		return parsed
	}
	if text == "" {
		return map[string]any{}
	}
	var parsed map[string]any
	if err := json.Unmarshal([]byte(text), &parsed); err != nil {
		return map[string]any{"error": "invalid_json", "_http_status": float64(resp.StatusCode)}
	}
	if parsed == nil {
		parsed = map[string]any{}
	}
	return parsed
}

func statusOf(data map[string]any) int {
	if data == nil {
		return 0
	}
	v, _ := py.NumOK(data["_http_status"])
	return int(v)
}

var appJSRE = regexp.MustCompile(`(?i)<script[^>]+src="([^"]*app-[a-f0-9]+\.js[^"]*)"`)
var clientIDRE = regexp.MustCompile(`(?s)production:.*?clientId:"([a-f0-9]+)"`)

func (c *Client) getText(rawURL string, timeout time.Duration) (string, error) {
	req, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return "", err
	}
	if ua := c.Files.CachedUserAgent(); ua != "" {
		req.Header.Set("User-Agent", ua)
	}
	client := *c.HTTP
	client.Timeout = timeout
	resp, err := client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return "", err
	}
	if resp.StatusCode >= 400 {
		return "", fmt.Errorf("HTTP %d", resp.StatusCode)
	}
	return bodyText(raw), nil
}

func (c *Client) ScrapeClientID() string {
	if cached := c.Files.CachedClientID(); cached != "" {
		return cached
	}
	html, err := c.getText(LoginURL, 20*time.Second)
	if err != nil {
		return ""
	}
	m := appJSRE.FindStringSubmatch(html)
	if m == nil {
		return ""
	}
	jsURL := m[1]
	if strings.HasPrefix(jsURL, "//") {
		jsURL = "https:" + jsURL
	} else if strings.HasPrefix(jsURL, "/") {
		jsURL = "https://my.wealthsimple.com" + jsURL
	}
	js, err := c.getText(jsURL, 20*time.Second)
	if err != nil {
		return ""
	}
	if m2 := clientIDRE.FindStringSubmatch(js); m2 != nil {
		c.Files.SaveClientID(m2[1])
		return m2[1]
	}
	return ""
}

func (c *Client) ClientIDFor(sess Session) string {
	if cid := sess.Str("client_id"); cid != "" {
		c.Files.SaveClientID(cid)
		return cid
	}
	return c.Files.CachedClientID()
}

func sessionHeaders(sess Session, headers map[string]string) map[string]string {
	out := map[string]string{}
	for k, v := range headers {
		out[k] = v
	}
	if v := sess.Str("wssdi"); v != "" {
		out["x-ws-device-id"] = v
	}
	if v := sess.Str("session_id"); v != "" {
		out["x-ws-session-id"] = v
	}
	return out
}

var hexTokenRE = regexp.MustCompile(`(?i)^[a-f0-9]{32,}$`)
var oauthCodeRE = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,64}$`)

func OAuthErrorCode(data map[string]any) string {
	err, ok := data["error"].(string)
	if !ok {
		return ""
	}
	err = strings.TrimSpace(err)
	if err == "" || hexTokenRE.MatchString(err) || !oauthCodeRE.MatchString(err) {
		return ""
	}
	return PublicSyncError(err)
}

func refreshFailureMessage(data map[string]any) string {
	status := statusOf(data)
	code := OAuthErrorCode(data)
	if code == "invalid_grant" {
		return RefusedLogin
	}
	var parts []string
	if status != 0 {
		parts = append(parts, fmt.Sprintf("Wealthsimple token refresh HTTP %d", status))
	}
	if code != "" {
		parts = append(parts, code)
	}
	if len(parts) == 0 {
		return "Wealthsimple token refresh failed"
	}
	return strings.Join(parts, " ")
}

func expiresAtTimestamp(data map[string]any) string {
	raw := data["expires_at"]
	if s, ok := raw.(string); ok && strings.Contains(strings.TrimSpace(s), "T") {
		return strings.TrimSpace(s)
	}
	var unix float64
	have := false
	if f, ok := raw.(float64); ok {
		unix, have = f, true
	} else if data["expires_in"] != nil {
		if n, ok := py.NumOK(data["expires_in"]); ok {
			unix, have = float64(time.Now().Unix())+float64(int64(n)), true
		}
	}
	if !have {
		return ""
	}
	return time.Unix(int64(unix), 0).UTC().Format("2006-01-02T15:04:05.000Z")
}

func (c *Client) RefreshSession(sess Session, adopt bool) bool {
	rt := sess.Str("refresh_token")
	if rt == "" {
		c.setError("missing refresh token")
		return false
	}
	c.refreshMu.Lock()
	defer c.refreshMu.Unlock()
	if adopt {
		current := c.Files.LoadSession()
		if current.Str("access_token") != "" && current.Str("refresh_token") != "" && current.Str("refresh_token") != rt {
			for k, v := range current {
				sess[k] = v
			}
			return true
		}
	}
	c.mu.Lock()
	refused := c.refused
	c.mu.Unlock()
	if rt == refused {
		c.setError(RefusedLogin)
		return false
	}
	return c.refreshLocked(sess, rt)
}

func (c *Client) refreshLocked(sess Session, rt string) bool {
	cid := c.ClientIDFor(sess)
	if cid == "" {
		c.setError("session has no client id")
		return false
	}
	body := map[string]any{"grant_type": "refresh_token", "refresh_token": rt, "client_id": cid}
	headers := sessionHeaders(sess, map[string]string{"x-wealthsimple-client": WSClient, "x-ws-profile": "invest"})
	data := c.HTTPJSON(http.MethodPost, OAuth+"/token", body, headers, 60*time.Second)
	if py.S(data["access_token"]) == "" {
		if OAuthErrorCode(data) == "invalid_grant" {
			c.mu.Lock()
			c.refused = rt
			c.mu.Unlock()
		}
		c.setError(refreshFailureMessage(data))
		return false
	}
	sess["access_token"] = data["access_token"]
	if py.S(data["refresh_token"]) != "" {
		sess["refresh_token"] = data["refresh_token"]
	}
	if stamped := expiresAtTimestamp(data); stamped != "" {
		sess["expires_at"] = stamped
	}
	sess["client_id"] = cid
	c.Files.SaveSession(sess)
	return true
}

func (c *Client) TokenInfo(sess Session) map[string]any {
	token := sess.Str("access_token")
	if token == "" {
		return map[string]any{}
	}
	headers := sessionHeaders(sess, map[string]string{"Authorization": "Bearer " + token, "x-wealthsimple-client": WSClient})
	data := c.HTTPJSON(http.MethodGet, OAuth+"/token/info", nil, headers, 60*time.Second)
	if st := statusOf(data); st == 401 || st == 403 {
		return map[string]any{}
	}
	if data == nil {
		return map[string]any{}
	}
	return data
}

func ClientIDFromTokenInfo(info map[string]any) string {
	if uid := py.S(info["application_uid"]); uid != "" {
		return strings.TrimSpace(uid)
	}
	if app, ok := info["application"].(map[string]any); ok {
		if uid := py.S(app["uid"]); uid != "" {
			return strings.TrimSpace(uid)
		}
	}
	return ""
}

func (c *Client) ApplyTokenInfoClientID(sess Session, info map[string]any) string {
	if sess.Str("access_token") == "" {
		return ""
	}
	if info == nil {
		info = c.TokenInfo(sess)
	}
	cid := ClientIDFromTokenInfo(info)
	if cid == "" {
		return ""
	}
	sess["client_id"] = cid
	c.Files.SaveClientID(cid)
	return cid
}

var IdentityKeys = []string{"identity_canonical_id", "identityCanonicalId", "canonical_id", "identity_id", "resource_owner_id", "sub"}

func IdentityFrom(obj map[string]any) string {
	for _, k := range IdentityKeys {
		if v := py.JSONStr(obj[k]); v != "" {
			return v
		}
	}
	return ""
}

func (c *Client) GraphQL(sess Session, operation string, variables map[string]any, query string) (map[string]any, error) {
	token := sess.Str("access_token")
	headers := map[string]string{
		"Authorization":         "Bearer " + token,
		"x-wealthsimple-client": WSClient,
		"x-ws-profile":          "trade",
		"x-ws-api-version":      GraphQLVersion,
		"x-ws-locale":           "en-CA",
		"x-platform-os":         "web",
		"Content-Type":          "application/json",
		"Origin":                "https://my.wealthsimple.com",
		"Referer":               "https://my.wealthsimple.com/app/trade",
	}
	if v := sess.Str("wssdi"); v != "" {
		headers["x-ws-device-id"] = v
	}
	if v := sess.Str("session_id"); v != "" {
		headers["x-ws-session-id"] = v
	}
	if query == "" {
		query = Queries[operation]
	}
	vars := map[string]any{}
	for k, v := range variables {
		if v != nil {
			vars[k] = v
		}
	}
	body := map[string]any{"operationName": operation, "query": query, "variables": vars}
	data := c.HTTPJSON(http.MethodPost, GraphQLURL, body, headers, 90*time.Second)
	if st := statusOf(data); st == 401 || st == 403 {
		return nil, ErrNotAuthorized
	}
	if errs := data["errors"]; errs != nil {
		var first any = errs
		if list, ok := errs.([]any); ok {
			if len(list) > 0 {
				first = list[0]
			} else {
				first = nil
			}
		}
		if first != nil {
			emsg := ""
			if m, ok := first.(map[string]any); ok {
				emsg = py.S(m["message"])
				if emsg == "" {
					emsg = py.S(m["error"])
				}
				if emsg == "" {
					b, _ := json.Marshal(m)
					emsg = string(b)
				}
			} else {
				emsg = py.S(first)
			}
			return nil, errors.New(operation + ": " + emsg)
		}
	}
	out, ok := data["data"].(map[string]any)
	if !ok || out == nil {
		if e := py.S(data["_error"]); e != "" {
			return nil, errors.New("graphql failed: " + operation + ": " + e)
		}
		return nil, errors.New("graphql failed: " + operation)
	}
	return out, nil
}

var bearerRE = regexp.MustCompile(`(?i)bearer\s+\S+`)
var tokenKVRE = regexp.MustCompile(`(?i)(access_token|refresh_token)\s*[:=]\s*\S+`)
var tokenWordRE = regexp.MustCompile(`(?i)(bearer|access_token|refresh_token)`)

func PublicSyncError(msg string) string {
	msg = strings.TrimSpace(strings.ReplaceAll(msg, "\n", " "))
	if strings.Contains(msg, "CERTIFICATE_VERIFY_FAILED") || strings.Contains(msg, "unable to get local issuer certificate") || strings.Contains(msg, "certificate signed by unknown authority") {
		return "could not verify HTTPS certificates"
	}
	msg = bearerRE.ReplaceAllString(msg, "[redacted]")
	msg = tokenKVRE.ReplaceAllString(msg, "$1=[redacted]")
	low := strings.ToLower(msg)
	if strings.Contains(low, "bearer") || strings.Contains(low, "access_token") || strings.Contains(low, "refresh_token") {
		msg = tokenWordRE.ReplaceAllString(msg, "[redacted]")
	}
	msg = py.Strip(py.CollapseSpace(msg))
	if len([]rune(msg)) > 180 {
		msg = string([]rune(msg)[:177]) + "..."
	}
	if msg == "" {
		return "unknown error"
	}
	return msg
}

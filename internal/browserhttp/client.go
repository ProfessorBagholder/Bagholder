package browserhttp

import (
	"bytes"
	"io"
	"net/url"
	"os"
	"strings"
	"time"

	fhttp "github.com/bogdanfinn/fhttp"
	tls_client "github.com/bogdanfinn/tls-client"
	"github.com/bogdanfinn/tls-client/profiles"
)

type Response struct {
	Status int
	Body   []byte
	Header map[string][]string
	URL    string
}

type Session struct {
	client tls_client.HttpClient
}

const UserAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36"

var chromeOrder = []string{"sec-ch-ua", "sec-ch-ua-mobile", "sec-ch-ua-platform", "upgrade-insecure-requests", "user-agent", "accept", "sec-fetch-site", "sec-fetch-mode", "sec-fetch-user", "sec-fetch-dest", "accept-encoding", "accept-language", "referer", "origin", "content-type", "cookie"}

var chromeHeaders = map[string]string{
	"sec-ch-ua":                 `"Not;A=Brand";v="8", "Chromium";v="150", "Google Chrome";v="150"`,
	"sec-ch-ua-mobile":          "?0",
	"sec-ch-ua-platform":        `"macOS"`,
	"upgrade-insecure-requests": "1",
	"user-agent":                UserAgent,
	"accept":                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7",
	"sec-fetch-site":            "none",
	"sec-fetch-mode":            "navigate",
	"sec-fetch-user":            "?1",
	"sec-fetch-dest":            "document",
	"accept-encoding":           "gzip, deflate, br, zstd",
	"accept-language":           "en-US,en;q=0.9",
}

func proxyFromEnv() string {
	for _, k := range []string{"HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"} {
		if v := strings.TrimSpace(os.Getenv(k)); v != "" {
			return v
		}
	}
	return ""
}

func New(timeoutSec int, followRedirects bool) (*Session, error) {
	jar := tls_client.NewCookieJar()
	options := []tls_client.HttpClientOption{
		tls_client.WithTimeoutSeconds(timeoutSec),
		tls_client.WithClientProfile(profiles.Chrome_150),
		tls_client.WithCookieJar(jar),
	}
	if !followRedirects {
		options = append(options, tls_client.WithNotFollowRedirects())
	}
	if p := proxyFromEnv(); p != "" {
		options = append(options, tls_client.WithProxyUrl(p))
	}
	client, err := tls_client.NewHttpClient(tls_client.NewNoopLogger(), options...)
	if err != nil {
		return nil, err
	}
	return &Session{client: client}, nil
}

func (s *Session) do(method, rawURL string, headers map[string]string, body []byte) (*Response, error) {
	var reader io.Reader
	if body != nil {
		reader = bytes.NewReader(body)
	}
	req, err := fhttp.NewRequest(method, rawURL, reader)
	if err != nil {
		return nil, err
	}
	given := map[string]string{}
	for k, v := range headers {
		given[strings.ToLower(k)] = v
	}
	order := []string{}
	seen := map[string]bool{}
	for _, k := range chromeOrder {
		v, ok := given[k]
		if !ok {
			v, ok = chromeHeaders[k]
		}
		if !ok {
			continue
		}
		req.Header.Set(k, v)
		order = append(order, k)
		seen[k] = true
	}
	for k, v := range given {
		if !seen[k] {
			req.Header.Set(k, v)
			order = append(order, k)
		}
	}
	req.Header[fhttp.HeaderOrderKey] = order
	resp, err := s.client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return nil, err
	}
	final := rawURL
	if resp.Request != nil && resp.Request.URL != nil {
		final = resp.Request.URL.String()
	}
	return &Response{Status: resp.StatusCode, Body: data, Header: resp.Header, URL: final}, nil
}

func (s *Session) Get(rawURL string, headers map[string]string) (*Response, error) {
	return s.do(fhttp.MethodGet, rawURL, headers, nil)
}

func (s *Session) Post(rawURL string, headers map[string]string, body []byte) (*Response, error) {
	return s.do(fhttp.MethodPost, rawURL, headers, body)
}

func (s *Session) Cookies(rawURL string) map[string]string {
	u, err := url.Parse(rawURL)
	if err != nil {
		return nil
	}
	out := map[string]string{}
	for _, c := range s.client.GetCookies(u) {
		out[c.Name] = c.Value
	}
	return out
}

func (s *Session) SetCookie(rawURL, name, value string) {
	u, err := url.Parse(rawURL)
	if err != nil {
		return
	}
	s.client.SetCookies(u, []*fhttp.Cookie{{Name: name, Value: value}})
}

var _ = time.Second

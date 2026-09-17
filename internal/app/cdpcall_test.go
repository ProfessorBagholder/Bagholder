package app

import (
	"bufio"
	"crypto/sha1"
	"encoding/base64"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func fakeDevTools(t *testing.T, answer bool) (string, func()) {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		key := r.Header.Get("Sec-WebSocket-Key")
		sum := sha1.Sum([]byte(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))
		accept := base64.StdEncoding.EncodeToString(sum[:])
		hj, ok := w.(http.Hijacker)
		if !ok {
			t.Error("no hijacker")
			return
		}
		conn, rw, err := hj.Hijack()
		if err != nil {
			t.Error(err)
			return
		}
		defer conn.Close()
		rw.WriteString("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " + accept + "\r\n\r\n")
		rw.Flush()
		serveFakeFrames(conn, rw, answer)
	}))
	return "ws://" + strings.TrimPrefix(srv.URL, "http://"), srv.Close
}

func serveFakeFrames(conn net.Conn, rw *bufio.ReadWriter, answer bool) {
	head := make([]byte, 2)
	for {
		conn.SetReadDeadline(time.Now().Add(5 * time.Second))
		if _, err := rw.Read(head); err != nil {
			return
		}
		n := int(head[1] & 0x7f)
		if head[1]&0x80 != 0 {
			n += 4
		}
		if _, err := rw.Discard(n); err != nil {
			return
		}
		if !answer {
			continue
		}
		body := []byte(`{"id":1,"result":{}}`)
		rw.Write([]byte{0x81, byte(len(body))})
		rw.Write(body)
		rw.Flush()
	}
}

func TestASlowDevToolsReplyIsNotAFailedInput(t *testing.T) {
	url, stop := fakeDevTools(t, false)
	defer stop()
	w, err := wsConnect(url, 2*time.Second)
	if err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer w.close()
	msg, live := cdpExchange(w, "Input.dispatchKeyEvent", map[string]any{"type": "keyDown"}, 300*time.Millisecond)
	if msg != nil {
		t.Errorf("msg = %v, want nil", msg)
	}
	if !live {
		t.Error("a reply that did not arrive in time reported the connection dead; the reference forwards the input regardless")
	}
}

func TestAClosedDevToolsSocketIsAFailedInput(t *testing.T) {
	url, stop := fakeDevTools(t, true)
	w, err := wsConnect(url, 2*time.Second)
	if err != nil {
		t.Fatalf("connect: %v", err)
	}
	stop()
	w.close()
	if _, live := cdpExchange(w, "Input.dispatchKeyEvent", map[string]any{"type": "keyDown"}, 300*time.Millisecond); live {
		t.Error("a closed socket reported the connection live")
	}
}

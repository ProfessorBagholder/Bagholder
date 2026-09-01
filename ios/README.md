# Bagholder iOS (M1 shell)

First pass: the desktop `ledger.html` in a `WKWebView`, talking to a loopback HTTP server in the app. Same `/api/status` and empty `/api/book` as `bagholder.py`.

Living brief: GitHub issue #14.

## What this is not

- Not TestFlight, not the App Store, not the paid Apple Developer Program.
- Not Connect. Do not type Wealthsimple login here yet (M3, on your phone, after SpikeCookie A–C).

## Open

1. Xcode on your Mac (air).
2. Open `ios/Bagholder.xcodeproj` (or `xcodegen generate` if you use `project.yml`).
3. Signing: Automatic, **Personal Team** (free Apple Account in Xcode).
4. Run on the iOS Simulator.

The Xcode project uses the repo-root `ledger.html` and `favicon.png` (same files as the desktop app).

On a real iPhone, Apple’s Personal Team profile lasts 7 days. Reinstall from Xcode after that. Developer Mode once on the phone.

## Ports

The in-app server binds `127.0.0.1:8765`, then `8766`, then `8767`. The WebView must load `http://127.0.0.1:<that-port>/` so `HAS_LOCAL_API` in `ledger.html` is true.

## Tests (Simulator)

```
xcodebuild test -project ios/Bagholder.xcodeproj -scheme Bagholder -destination 'platform=iOS Simulator,name=iPhone 16'
```

`testLoopbackStatusOK` hits `/api/status`. `testEmptyBook` hits `/api/book`. `testProcessCanGETLoopbackHTTP` hits `/health`. Those prove URLSession in the test process can GET loopback HTTP. They are not the WKWebView ATS gate; that is the Simulator app load of `http://127.0.0.1:<port>/`.

Info.plist uses `NSAllowsLocalNetworking` and `NSExceptionDomains` for `127.0.0.1` and `localhost` with `NSExceptionAllowsInsecureHTTPLoads`. It does not set `NSAllowsArbitraryLoads`.

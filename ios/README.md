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

On a real iPhone, Apple’s Personal Team profile lasts 7 days. Reinstall from Xcode after that. Developer Mode once on the phone.

## Ports

The in-app server binds `127.0.0.1:8765`, then `8766`, then `8767`. The WebView must load `http://127.0.0.1:<that-port>/` so `HAS_LOCAL_API` in `ledger.html` is true.

# Mobile

Decided 2026-09-08: Bagholder stays local-first on every platform. Nothing leaves the user's device, so there is no server for the apps to lean on, and each app carries the whole model. The apps are plain native, SwiftUI on iOS (`ios/`) and Jetpack Compose on Android (`android/`), in this repository. Each has its own implementation of the model, and the three implementations, with the Rust model (`crates/model`) as the reference, are held to the same answers by the shared cases in `tests/cases`, which every implementation's tests run: a rule changed in one place and not the others fails a PR before it lands. What the apps show is `SPEC.md` (§8 for the phone); how work lands is `CLAUDE.md`. This file is how the apps are built, run, seeded and tested, and what the platforms will not allow.

## What is where

- `ios/Bagholder/`: `Model.swift` + `ModelView.swift` (the model), `Screens.swift` (every screen), `Charts.swift`, `Theme.swift` (the page's tokens and the spec's formatting), `AppState.swift` (`Book`: the keychain session, the last pull, the journal, the filters, the pull, the market loop, the login web view), `Market.swift` (quotes, distribution records, index closes, daily bars), `WSPull.swift` (the Wealthsimple pull). `ios/BagholderTests/ModelCasesTests.swift` runs the shared cases.
- `android/app/src/main/kotlin/com/bagholder/app/`: `MainActivity.kt` (every screen), `Charts.kt`, `Theme.kt`, `Journal.kt` (`Book`), `Store.kt`, `Market.kt`, `WSPull.kt`, `Queries.kt` (the GraphQL documents, generated from the Swift file). `android/model/` is the model; its test runs the shared cases.
- The version is one number across the product: `APP_VERSION` in `crates/server/src/app.rs`, `MARKETING_VERSION` in `ios/Bagholder.xcodeproj/project.pbxproj`, `versionName` in `android/app/build.gradle.kts`.

## Build, run, test

**iOS** (Xcode 26; the user builds to the phone from Xcode, so the project file carries no team: signing is picked once in Signing & Capabilities).

```
cd ios
xcodebuild build -project Bagholder.xcodeproj -scheme Bagholder -destination 'platform=iOS Simulator,name=iPhone 17'
xcodebuild test  -project Bagholder.xcodeproj -scheme Bagholder -destination 'platform=iOS Simulator,name=iPhone 17' -only-testing:SpikeLoopback   # the shared cases
```

Headless on the simulator: `xcrun simctl install <udid> <DerivedData>/Build/Products/Debug-iphonesimulator/Bagholder.app` (skip `Index.noindex` when locating it), `simctl launch <udid> com.bagholder.app`, `simctl io <udid> screenshot <absolute path>`. Seed the app container's `Library/Application Support/Bagholder/`: `last-pull.json` in the `WSPullResult` shape (activities, listings, nav, navByAccount, and since the Portfolio tab accounts with their net liquidation value, balances and margin), `journal.json`, `fx.json`, `indexes.json`; the FRED and Bank of Canada caches are `defaults` keys (`simctl spawn <udid> defaults write com.bagholder.app …`). Taps and long presses can be driven from Claude Code's simulator tool; the tab bar sits at y≈828 pt.

**Android** (JDK 21, `ANDROID_HOME`; Gradle runs offline once the caches are warm).

```
cd android
JAVA_HOME=/opt/homebrew/opt/openjdk@21 ANDROID_HOME=~/Library/Android/sdk ./gradlew --offline :app:assembleDebug
JAVA_HOME=/opt/homebrew/opt/openjdk@21 ANDROID_HOME=~/Library/Android/sdk ./gradlew --offline :model:test --rerun-tasks   # the shared cases
```

Headless: `emulator -avd Medium_Phone_API_35 -no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect`, `adb install -r app/build/outputs/apk/debug/app-debug.apk`, `adb shell cmd uimode night yes`, `adb exec-out screencap -p`. Seed `files/bagholder/` through `adb push … /data/local/tmp/` then `adb shell run-as com.bagholder.app cp …` (`last-pull.json` with activities, listings, nav, navByAccount, syncedAt, accounts, balances, margin). A long press on the emulator is `input motionevent DOWN`, a pause, `MOVE`, `UP`; an `input swipe` with a long duration is a drag, not a press. The tab bar sits at y≈2310 px. The debug APK is signed with this machine's debug key; a tester installs it by opening the file (Android 8 or newer) and a later build from the same machine installs over it.

**Side by side.** The apps are the same only when they have been shown to be: seed both with the same rows (the desktop's snapshot converted to each app's pull shape), capture every screen and state on each (tile pages, card-row pages, details top and bottom, sheets, empty page, pressed charts), read them next to each other, and fix or list what differs. Card gaps are measured in the screenshots, not judged by eye. Speed is judged on a phone launched from the home screen: under Xcode's debugger the web engine and the first frame run about ten times slower and look like an app bug.

**A phone test build before a merge** is a GitHub pre-release pointing at its branch, tagged outside the `vX.Y.Z` scheme; a release page carries only what its tag contains (`CLAUDE.md`).

## What the platforms will not allow

- **Passkeys in the login.** The sign-in is Wealthsimple's page in the app's own web view, because the app needs the session cookies that page sets, which no system sign-in sheet hands back. iOS runs a passkey inside a third-party app's web view only when the site lists the app in its `apple-app-site-association` (Wealthsimple lists only its own apps) or the app holds Apple's web-browser passkey entitlement (granted to browsers on application; Xcode rejects it for this account); the request reaches iOS and is refused (`ASAuthorizationError 1004`), verified on the phone with no debugger attached. Android's web view exposes no passkey support to an app without the same association, so the page hides the button. Sign-in is email, password and the two-factor code.
- **The login web view on iOS** is one persistent instance, loaded ahead of the tap and parked in the window behind the opaque root while it loads, because WebKit suspends a web view that is not in a window and then blocks the main thread when such a view is shown. Disconnect clears cookies and storage but keeps the HTTP cache. Do not add a keyboard prewarm at launch (a hidden first responder can block a device for seconds).
- **Sizes.** Both apps ask for the same sizes in points and dp, which are within a few percent of each other physically at the phones' default display settings; a uniform difference between two phones is a display-size setting, not the app, and is not compensated for.

## Not there yet

Intraday timeframes (1H, 4H) on the trade chart; the Midnight theme; Wealthsimple's balance beside a position's quantity (the phone pull does not fetch balances); the equity curve's account switch beyond the account filter; a real Wealthsimple login and pull on Android (iOS has done both on a phone).

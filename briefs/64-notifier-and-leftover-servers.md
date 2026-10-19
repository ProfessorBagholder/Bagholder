# Brief 64: The Mac notifier must never show in the Dock, and no copy of the app outlives the work that started it

## What happened

Two problems, reported by the owner on 2026-10-19 from their Mac, and traced there in the system log:

1. **Every desktop notification flashes a Bagholder icon in the Dock.** The notifier is an `osacompile` applet launched once per notification (`open -n -W … Bagholder.app`, `notify.rs`, `mac_open_command`). Its `Info.plist` has no `LSUIElement` key (`mac_build` sets only `CFBundleIdentifier` and `CFBundleDisplayName`). So macOS runs it as an ordinary app: a Dock tile for about 0.25 s, nine times in 40 minutes.
2. **Two copies of the app, left running by earlier sessions, were posting notifications.** They were scratch servers of the old Python app, on ports 8801 and 8803, with data folders in a session scratchpad and in `/tmp`. They had run since 2026-10-18, unknown to the owner, without `BAGHOLDER_NOTIFY=browser`. They posted the same press releases twice.

The owner's words: "You and other agents working on this are never permitted to leave old copies of the app running after your work/tasks are completed. And once again, features must be designed in line with best practices and how top human engineers of the top apps in the world would do it… a dock icon randomly appearing and disappearing and multiple instances of the app laying around unexpectedly after dev work is not in line with those instructions… zero of the apps i've ever seen do this."

Both are defects of design, not of the moment. Each is fixed so that it cannot recur, by construction and by test, never by a rule someone has to remember.

## 1. The notifier is an agent app

**The established answer.** macOS has one mechanism for a helper that works for an app without appearing as one: the agent app. Apple's own words: "If this key is set to `YES`, Launch Services runs the app as an agent app. Agent apps do not appear in the Dock or in the Force Quit window" (Information Property List Key Reference, Launch Services Keys, `LSUIElement`). Background helpers of shipping apps are built this way, and so are the Dock and loginwindow themselves.

Required:
1. `mac_build` sets `LSUIElement` to true in the applet's `Info.plist`.
   - Not `LSBackgroundOnly`: that key is for faceless apps that never connect to the window server, and a notification needs it.
   - The key goes into what `mac_stamp` covers, so every applet already installed is rebuilt at its next use.
2. **Test, on CI:** `test_the_mac_applet_is_built_once_under_bagholders_name_and_icon` asserts `"LSUIElement" => true` in the built plist. That test already runs in the `rust-macos` job (`--ignored --exact`), so it needs no new way to run. A second assertion: a built applet without the key has a different stamp from one with it, so an old applet can never be taken as current.
3. **Verified on a Mac with a desktop**, by the session on the owner's Mac, never by the owner. Post a real notification through the rebuilt applet, then confirm:
   - `log show --predicate 'process == "Dock"'` across it has no new tile line;
   - the banner, and the entry in System Settings → Notifications, still say Bagholder with its icon;
   - a click on the banner still opens the app in the browser (SPEC notifications-3 and notifications-7).

   If the agent applet loses Bagholder's name or icon on the banner, stop and report it with the log. The next step is then a signed helper posting through `UNUserNotificationCenter`, a plan of its own.

Launching one invisible applet per notification is kept: each launch is a process the person never sees, and nothing else in SPEC changes.

## 2. Only the person's own copy posts to the desktop

`BAGHOLDER_NOTIFY=browser` relies on whoever starts a copy remembering to set it. Neither leftover copy had it. Make it hold by construction instead:

4. **A copy posts native notifications only when its data folder is the default one** (`home_dir()` with no `BAGHOLDER_HOME`), or when it is told `BAGHOLDER_NOTIFY=native`. Any other data folder, which every scratch, test, e2e or capacity copy uses, gets the page's channel, as `BAGHOLDER_NOTIFY=browser` gives today.
   - The container has no desktop, so it is unaffected.
   - A person who keeps their real data elsewhere sets `BAGHOLDER_NOTIFY=native` once. Say so in SPEC notifications-6, beside the rule.
   *Test:* `channel_for` over generated homes and modes. Only the default home, or an explicit `native`, gives a native channel. Every other home gives none, on each OS with a desktop.

## 3. No copy outlives the work that started it

The plan template already ends with **"Nothing left running."** That is a rule to remember, and it was forgotten. Make a scratch copy's life end with its owner's:

5. **`BAGHOLDER_EXIT_WITH=<pid>`:** the server exits cleanly, through its normal stop, the moment that process exits. It is woken by the process's exit itself, never by polling: `pidfd_open` on Linux, a kqueue `EVFILT_PROC`/`NOTE_EXIT` on macOS, and a process handle wait on Windows. No entry in `TIMED_WAITS`.
   - A process already gone at start refuses the start, saying so.
   *Tests:*
   - a server started with the pid of a child the test then kills exits within its stop path, with no wait on a clock;
   - an e2e or capacity copy started by the test harnesses exits when the harness does.
6. **Every way the repository starts a non-default copy sets it:**
   - `CLAUDE.md`'s scratch command sets `BAGHOLDER_EXIT_WITH` to the starting shell's parent, the session;
   - `web/e2e/serve.mjs` sets its own pid;
   - the capacity job and the idle-hour pair set the pid of their runner.

   *Test:* a source scan over `CLAUDE.md`, `web/e2e/*.mjs`, `.github/workflows/*.yml` and `rust/crates/server/src/capacity.rs`. Every start of the server with a `BAGHOLDER_HOME` that isn't the default also names `BAGHOLDER_EXIT_WITH`.
7. **The old Python app:** sessions do not start it (the migration's constraint 5, `briefs/reference.md`). Add one line to `CLAUDE.md`, *Do not*: start the Python app.

## 4. The owner's decision

8. Record the owner's words above in `docs/decisions.md` under 2026-10-19, in the PR that builds this. Held by:
   - the plist test (change 2);
   - the home-channel test (change 4);
   - the exit-with tests (change 5);
   - the source scan (change 6).

## Now, before any of it is built

- **This server:** two scratch servers of the CTO's idle-hour pair were listening on 8797 and 8798, an hour after starting, with no `BAGHOLDER_NOTIFY=browser`. Stop them as soon as their hour is read. Every session lists its own processes (`pgrep -af bagholder`) before it reports a task done.
- **The owner's Mac:** the owner has stopped the two leftover Python servers. The session there confirms nothing of Bagholder's is still running outside the owner's own app.

Build all of it in one PR. It is a change to running, so it carries the container check, and it goes before the rest of the bug list.

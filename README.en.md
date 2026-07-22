**English** · [中文](./README.md)

# ccbar

A tiny macOS 14+ menu bar app that surfaces your **Claude Code + OpenAI
Codex** usage quotas right in the menu bar.

- **Claude and Codex together**: Claude shows Session / Weekly / current
  premium-model limits. Codex rows follow the actual 5-hour, 7-day, and
  model-specific windows returned for your plan. Providers that are not
  currently running disappear within 1 minute, and one failing does not hide the other.
- **Tiny footprint**: 1.0 MB bundle, ~45 MB RAM idle, ~0 % CPU when idle
  (auto-refresh every 5 minutes, also manual ⌘R). Rust + AppKit
  via [`objc2`], ad-hoc signed, no login, no daemon, no telemetry.

[`objc2`]: https://github.com/madsmtm/objc2

```
 ┌─────────────────────────────────────────┐
 │  Claude                   Updated 12:41 │
 │ ─────────────────────────────────────── │
 │  Session    ██░░░░░░      72%  4h 8m    │
 │ ─────────────────────────────────────── │
 │  Weekly     ██████░░      97%  1d 18h   │
 │ ─────────────────────────────────────── │
 │  Fable      ████████      100%          │
 │ ─────────────────────────────────────── │
 │  Codex · PRO              Updated 12:41 │
 │ ─────────────────────────────────────── │
 │  Weekly     ██░░░░░░      97%  6d 12h   │
 │ ─────────────────────────────────────── │
 │  Spark      ░░░░░░░░      100%  7d 0h   │
 │ ─────────────────────────────────────── │
 │  Refresh                           ⌘R   │
 │  Open on GitHub                         │
 │  Quit                              ⌘Q   │
 └─────────────────────────────────────────┘
```

The status bar icon shows at most two meters: Claude's current 5-hour Session
and Codex's current primary window (preferring 5 hours and falling back to the
server-provided primary window). Weekly/model-specific limits and reset times
remain in the expanded menu. Template mode auto-inverts for light/dark bars.

Anthropic caps each Claude account with three independent windows — Session
(5h rolling), Weekly (7d all-models), and a 7-day window carved out for the
current premium model — and any one hitting zero throttles you. ccbar keeps all
three visible so you see which cap is about to hit before firing off that big
task.

The model name on the third row (Fable in the screenshot) is not compiled in.
It is read from `limits[].scope.model.display_name` in the API response, so when
Anthropic rotates the premium model — as it does every few months — ccbar follows
the server without needing a release. See [`REFERENCE.md`](./REFERENCE.md) for
the full window semantics.

Codex is self-describing too. Plans do not necessarily return both a 5-hour
and a 7-day window, so ccbar labels each row from `limit_window_seconds` and
uses `additional_rate_limits[].limit_name` for model-specific rows.

## Install

The current source version is **v0.3.0**, with dynamic Claude + Codex display.
See the source-build command below.

> ### **[⬇ ccbar v0.2.9 — 259 KB zip](https://github.com/GuinsooRocky/ccbar/releases/download/v0.2.9/ccbar-v0.2.9-macos.zip)**
>
> macOS 14+ (Apple Silicon + Intel) · ad-hoc signed · no notarization

> The published v0.2.9 build is Claude-only; build v0.3.0 from source until its
> release is published.

> ⚠️ **Do NOT double-click `ccbar.app` in Downloads / Desktop** after unzipping.
> Move it to `/Applications` first — otherwise macOS runs unsigned apps from a
> read-only **AppTranslocation** sandbox, which pins CPU at 100%. From v0.2.0,
> ccbar detects this and bails out with a dialog instead of hanging.

1. Double-click the zip to unzip, drag `ccbar.app` into `/Applications`.
2. **Recommended**: in Terminal, strip the quarantine attribute — afterwards a
   normal double-click works, no Gatekeeper dialog, no AppTranslocation:
   ```bash
   xattr -rd com.apple.quarantine /Applications/ccbar.app
   ```
   **Alternative** (no Terminal): double-click `ccbar.app` → macOS shows
   *"Apple could not verify ccbar is free of malware"* → **do NOT click
   "Move to Trash"** → open **System Settings → Privacy & Security**, scroll
   to the bottom, click **Open Anyway** next to the ccbar entry, then
   double-click again to confirm. Note: this path may trigger AppTranslocation
   on first launch; if ccbar shows its 100%-CPU warning dialog, follow it and
   run the `xattr` command above.
3. macOS asks to access the `Claude Code-credentials` Keychain item — click
   **Always Allow**. You'll enter your Mac login password once. The ACL is
   attached to `/usr/bin/security` (a signed Apple binary), so subsequent
   launches never prompt again, even after rebuilding ccbar itself.

**Requirements**: at least one supported subscription account. Claude needs
the `claude` CLI signed in with a token carrying `user:profile` (run
`claude setup-token` if needed). Codex needs `codex login` with ChatGPT and
file-backed credentials (normally `~/.codex/auth.json`). API-key-only accounts
do not have the subscription quotas shown here.

Building from source: `cargo build --release && ./packaging/bundle.sh release`.

## How it works

Each signed-in provider makes one HTTP call on launch, every 5 minutes, and on
manual `Refresh` (⌘R). Request volume stays well below a browser dashboard
refresh. Requests run on a worker thread and results are dispatched back to the
main run loop via GCD, so the menu bar stays clickable even when the network
stalls all the way to the 30 s timeout.

```
GET https://api.anthropic.com/api/oauth/usage
    Authorization: Bearer <access_token>
    anthropic-beta: oauth-2025-04-20
    User-Agent: claude-code/2.1.0

GET https://chatgpt.com/backend-api/wham/usage
    Authorization: Bearer <codex_access_token>
    ChatGPT-Account-Id: <account_id>
```

| Response field                                      | Menu row |
|-----------------------------------------------------|----------|
| `limits[]` where `kind = "session"`                 | Session  |
| `limits[]` where `kind = "weekly_all"`              | Weekly   |
| the `limits[]` entry carrying `scope.model`         | that model's `display_name` (e.g. Fable) |
| Codex `rate_limit.primary_window` / `secondary_window` | label derived from `limit_window_seconds` |
| Codex `additional_rate_limits[]`                    | its `limit_name` |

The `limits` array is self-describing — the model name ships with the data. If an
account is served a response without it, ccbar falls back to the older top-level
`five_hour` / `seven_day` / `seven_day_sonnet` / `seven_day_opus` keys.

The access token is read from the macOS Keychain service `Claude
Code-credentials` via `/usr/bin/security`, with a file fallback at
`~/.claude/.credentials.json` and an env override
`CCBAR_CLAUDE_OAUTH_TOKEN`. Codex credentials come from
`$CODEX_HOME/auth.json` (normally `~/.codex/auth.json`), with
`CCBAR_CODEX_OAUTH_TOKEN` and `CCBAR_CODEX_ACCOUNT_ID` overrides. Nothing is
sent to third-party servers: no analytics and no update pings, only the two
official HTTPS endpoints above. Full schema notes are in
[`REFERENCE.md`](./REFERENCE.md).

## Not supported

By design this is a lightweight tool. Multi-account switching, local cost
estimation from Claude/Codex JSONL logs, web cookie / CLI PTY fallbacks,
and Developer ID signing + notarization are all out of scope. If you want
any of those, upstream [CodexBar] is the fuller-featured option.

[CodexBar]: https://github.com/steipete/CodexBar

## Credits

Data-source logic — endpoint, headers, credential locations, and window
semantics — was checked against the official
[`openai/codex`](https://github.com/openai/codex) source and the Swift source of
[`steipete/CodexBar`](https://github.com/steipete/CodexBar) (MIT). No code was
copied; the behavior was re-implemented in Rust.

MIT License.

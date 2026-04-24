**English** · [中文](./README.md)

# ccbar

A tiny macOS 14+ menu bar app that surfaces your **Claude Code** usage
quotas (Session / Weekly / Sonnet) right in the menu bar.

- **For Claude Code users** on a Pro / Max / Team / Enterprise subscription.
  If `claude` is your daily driver, ccbar shows the three rate-limit windows
  that cap your account. API-key-only accounts have no such quotas and
  won't benefit.
- **Tiny footprint**: 2.8 MB bundle, ~45 MB RAM idle, 0 % CPU when idle
  (no automatic polling — HTTP only fires on launch or ⌘R). Rust + AppKit
  via [`objc2`], ad-hoc signed, no login, no daemon, no telemetry.

[`objc2`]: https://github.com/madsmtm/objc2

```
 ┌─────────────────────────────────────────┐
 │  Claude                   Updated 12:41 │
 │ ─────────────────────────────────────── │
 │  Session                  72%  4h 8m    │
 │    ██░░░░░░                             │
 │  Weekly                   97%  1d 18h   │
 │    ██████░░                             │
 │  Sonnet                   100%          │
 │    ████████                             │
 │ ─────────────────────────────────────── │
 │  ⟳  Refresh                       ⌘R    │
 │  📗  Open on GitHub                      │
 │  🗙  Quit                          ⌘Q    │
 └─────────────────────────────────────────┘
```

The status bar icon is a two-bar meter (top = 5-hour Session, bottom = 7-day
Weekly), template-mode so it auto-inverts for light/dark menu bars.

Anthropic caps each Claude account with three independent windows — Session
(5h rolling), Weekly (7d all-models), and Sonnet/Opus (7d premium) — and any
one hitting zero throttles you. ccbar keeps all three visible so you see
which cap is about to hit before firing off that big task. See
[`REFERENCE.md`](./REFERENCE.md) for the full window semantics.

## Install

> ### **[⬇ ccbar v0.1.0 — 1.4 MB zip](https://github.com/GuinsooRocky/ccbar/releases/download/v0.1.0/ccbar-v0.1.0-macos.zip)**
>
> macOS 14+ (Apple Silicon + Intel) · ad-hoc signed · no notarization

1. Double-click the zip to unzip, move `ccbar.app` to `~/Applications` or `/Applications`.
2. Double-click `ccbar.app`. macOS will refuse with a dialog like
   *"Apple could not verify ccbar is free of malware"* — **do NOT click
   "Move to Trash"**. Pick one:
   - **System Settings → Privacy & Security**, scroll all the way to the
     bottom, click **Open Anyway** next to the ccbar entry, then launch
     ccbar again and hit **Open** on the confirmation dialog.
   - Or in Terminal:
     `xattr -rd com.apple.quarantine ~/Applications/ccbar.app`, then
     double-click normally.
3. macOS asks to access the `Claude Code-credentials` Keychain item — click
   **Always Allow**. You'll enter your Mac login password once. The ACL is
   attached to `/usr/bin/security` (a signed Apple binary), so subsequent
   launches never prompt again, even after rebuilding ccbar itself.

**Requirements**: an active Claude **Pro / Max / Team / Enterprise**
subscription, and the `claude` CLI already signed in on this machine so the
Keychain holds a token with the `user:profile` scope (modern `claude login`
produces this by default; if ccbar complains, run `claude setup-token`).
API-key-only accounts have no session/weekly/sonnet quotas to show.

Building from source: `cargo build --release && ./packaging/bundle.sh release`.

## How it works

One HTTP call, only on launch or manual `Refresh` (⌘R) — **no automatic
polling**, so request volume is indistinguishable from a human checking the
dashboard occasionally.

```
GET https://api.anthropic.com/api/oauth/usage
    Authorization: Bearer <access_token>
    anthropic-beta: oauth-2025-04-20
    User-Agent: claude-code/2.1.0
```

| Response field                                      | Menu row |
|-----------------------------------------------------|----------|
| `five_hour`                                         | Session  |
| `seven_day`                                         | Weekly   |
| `seven_day_sonnet` (falls back to `seven_day_opus`) | Sonnet   |

The access token is read from the macOS Keychain service `Claude
Code-credentials` via `/usr/bin/security`, with a file fallback at
`~/.claude/.credentials.json` and an env override
`CCBAR_CLAUDE_OAUTH_TOKEN`. Nothing is sent to third-party servers, no
analytics, no update pings — the one HTTPS call above is the entire network
surface. Full recovered schema in [`REFERENCE.md`](./REFERENCE.md).

## Not supported

By design this is a single-purpose tool. Multi-account switching, automatic
polling timers, local cost estimation from `~/.claude/projects/*.jsonl`, web
cookie / CLI PTY fallbacks, and Developer ID signing + notarization are all
out of scope. If you want any of those, upstream [CodexBar] is the
fuller-featured option.

[CodexBar]: https://github.com/steipete/CodexBar

## Credits

Data-source logic — endpoint, headers, scope requirements, credential
locations, window semantics — was reconstructed by reading the Swift source
of [`steipete/CodexBar`](https://github.com/steipete/CodexBar) (MIT). No
code was copied; the behavior was re-implemented in Rust. The OAuth surface
and its quirks would have been much harder to get right without that
reference.

MIT License.

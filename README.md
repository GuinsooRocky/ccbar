# ccbar

A tiny macOS 14+ menu bar app that shows your **Claude Code** usage quotas
(Session / Weekly / Sonnet) at a glance — no login, no daemon, no telemetry.
Rust + AppKit via [`objc2`], signed ad-hoc, **2.8 MB**.

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

The status bar icon is a tiny two-bar meter: top = 5-hour Session, bottom =
7-day Weekly. Template mode auto-inverts for light/dark menu bars.

## Why

Anthropic caps your Claude Code account with three independent windows and
**any one hitting zero throttles you**:

- **Session (5h rolling)** — the one you hit most
- **Weekly (7d, all models)** — bites for a week if emptied (minor reserve aside)
- **Sonnet / Opus (7d premium-only)** — Haiku still works when this is empty

ccbar keeps all three visible in your menu bar so you know which cap is about
to hit before you fire off that big task. See [`REFERENCE.md`](./REFERENCE.md)
for the full window semantics.

## Requirements

- macOS 14 (Sonoma) or newer — Apple Silicon or Intel
- An active **Claude Pro / Max / Team / Enterprise** subscription
- `claude` CLI already signed in on this machine (creates the OAuth token
  ccbar reads). The token must carry the `user:profile` scope — produced by
  `claude login` on recent versions. If ccbar reports "token missing
  user:profile scope", run `claude setup-token` to refresh it.

API-key-only users have no session/weekly/sonnet quotas to display, so ccbar
is not useful for them.

## Install

### Prebuilt .app

Grab `ccbar.app` from [Releases](https://github.com/GuinsooRocky/ccbar/releases)
(when available), move it to `~/Applications` or `/Applications`, and launch.

### Build from source

```bash
git clone https://github.com/GuinsooRocky/ccbar.git
cd ccbar
cargo build --release
./packaging/bundle.sh release
open ccbar.app
```

Release build is ~2.8 MB (strip + LTO on).

## First run

On first launch macOS will show a Keychain prompt:

> "security" wants to use your confidential information stored in
> "Claude Code-credentials" in your keychain.

Click **Always Allow** (you'll enter your Mac login password once). ccbar
reuses the token that `claude` CLI stored there — nothing is sent to
third-party servers. After that, subsequent launches will never prompt again,
even after rebuilding ccbar itself, because the ACL is attached to
`/usr/bin/security` (a signed Apple binary), not to ccbar.

If you prefer a file over the Keychain, set
`CCBAR_CLAUDE_OAUTH_TOKEN=<access_token>` in your environment instead. See
[`src/credentials.rs`](./src/credentials.rs) for the exact resolution order.

## Features

- Menu bar icon with two live meters (Session + Weekly)
- Session / Weekly / Sonnet-or-Opus rows with percent-left + reset countdown
- `Refresh` (⌘R) triggers a fresh fetch — useful after a big task
- `Open on GitHub` jumps here
- `Quit` (⌘Q)
- **No automatic polling by default** — ccbar only hits the API when launched
  or when you hit Refresh, keeping request volume indistinguishable from a
  human checking the dashboard occasionally

## How it works

One data source: the OAuth-gated usage endpoint that `claude` CLI itself
calls.

```
GET https://api.anthropic.com/api/oauth/usage
    Authorization: Bearer <access_token>
    anthropic-beta: oauth-2025-04-20
    User-Agent: claude-code/2.1.0
```

Response fields we surface:

| Field              | Menu row |
|--------------------|----------|
| `five_hour`        | Session  |
| `seven_day`        | Weekly   |
| `seven_day_sonnet` (falls back to `seven_day_opus`) | Sonnet |

The access token is read from the macOS Keychain service `Claude
Code-credentials`, with a file fallback at `~/.claude/.credentials.json`.
Full recovered schema in [`REFERENCE.md`](./REFERENCE.md).

## Privacy

- No network traffic other than the single `api.anthropic.com` request above,
  triggered only on launch / manual Refresh.
- No analytics, no telemetry, no update pings.
- Your OAuth token never leaves your machine.
- No filesystem scanning beyond `~/.claude/.credentials.json` (file fallback
  only, not default) — we use the Keychain via `/usr/bin/security`.

## Limitations

By design this is a single-purpose tool. Things it **does not** do:

- Multi-account switching
- Local cost estimation from `~/.claude/projects/*.jsonl`
- Automatic polling / timers (on the todo list, currently off to be
  rate-limit-friendly)
- Web cookie fallback, CLI PTY fallback
- Developer ID code signing / notarization (ad-hoc only; Gatekeeper may ask
  on first open — right-click → Open)

If you want any of those, upstream [CodexBar] is the fuller-featured option.

[CodexBar]: https://github.com/steipete/CodexBar

## Acknowledgments

Data-source logic (endpoint, headers, scope requirements, credential
locations, window semantics) was reconstructed by reading the Swift source of
[`steipete/CodexBar`](https://github.com/steipete/CodexBar) (MIT). No code was
copied — the behavior was re-implemented in Rust — but the OAuth surface and
its quirks would have been much harder to get right without that reference.

## License

MIT

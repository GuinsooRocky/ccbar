# ccbar — Claude quota data source reference

Distilled from `steipete/CodexBar` (MIT), then re-verified against the live
endpoint on 2026-07-13. Only the subset needed for the "Claude title + Session +
Weekly + premium-model + Quit" menu.

## Scope for ccbar

- Data source: **OAuth API only**. No Web cookie, no CLI PTY, no local JSONL
  cost scan.
- Menu surface: Claude title, Session, Weekly, the current premium model's
  window (name read from the response, not hardcoded), Quit.
- Everything below that CodexBar does beyond this (Web fallback, CLI fallback,
  extra_usage, multi-account, watchdog process) is intentionally out of scope.

## OAuth usage endpoint

- `GET https://api.anthropic.com/api/oauth/usage`
- Required headers:
  - `Authorization: Bearer <access_token>`
  - `anthropic-beta: oauth-2025-04-20`
  - `Accept: application/json`
  - `Content-Type: application/json`
  - `User-Agent: claude-code/<version>` (fallback `claude-code/2.1.0`)
- Required scope on the token: `user:profile`. Tokens issued by `claude login`
  usually have `user:inference` only — they will 403 with body containing
  `user:profile`. Fix: user runs `claude setup-token`.

### Response JSON (snake_case)

```
{
  "five_hour":         { "utilization": 98.0, "resets_at": "2026-07-13T08:50:00Z" },
  "seven_day":         { "utilization": 22.0, "resets_at": "2026-07-18T22:00:00Z" },

  // Per-model top-level keys. All null as of 2026-07 — superseded by `limits`.
  // Anthropic also ships unreleased codenames here (seven_day_cowork,
  // seven_day_omelette, tangelo, iguana_necktie, nimbus_quill, cinder_cove,
  // amber_ladder …). Do not chase these; read `limits` instead.
  "seven_day_sonnet":  null,
  "seven_day_opus":    null,

  // Current shape: self-describing, one entry per active limit.
  "limits": [
    { "kind": "session",       "group": "session", "percent": 98, "severity": "critical",
      "resets_at": "2026-07-13T08:50:00Z", "scope": null, "is_active": true },
    { "kind": "weekly_all",    "group": "weekly",  "percent": 22, "severity": "normal",
      "resets_at": "2026-07-18T22:00:00Z", "scope": null, "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly",  "percent": 15, "severity": "normal",
      "resets_at": "2026-07-18T22:00:00Z", "is_active": false,
      "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null } }
  ],

  "extra_usage":       { "is_enabled": true, "monthly_limit": 2000, "used_credits": 0, "currency": "USD" }
}
```

- **`limits` is the field to read.** Each entry names its own window via `kind`,
  and the premium-model entry carries the model's `display_name`. Because the
  name travels with the data, rotating the premium model (Opus → Sonnet → Fable →
  …) needs no ccbar release. Hardcoding model field names does not survive.
- `percent` (in `limits`) and `utilization` (top-level) are both **used**
  percentages on a 0..100 scale, not a 0..1 fraction. UI shows `100 - percent`
  as "% left".
- `severity` (`normal` / `critical`) and `is_active` are carried but unused by
  ccbar today.
- `resets_at` = ISO-8601 UTC. Parse with `chrono::DateTime<Utc>`. For the
  session bar CodexBar displays the wall-clock time in the token's timezone
  (e.g. "Resets 2am (Asia/Tokyo)"). MVP can just format in local time.
- `extra_usage.used_credits` and `monthly_limit` are **in cents** — divide by
  100 for dollars (CodexBar `ClaudeUsageFetcher.normalizeClaudeExtraUsageAmounts`).

### Field mapping to menu

| Menu row                    | JSON source                                  |
|-----------------------------|----------------------------------------------|
| Session                     | `limits[]` where `kind == "session"`         |
| Weekly                      | `limits[]` where `kind == "weekly_all"`      |
| *(model's `display_name`)*  | the `limits[]` entry carrying `scope.model`  |

Fallback for responses served without `limits` (older accounts): `five_hour`,
`seven_day`, and `seven_day_sonnet ?? seven_day_opus` (labelled "Sonnet"/"Opus").
The row is hidden entirely when no scoped window exists in either shape.

## Credentials

Two sources, checked in order:

### 1. macOS Keychain (primary)

- Service: `Claude Code-credentials`
- Account: user's email (present in the item record)
- Query: `SecItemCopyMatching` with `kSecClass = kSecClassGenericPassword`
  and `kSecAttrService = "Claude Code-credentials"`.
- Value (`kSecValueData`) is JSON with shape:

```
{
  "claudeAiOauth": {
    "accessToken":  "sk-ant-oat01-...",
    "refreshToken": "sk-ant-ort01-...",
    "expiresAt":    1735689600000,          // epoch ms
    "scopes":       ["user:inference", "user:profile"],
    "rateLimitTier": "claude_max_20x"       // or claude_pro_*, claude_team_*, claude_enterprise_*
  }
}
```

### 2. File fallback

- `$HOME/.claude/.credentials.json`
- Same top-level `claudeAiOauth` shape.

### 3. Environment overrides (useful for ccbar dev)

- `CODEXBAR_CLAUDE_OAUTH_TOKEN` — raw access token
- `CODEXBAR_CLAUDE_OAUTH_SCOPES` — comma-separated, e.g. `user:inference,user:profile`
- For ccbar, rename to `CCBAR_CLAUDE_OAUTH_TOKEN` etc.

## Token refresh

When `accessToken` is expired (`now >= expiresAt`):

- `POST https://platform.claude.com/v1/oauth/token`
- Body (JSON):

```
{
  "grant_type":    "refresh_token",
  "refresh_token": "<current refresh token>",
  "client_id":     "9d1c250a-e61b-44d9-88ed-5944d1962f5e"
}
```

- Response contains `access_token`, `refresh_token` (may be rotated),
  `expires_in` (seconds).
- Writing the refreshed token back to Keychain requires re-authorising the
  Keychain item — CodexBar avoids this by delegating refresh to `claude` CLI
  (it writes back to the same Keychain entry). MVP strategy: try refresh,
  on success update in-memory only, and let the user run `claude` to refresh
  persistently. Can harden later.

## Plan inference

From `rateLimitTier` string, lowercased `contains()`:

- `max` → Max
- `pro` → Pro
- `team` → Team
- `enterprise` → Enterprise
- else → None

## Selection order (CodexBar for reference; ccbar does not implement)

- App runtime: OAuth → CLI PTY → Web API
- CLI runtime: Web API → CLI PTY

## Window semantics — what the three bars actually mean

The three bars Claude exposes via `api/oauth/usage` are **AND-gated** — any one
hitting zero throttles you. They're independent ceilings, not a single budget.

### Session — `five_hour`

- 5-hour rolling window starting at your first message of a new window.
- **Not tied to subscription plan** — every claude.ai user has a session
  window. Pro/Max just get a bigger bucket inside it.
- This is the most common bottleneck in daily use. When it empties, wait it
  out (≤5h) and the window rolls, no action required.

### Weekly (all models) — `seven_day`

- 7-day rolling window, sum of **all models** including Haiku.
- For Max/Pro users the `/usage` panel also prints a "reserve" line
  (e.g. "11% in reserve · Lasts until reset"). That text comes straight from
  the Claude CLI output — **CodexBar does not parse it into structured fields**
  (verified: no business logic for "reserve" in `Providers/Claude/`).
  The number is user-specific and not a documented constant. The OAuth
  `utilization` we read does **not** expose the reserve split — it is
  presentation only, surfaced by the CLI PTY path, not the OAuth API.
- When Weekly empties you're rate-limited for a week (minus reserve
  spend-down), even if Session is full.

### Premium model — `limits[]` with `kind == "weekly_scoped"`

- 7-day window specifically for the current premium model tier.
- Anthropic carves this out so a Max user cannot burn an entire week on the
  top model.
- The model rotates (Opus → Sonnet → Fable as of 2026-07). Its name is in
  `scope.model.display_name`; render that rather than a compiled-in string.
- Historically this came through the top-level `seven_day_sonnet` /
  `seven_day_opus` keys, of which at most one was populated. Those are null
  now — keep them only as a fallback.
- When this empties you can still use **Haiku** — it doesn't charge against
  this bar, only against Session + Weekly (all models).

### Throttle interactions

| Drained first | Effect | Workaround |
|---|---|---|
| Session | Rate-limited for the remainder of the 5h window | Wait |
| Weekly (all models) | Rate-limited for the rest of the 7d (reserve may cover briefly) | Wait, or switch account |
| Premium model | That model blocked, everything else still works | Downgrade to Haiku |

### What ccbar displays from these

- Menu row "Session" ← the `session` entry in `limits`
- Menu row "Weekly"  ← the `weekly_all` entry in `limits`
- Menu row named after the model (e.g. "Fable") ← the `limits` entry with
  `scope.model`, labelled with its `display_name`
- Reset countdowns: parse ISO-8601 `resets_at`, diff against `now`,
  render as "Resets in Xh Ym" or "Resets Xam (tz)" for same-day absolute
  anchors.
- The "reserve" and "Lasts until reset" sublines are **not in MVP** because
  the OAuth path doesn't expose them. If we ever want them, we'd have to add
  the CLI PTY source — out of scope for ccbar.

## Source files referenced

- `docs/claude.md`
- `Sources/CodexBarCore/Providers/Claude/ClaudeUsageFetcher.swift`
- `Sources/CodexBarCore/Providers/Claude/ClaudeSourcePlanner.swift`
- `Sources/CodexBarCore/Providers/Claude/ClaudeUsageDataSource.swift`
- `Sources/CodexBarCore/Providers/Claude/ClaudePlan.swift`
- `Sources/CodexBarCore/Providers/Claude/ClaudeOAuth/ClaudeOAuthUsageFetcher.swift`
- `Sources/CodexBarCore/Providers/Claude/ClaudeOAuth/ClaudeOAuthCredentials.swift`

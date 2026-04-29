[English](./README.en.md) · **中文**

# ccbar

一个小巧的 macOS 14+ 菜单栏 App，把你的 **Claude Code** 用量配额
（Session / Weekly / Sonnet）直接显示在菜单栏上。

- **面向 Claude Code 用户** — 订阅了 Pro / Max / Team / Enterprise 套餐、
  日常用 `claude` CLI 的人，ccbar 会把限制你账号的三个 rate-limit 窗口
  直接显示在菜单栏。只用 API key 的账号没有这些配额，ccbar 对他们没意义。
- **体积极小**：1.0 MB 安装包，空闲约 45 MB 常驻内存、接近 0 % CPU
  （每 5 分钟自动刷新一次，也可按 ⌘R 手动刷新）。Rust + AppKit
  via [`objc2`]，ad-hoc 签名，无登录、无后台进程、无遥测。

[`objc2`]: https://github.com/madsmtm/objc2

```
 ┌─────────────────────────────────────────┐
 │  Claude                   Updated 12:41 │
 │ ─────────────────────────────────────── │
 │  Session    ██░░░░░░      72%  4h 8m    │
 │ ─────────────────────────────────────── │
 │  Weekly     ██████░░      97%  1d 18h   │
 │ ─────────────────────────────────────── │
 │  Sonnet     ████████      100%          │
 │ ─────────────────────────────────────── │
 │  Refresh                           ⌘R   │
 │  Open on GitHub                         │
 │  Quit                              ⌘Q   │
 └─────────────────────────────────────────┘
```

菜单栏图标是两条横向 meter（上条 = 5 小时 Session，下条 = 7 天 Weekly），
template 模式下会根据菜单栏深/浅色自动反色。

Anthropic 对每个 Claude 账号设三个独立的配额窗口 —— Session（5 小时滚动）、
Weekly（7 天所有模型累计）、Sonnet/Opus（7 天顶级模型专用）—— 任何一个见底
都会让你被限流。ccbar 把三个窗口同时显示，让你在跑大任务前先知道哪条最紧。
窗口语义详见 [`REFERENCE.md`](./REFERENCE.md)（英文）。

## 安装

> ### **[⬇ ccbar v0.2.7 — 988 KB zip](https://github.com/GuinsooRocky/ccbar/releases/download/v0.2.7/ccbar-v0.2.7-macos.zip)**
>
> macOS 14+（Apple Silicon + Intel）· ad-hoc 签名 · 未做 notarization

> ⚠️ **不要在 Downloads / Desktop 里原地双击**解压出来的 `ccbar.app`。
> 必须先拖到 `/Applications`，否则 macOS 会把未签名 app 放进 **AppTranslocation
> 只读沙盒**运行，导致 CPU 持续 100%。v0.2.0 起，ccbar 检测到自己在
> AppTranslocation 路径时会直接弹窗并退出。

1. 双击解压 zip，把 `ccbar.app` 拖到 `/Applications`。
2. **推荐**：在终端执行一次命令解除 quarantine，之后双击打开即可、无弹窗、无
   AppTranslocation：
   ```bash
   xattr -rd com.apple.quarantine /Applications/ccbar.app
   ```
   **备选**（不想开终端）：直接双击 `ccbar.app` → macOS 弹 *"Apple 无法验证
   ccbar 是否包含恶意软件"* → **千万不要点"移到废纸篓"** → 进 **系统设置 →
   隐私与安全性** → 滚动到底部，点 ccbar 旁边的 **仍要打开**，再次双击确认。
   注意此路径首启有概率进入 AppTranslocation 沙盒，若 ccbar 自检弹窗告知
   CPU 100%，请按弹窗提示改用上面的 `xattr` 命令。
3. macOS 会弹窗请求访问 `Claude Code-credentials` 钥匙串条目 —— 点
   **始终允许**，输入一次 Mac 登录密码。ACL 绑定在 `/usr/bin/security`
   （Apple 自签的系统二进制）上，所以重新编译 ccbar 也不会再弹。

**前置条件**：开通 Claude **Pro / Max / Team / Enterprise** 订阅，并在
本机用 `claude` CLI 登录过（让钥匙串里保留带 `user:profile` scope 的 token）。
新版 `claude login` 默认产出 `user:profile`；如果 ccbar 报错缺 scope，跑
`claude setup-token` 重新签发。只用 API key 的账号不存在 session/weekly/sonnet
配额，ccbar 对他们没意义。

源码构建：`cargo build --release && ./packaging/bundle.sh release`。

## 工作原理

只有一个 HTTP 请求，在启动时、每 5 分钟自动触发、以及手动 `Refresh`（⌘R）
时发出。请求频率远低于浏览器刷 dashboard。请求跑在后台线程上，刷新结果通过
GCD 主队列回到主线程更新 UI —— 即使网络卡到 30 秒超时，菜单栏点击也不会卡。

```
GET https://api.anthropic.com/api/oauth/usage
    Authorization: Bearer <access_token>
    anthropic-beta: oauth-2025-04-20
    User-Agent: claude-code/2.1.0
```

| 响应字段                                                | 菜单栏行 |
|-------------------------------------------------------|----------|
| `five_hour`                                           | Session  |
| `seven_day`                                           | Weekly   |
| `seven_day_sonnet`（缺失时回退到 `seven_day_opus`）  | Sonnet   |

Access token 从 macOS 钥匙串 service `Claude Code-credentials` 经
`/usr/bin/security` 读取；回退路径是 `~/.claude/.credentials.json` 文件；
也可以用环境变量 `CCBAR_CLAUDE_OAUTH_TOKEN` 直接覆盖。ccbar 不向任何第三方
服务器发数据，没有分析埋点、没有更新检查 —— 网络出口只有上面那一个 HTTPS
请求。还原出的完整 schema 见 [`REFERENCE.md`](./REFERENCE.md)。

## 不支持

ccbar 按单功能工具设计，以下都不做：多账号切换、从
`~/.claude/projects/*.jsonl` 做本地成本估算、浏览器 cookie / CLI PTY 回退
路径、Developer ID 签名 + notarization。想要这些，上游的 [CodexBar] 更全。

[CodexBar]: https://github.com/steipete/CodexBar

## 致谢

取数逻辑（endpoint、header、scope 要求、凭证位置、窗口语义）是通过阅读
[`steipete/CodexBar`](https://github.com/steipete/CodexBar)（MIT）的 Swift
源码还原出来的。没有复制代码，用 Rust 重新实现了同样的行为，但如果没有那份
参考，OAuth 接口和它的各种坑要搞清楚会困难得多。

MIT License.

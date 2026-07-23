[English](./README.en.md) · **中文**

# ccbar

一个小巧的 macOS 14+ 菜单栏 App，把你的 **Claude Code + OpenAI Codex**
用量配额直接显示在菜单栏上。

- **同时看 Claude 与 Codex** — Claude 展示 Session / Weekly / 当前顶级模型；
  Codex 只展示所有主力模型共用的 7 天 Weekly 额度，不显示模型专属窗口。
  当前未运行的服务会在 1 分钟内整块隐藏；任一服务请求失败时，另一边仍可正常显示。
- **体积极小**：1.0 MB 安装包，空闲约 45 MB 常驻内存、接近 0 % CPU
  （每 5 分钟自动刷新一次，也可按 ⌘R 手动刷新）。Rust + AppKit
  via [`objc2`]，ad-hoc 签名，无登录、无后台进程、无遥测。

[`objc2`]: https://github.com/madsmtm/objc2

```
 ┌─────────────────────────────────────┐
 │  Claude                     ↻ 18:35 │
 │  Session     ────────        100%   │
 │  Weekly      ███████─    15%  2d 12h│
 │  Fable       █████───    34%  2d 12h│
 │ ─────────────────────────────────── │
 │  Codex                      ↻ 18:35 │
 │  Weekly      ██████──     29%  5d 7h│
 │ ─────────────────────────────────── │
 │   [ Refresh ]  [ GitHub ]  [ Quit ] │
 └─────────────────────────────────────┘
```

菜单栏图标按当前活跃的服务显示 Weekly 额度：Claude 和 Codex 都活跃时显示两条，
只有一个活跃时显示一条并垂直居中；都不活跃时显示一个低对比度的半透明实心方块。
点开后，菜单气泡中心会对齐状态栏图标。额度条的粗段表示已用，细段表示剩余；
八段外观按像素绘制，粗细交界仍跟随真实百分比，不再取整成整字符格。右侧数字显示
剩余额度和重置倒计时。底部的 Refresh / GitHub / Quit 收在同一行三个圆角按钮里，
其中 Refresh 和 Quit 仍支持 ⌘R / ⌘Q。template 模式会根据菜单栏深/浅色自动反色。

Anthropic 对每个 Claude 账号设三个独立的配额窗口 —— Session（5 小时滚动）、
Weekly（7 天所有模型累计）、以及一个给当前顶级模型单开的 7 天窗口 —— 任何一个
见底都会让你被限流。ccbar 把三个窗口同时显示，让你在跑大任务前先知道哪条最紧。

第三行的模型名（截图里是 Fable）不是写死的，而是从 API 的
`limits[].scope.model.display_name` 现读的：Anthropic 隔几个月换一次顶级模型，
ccbar 跟着服务端返回的名字走，不用发新版本。窗口语义详见
[`REFERENCE.md`](./REFERENCE.md)（英文）。

Codex 同样使用服务端自描述数据。ccbar 只从主 `rate_limit` 中选取
`limit_window_seconds = 604800` 的 7 天窗口；`additional_rate_limits`
中的模型专属额度会被忽略。

## 安装

当前源码版本为 **v0.3.1**，支持 Claude + Codex 动态显示；菜单栏图标会按当前活跃
服务展示各自的 Weekly 额度。源码构建命令见下方。

> ### **[⬇ ccbar v0.2.9 — 259 KB zip](https://github.com/GuinsooRocky/ccbar/releases/download/v0.2.9/ccbar-v0.2.9-macos.zip)**
>
> macOS 14+（Apple Silicon + Intel）· ad-hoc 签名 · 未做 notarization

> 已发布的 v0.2.9 安装包只含 Claude；v0.3.1 发布前请从源码构建新版。

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

**前置条件**：至少登录一个受支持的订阅账号。Claude 需要在本机用 `claude`
CLI 登录过，并持有带 `user:profile` scope 的 token；缺 scope 时运行
`claude setup-token`。Codex 需要通过 `codex login` 登录 ChatGPT 订阅账号，并使用
文件凭证（默认 `~/.codex/auth.json`）。只用 API key 的账号没有这里展示的订阅限额。

源码构建：`cargo build --release && ./packaging/bundle.sh release`。

### 本机一次性改造收尾规范

如果仓库只是临时克隆到本机做一次改造，完成后按下面顺序收尾：

1. 相关测试通过后，先提交功能改造并 push。
2. 同步 README，再单独提交并 push，保持远端历史清楚。
3. 用最终源码构建 release，安装到 `/Applications/ccbar.app`，完成签名、启动和
   实际界面验证。
4. 确认远端分支包含全部提交、本地没有未提交改动后，清理构建缓存和 ccbar
   应用缓存。
5. 只有用户明确确认不再保留本地源码时，才把整个仓库移入 macOS 废纸篓；
   不永久删除，也不影响已经安装的 App。

## 工作原理

每个已登录服务各发一个 HTTP 请求，在启动时、每 5 分钟自动触发、以及手动
`Refresh`（⌘R）时发出。请求频率远低于浏览器刷 dashboard。请求跑在后台线程上，
刷新结果通过 GCD 主队列回到主线程更新 UI —— 即使网络卡到 30 秒超时，菜单栏
点击也不会卡。

```
GET https://api.anthropic.com/api/oauth/usage
    Authorization: Bearer <access_token>
    anthropic-beta: oauth-2025-04-20
    User-Agent: claude-code/2.1.0

GET https://chatgpt.com/backend-api/wham/usage
    Authorization: Bearer <codex_access_token>
    ChatGPT-Account-Id: <account_id>
```

| 响应字段                                                | 菜单栏行 |
|-------------------------------------------------------|----------|
| `limits[]`，`kind = "session"`                        | Session  |
| `limits[]`，`kind = "weekly_all"`                     | Weekly   |
| `limits[]` 中带 `scope.model` 的那条                  | 该模型的 `display_name`（如 Fable） |
| Codex 主 `rate_limit` 中的 7 天窗口              | Weekly   |
| Codex `additional_rate_limits[]`                       | 不显示   |

`limits` 数组是自描述的，模型名随数据下发。老账号若拿不到 `limits`，会回退到
早期的顶层字段 `five_hour` / `seven_day` / `seven_day_sonnet` / `seven_day_opus`。

Access token 从 macOS 钥匙串 service `Claude Code-credentials` 经
`/usr/bin/security` 读取；回退路径是 `~/.claude/.credentials.json` 文件；
也可以用环境变量 `CCBAR_CLAUDE_OAUTH_TOKEN` 直接覆盖。Codex 凭证读取自
`$CODEX_HOME/auth.json`（默认 `~/.codex/auth.json`），也可用
`CCBAR_CODEX_OAUTH_TOKEN` 和 `CCBAR_CODEX_ACCOUNT_ID` 覆盖。ccbar 不向第三方
服务器发数据，没有分析埋点、没有更新检查；网络出口只有上面两个官方 HTTPS
接口。还原出的 schema 见 [`REFERENCE.md`](./REFERENCE.md)。

## 不支持

ccbar 按轻量工具设计，以下都不做：多账号切换、从 Claude / Codex 本地 JSONL
做 token 成本估算、浏览器 cookie / CLI PTY 回退
路径、Developer ID 签名 + notarization。想要这些，上游的 [CodexBar] 更全。

[CodexBar]: https://github.com/steipete/CodexBar

## 致谢

取数逻辑（endpoint、header、scope 要求、凭证位置、窗口语义）参考了 OpenAI
官方 [`openai/codex`](https://github.com/openai/codex) 源码，并通过阅读
[`steipete/CodexBar`](https://github.com/steipete/CodexBar)（MIT）的 Swift
源码还原出来的。没有复制代码，用 Rust 重新实现了同样的行为，但如果没有那份
参考，OAuth 接口和它的各种坑要搞清楚会困难得多。

MIT License.

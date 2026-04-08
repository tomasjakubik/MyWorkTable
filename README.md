# MyWorkTable

A lightweight desktop dashboard for managing todos and monitoring [Claude Code](https://docs.anthropic.com/en/docs/claude-code) sessions. Built with Rust (Axum) and served as a local web app.

Totally vibecoded. Didn't even read it.

![Screenshot](screenshot.png)

## Features

- **Todos** — create, edit, reorder, and check off tasks with markdown notes
- **Claude Code sessions** — see active sessions with their status (working / waiting for approval / stale / ended), model, project path, and git branch
- **Rate limits** — global 5-hour and 7-day API rate limit bars with reset times, polled from the Anthropic API
- **Task tracking** — shows Claude's internal task progress (from TodoWrite) per session
- **Stale detection** — flags sessions that haven't received events for 10+ minutes
- **Installable** — works as a standalone PWA via the web app manifest

## Installation

The install script builds the binary, configures Claude Code hooks, and sets up a systemd user service. Requires `cargo`, `jq`, and `systemctl`.

```bash
./install.sh [SERVER_HOST]
```

`SERVER_HOST` defaults to `localhost`. If Claude Code runs inside a Docker container that resolves `dockerhost` to the host, use:

```bash
./install.sh dockerhost
```

The script is idempotent — safe to re-run after pulling updates.

After installation:

- Dashboard: [http://localhost:5548](http://localhost:5548)
- Service: `systemctl --user status myworktable`
- Hooks are configured in `~/.claude/settings.json`

### Manual setup

If you prefer not to use the install script:

1. Build: `cargo build --release`
2. Merge [claude-settings.json](claude-settings.json) into `~/.claude/settings.json`
3. Run the binary from the repo root: `./target/release/server`

## Claude Code Integration

### Hooks

The server listens for HTTP hook events at `/hooks/{event}`:

`SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `PreToolUse`, `PostToolUse`, `PostCompact`, `Notification`, `Stop`, `SubagentStart`, `SubagentStop`, `SessionEnd`, `TaskCreated`, `TaskCompleted`, `CwdChanged`

Event-to-status mapping:

| Pattern in event name | Session status       |
| --------------------- | -------------------- |
| `Stop`, `End`         | ended                |
| `PermissionRequest`   | waiting for approval |
| anything else         | working              |

The first `UserPromptSubmit` sets the session title. `CwdChanged` updates the working directory. `TaskCreated`/`TaskCompleted` track Claude's internal task progress.

### Rate limits

The server polls `https://api.anthropic.com/api/oauth/usage` every 60 seconds using the OAuth token from `~/.claude/.credentials.json`. This feeds the 5-hour and 7-day rate limit bars in the dashboard header.

## Stack

- [Axum](https://github.com/tokio-rs/axum) — HTTP server
- [SQLx](https://github.com/launchbadge/sqlx) + SQLite — persistence
- [Askama](https://github.com/djc/askama) — HTML templates
- [HTMX](https://htmx.org) + [Sortable.js](https://sortablejs.github.io/Sortable/) — interactivity
- [Tailwind CSS](https://tailwindcss.com) — styling

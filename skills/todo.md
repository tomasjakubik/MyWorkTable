---
name: todo
description: Manage todos in the MyWorkTable dashboard
user_invocable: true
---

# /todo skill

Manage todos via the MyWorkTable dashboard REST API at http://127.0.0.1:5544.

Each todo has a **title** (one-line summary) and an optional **note** (multiline markdown description).

## Usage

The user will invoke this skill as `/todo <action> <args>`.

### Actions

- `/todo add <title>` — Create a new todo with the given title
- `/todo add <title> | <note>` — Create a todo with title and note (pipe-separated)
- `/todo note <id> <text>` — Set or update the note on an existing todo
- `/todo done <id>` — Mark a todo as completed
- `/todo undo <id>` — Unmark a completed todo
- `/todo delete <id>` — Delete a todo
- `/todo list` — List all todos

## Implementation

Use the Bash tool to execute curl commands against the MyWorkTable API.

### Add a todo

```bash
curl -s -X POST http://127.0.0.1:5544/api/todos \
  -H 'Content-Type: application/json' \
  -d '{"text": "<title>", "note": "<optional note>", "session_id": "'"$CLAUDE_SESSION_ID"'"}'
```

The `note` field is optional. Omit it to create a todo with just a title.
Always include `session_id` with the current `$CLAUDE_SESSION_ID` so the todo is auto-linked to this session.

### Update title or note

```bash
curl -s -X POST http://127.0.0.1:5544/api/todos/<id>/update-json \
  -H 'Content-Type: application/json' \
  -d '{"text": "<new title>", "note": "<new note>"}'
```

Both fields are optional — omit either to leave it unchanged.

### Complete a todo

```bash
curl -s -X POST http://127.0.0.1:5544/api/todos/<id>/done
```

### List todos

```bash
curl -s http://127.0.0.1:5544/api/todos | python3 -m json.tool
```

### Delete a todo

```bash
curl -s -X DELETE http://127.0.0.1:5544/api/todos/<id>
```

## Behavior

- When adding, confirm with the todo title, ID, and session ID returned.
- When listing, format as a readable list showing ID, status, title, and note preview.
- When completing, confirm the todo was marked done.
- The note field supports markdown.
- If the API is unreachable, tell the user to start MyWorkTable first (`cargo run --bin server` or `make run` in the MyWorkTable directory).

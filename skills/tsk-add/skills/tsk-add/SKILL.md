---
name: tsk-add
description: Queue a single task based on the current conversation using tsk add
argument-hint: [optional additional context]
user-invocable: true
disable-model-invocation: true
allowed-tools:
  - Bash(tsk help)
  - Bash(tsk add --help)
  - Bash(tsk template list)
  - Bash(tsk template show)
---

# Queue Task from Conversation

You are helping the user queue a task based on the conversation you just had. The user has been discussing a design, architecture decision, or code change with you, and now wants to queue it as an asynchronous task.

## Available Templates

These are the task templates available for use with `tsk add -t <template>`:

```
!`tsk template list`
```

To see the full content of a template, run `tsk template show <template>`. When a task is created, the `{{PROMPT}}` placeholder in the template is replaced with whatever you pipe in via heredoc or pass via the `-p` flag, and any YAML frontmatter is stripped. The result is what gets sent to the agent.

## Your Job

1. **Summarize the agreed-upon work**: Review the conversation above and identify:
   - What change or feature was discussed
   - Key design decisions that were made
   - Any specific files, components, or areas mentioned
   - Acceptance criteria or requirements that were established
   - Ask clarifying questions about requirements, architecture choices, or task details
   - Only proceed once agreement has been reached

2. **Create a task name**: Come up with a short, branch-friendly name (e.g., `add-auth`, `refactor-api`, `fix-validation`)

3. **Pick the best template**: Based on the nature of the task, choose the template from the list above that best matches the work to be done (e.g., `feat` for new features, `fix` for bug fixes, `refactor` for restructuring, `doc` for documentation).

4. **Write the task description**: Create a clear, self-contained description that includes:
   - The goal of the change
   - Key design decisions from the conversation
   - Specific files or areas to focus on (if discussed)
   - Acceptance criteria

5. **Queue the task**: Use `tsk add` to create the task:

```bash
tsk add -t <template> -n "<task-name>" <<'EOF'
<task description>
EOF
```

## Guidelines

- Include enough context that an agent without access to this conversation can understand and execute the task
- Reference specific files, functions, or architectural patterns discussed in the conversation
- If the user provided additional context via $ARGUMENTS, incorporate that as well
- Keep the description concise but complete - the agent can explore the codebase for details
- Either pipe in input using HEREDOC format OR use the `-p` flag. They do not work together.

## Example Output

```bash
tsk add -t feat -n "add-rate-limiting" <<'EOF'
Add rate limiting to the API endpoints.

### Context
The API currently has no rate limiting, which could allow abuse. We discussed using a token bucket algorithm with Redis for distributed rate limiting.

### Requirements
- Add rate limiting middleware to all /api/* routes
- Use token bucket algorithm: 100 requests per minute per API key
- Store rate limit state in Redis (connection config already exists)
- Return 429 Too Many Requests with Retry-After header when limit exceeded

### Files to Focus On
- src/middleware/ - add new rate limiting middleware
- src/routes/api.ts - apply middleware to routes
- src/config/redis.ts - reuse existing Redis connection

### Acceptance Criteria
- Rate limiting works correctly under load
- Proper error responses with Retry-After header
- Tests cover rate limit enforcement and reset behavior
EOF
```

After queuing the task, remind the user they can run `tsk server start` if the server isn't already running.

## How To Use tsk

`tsk` is installed and on the path. Here are the main commands:
```
!`tsk help`
```

Here are the options for adding tasks:
```
!`tsk add --help`
```

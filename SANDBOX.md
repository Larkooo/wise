# wise sandbox — optional CLI policies for automation

> A sandbox is a TOML policy that constrains what `wise` commands can do.
> Set `WISE_SANDBOX=<name>` (or pass `--sandbox <name>`) and every subsequent
> CLI invocation in that environment is filtered through the policy.

The motivating use case is **giving automation, including LLM agents, scoped
Wise access** without inventing a parallel command tree. The caller runs the
regular `wise` CLI; the sandbox decides what that means.

This document is the design + safety contract for the sandbox primitive.
The agent-card use case that sits on top of it is documented separately in
[`AGENT.md`](AGENT.md).

Status: **implemented in the CLI; `tty` and `command` escalation modes remain pending.**

---

## Why a generic sandbox instead of a separate automation subtree

Earlier drafts carved out `wise agent ...` as its own command group. The
sandbox model is strictly better:

1. **Reuses the existing CLI.** Agent calls `wise balance get` like any
   other caller; sandbox handles the policy. No parallel tree.
2. **Composable.** Any future agent feature (research bot, scheduling
   bot, payouts bot) gets a different sandbox file — no new code.
3. **Familiar mental model.** `sudo` policies, k8s RBAC, Linux capabilities.
4. **Inspectable.** "What can this agent do?" → read one TOML file.
5. **Multi-agent safe.** Multiple sandboxes can coexist; switching is one
   env var.

---

## Activation

A sandbox is active when *either* of these is true:

- `WISE_SANDBOX=<name>` is set in the process environment, or
- `--sandbox <name>` is passed on the command line.

The CLI loads `~/.config/wise/sandboxes/<name>.toml`, validates it, and
applies the policy to *every* command in that invocation.

> **The sandbox is activated, not entered.** There is no shell hook that
> can be `unset`'d from inside an agent prompt to escape it. The agent's
> environment is set by whatever wraps it (the user's shell init, a
> systemd unit, a docker container) and persists across `wise` invocations.

Inside the sandbox, the CLI sets a `WISE_SANDBOX_ACTIVE=1` env var and
prepends `[sandbox:<name>] ` to any verbose log lines so it's obvious which
policy is in effect.

---

## Sandbox config schema

Stored at `~/.config/wise/sandboxes/<name>.toml`. Mode `0600`. Schema:

```toml
# Required identity
name        = "coding-agent"
description = "Pays for SaaS APIs while drafting code"

# Resource scoping. Even allowed commands only see these resources.
# Empty / missing = no scoping (don't do this for an actual agent).
profiles = [73459809]
cards    = ["tok_abc..."]
balances = [12345678]

# Command allow-list. Wildcards supported (see "command paths" below).
allow = [
  "balance.list",
  "balance.get",
  "card.get",
  "card.freeze",          # alias for `card.status` with status=FROZEN
  "rate.get",
  "currency.list",
  "docs.ask",
  "agent.fetch",          # the JWE PAN-fetch flow
]

# Hard denies override allows. Useful for narrowing wildcards.
deny = [
  "balance.move",
  "transfer.create",
  "card.permissions.set",
  "card.unfreeze",
]

# Per-command conditions. Evaluated inside the command handler.
[conditions."agent.fetch"]
rate_limit            = "3/hour"
require_justification = true       # --justify "..." mandatory
audit                 = "~/.config/wise/agent-audit.log"

# Escalation — what happens when a denied command is called with --sudo
[escalation]
mode = "deny"                      # | "tty" | "command"
# command = "wise-approver --tg @nas"   # only if mode = "command"
# timeout = "30s"
```

### Field reference

| Field            | Required | Notes                                                       |
|------------------|----------|-------------------------------------------------------------|
| `name`           | yes      | Must match the file basename.                               |
| `description`    | no       | Free-form, surfaced in `wise sandbox show`.                 |
| `profiles`       | no       | Profile id allow-list. Empty = unrestricted.                |
| `cards`          | no       | Card token allow-list. Empty = unrestricted.                |
| `balances`       | no       | Balance id allow-list.                                      |
| `allow`          | yes      | At least one entry required. Globs supported.               |
| `deny`           | no       | Hard overrides for `allow`. Globs supported.                |
| `conditions`     | no       | Per-command extra checks (rate limit, justify, audit).      |
| `escalation`     | no       | How `--sudo` behaves. Default `mode = "deny"`.              |

If `allow` is empty or unset, **the sandbox denies everything**. This is
intentional — there is no way to accidentally grant a permissive sandbox by
forgetting a field.

---

## Command paths

The CLI encodes the full clap subcommand chain as dot-separated paths:

| CLI invocation                              | Path                       |
|---------------------------------------------|----------------------------|
| `wise balance list`                         | `balance.list`             |
| `wise balance move ...`                     | `balance.move`             |
| `wise card permissions set ...`             | `card.permissions.set`     |
| `wise card-order create ...`                | `card-order.create`        |
| `wise simulate transfer-state ...`          | `simulate.transfer-state`  |
| `wise docs ask ...`                         | `docs.ask`                 |
| `wise sandbox show ...`                     | `sandbox.show`             |

Globs:

- `balance.*` — anything under `balance`
- `*.list` — any command named `list`
- `*` — everything

The deny list takes precedence over allow. Argument-aware denies are
expressed as `path:key=value`, e.g.:

```toml
deny = [
  "card.status:status=ACTIVE",   # may freeze, may not unfreeze
]
```

The DSL is intentionally tiny in v1 — only `key=value` matches. More complex
predicates (numeric comparisons, regex) are explicitly out of scope for v1.
If you need them, write a wrapper command and use the escalation mechanism.

---

## Enforcement points

The sandbox is enforced at three layers, in order:

### 1. Dispatch gate (`main.rs::dispatch`)

Before any command handler runs, the CLI computes the command path from the
parsed clap tree and checks it against the active sandbox. Denied commands
exit immediately with a structured error:

```json
{
  "error": {
    "code": "sandbox_denied",
    "command": "balance.move",
    "sandbox": "coding-agent",
    "hint": "this command is not allowed by the active sandbox; \
             re-run with --sudo to escalate (escalation mode: deny)"
  }
}
```

This is the fastest possible reject — the agent never even establishes a
network connection for a denied call.

### 2. Resource gate (`Ctx::require_profile()` and friends)

Inside `Ctx`, any helper that resolves a profile/card/balance id checks
whether the active sandbox restricts that resource type. Out-of-list ids
fail with:

```json
{
  "error": {
    "code": "sandbox_resource_denied",
    "resource": "profile",
    "id": 32171066,
    "sandbox": "coding-agent",
    "allowed": [73459809]
  }
}
```

This catches commands that try to operate on resources outside their scope
even though the command itself is allowed.

### 3. Per-command conditions (in handlers)

Command handlers ask `ctx.sandbox.condition("agent.fetch")` and apply:

- **Rate limiting**: `3/hour` → maintained as a tiny SQLite or jsonl ledger
  at `~/.config/wise/sandboxes/<name>.rate.jsonl`. Exceeding the limit
  returns `sandbox_rate_limited`.
- **Required justification**: `--justify "..."` becomes mandatory.
- **Audit**: every call (success or failure) appends a line to the audit
  log *before* the network round-trip.

Conditions cannot weaken the sandbox; they only add extra friction on top
of allow/deny rules.

---

## Escalation: `--sudo`

When a denied command is run with `--sudo`, the CLI consults the
`[escalation]` section:

| `mode`     | Behavior                                                                         |
|------------|----------------------------------------------------------------------------------|
| `deny`     | `--sudo` does nothing extra; denied stays denied. Default. Use for autonomous agents. |
| `tty`      | Prompts y/N on the controlling terminal. Use when a human is at the keyboard.    |
| `command`  | Runs `escalation.command` (the program), waits for exit code 0 to allow. Use for out-of-band approvals (Telegram bot, push notification, Slack). |

The `command` mode receives the requested action as JSON on stdin:

```json
{
  "sandbox": "coding-agent",
  "command": "transfer.create",
  "args": {"--quote": "abc-123", "--target-account": 999, "--reference": "..."},
  "now": "2026-04-07T19:50:00Z"
}
```

The approver script can prompt the user via any channel and exit 0 to allow,
non-zero to deny. A `timeout` field (default 30s) bounds how long the CLI
waits.

This is also the integration point for the future watchdog: the same
external program can be used to ask for approval *and* to push notifications
about webhook events.

---

## Sandbox management commands (human-side, never sandboxed)

```text
wise sandbox new <name>              interactive: pick profile/card, set caps
wise sandbox list                    show all configured sandboxes
wise sandbox show <name>             dump the active policy
wise sandbox edit <name>             open in $EDITOR
wise sandbox delete <name>           with --yes confirmation
wise sandbox check <name> <cmdpath>  dry-run: would this command be allowed?
wise sandbox shell <name>            spawn $SHELL with WISE_SANDBOX=<name>
wise sandbox audit <name>            tail/grep the audit log
```

These commands themselves cannot be called from inside an active sandbox —
the sandbox cannot edit its own policy or list other sandboxes. (`sandbox.*`
is implicitly in the deny set when a sandbox is active.)

---

## Worked examples

### Example A — pure read-only agent

```toml
name        = "researcher"
description = "Read-only access for a research agent"

profiles = [32171066]
allow = [
  "*.list",
  "*.get",
  "rate.*",
  "currency.list",
  "docs.ask",
]
deny = ["*.create", "*.update", "*.delete", "*.move", "*.fund", "*.cancel"]

[escalation]
mode = "deny"
```

Anything that reads data is allowed; anything that writes is hard-denied.
No escalation, fully autonomous, no surprises.

### Example B — coding agent with a virtual card

```toml
name        = "coding-agent"
description = "Pays for SaaS APIs while drafting code"

profiles = [73459809]
cards    = ["tok_abc..."]
balances = [88888888]

allow = [
  "balance.list",
  "balance.get",
  "card.get",
  "card.freeze",
  "rate.get",
  "docs.ask",
  "agent.fetch",
]
deny = ["balance.move", "transfer.*", "card.unfreeze", "card.permissions.set"]

[conditions."agent.fetch"]
rate_limit            = "3/hour"
require_justification = true
audit                 = "~/.config/wise/sandboxes/coding-agent.audit.jsonl"

[escalation]
mode = "tty"
```

The agent can see its card, freeze it, and fetch sensitive details up to 3x
per hour with a justification. Unfreezing requires a human at the keyboard
saying y to a `--sudo` prompt.

### Example C — hands-off payouts agent with external approver

```toml
name = "payouts-agent"

profiles = [32586336]
allow    = ["recipient.list", "recipient.get", "rate.get", "docs.ask"]
deny     = ["transfer.create"]   # always denied unless escalated

[escalation]
mode    = "command"
command = "/usr/local/bin/wise-approver --channel telegram --target @nas"
timeout = "60s"
```

The agent can list recipients and check rates freely, but every transfer
requires an out-of-band Telegram approval. Useful when the agent is running
on a server with no human at the keyboard.

---

## Things this design deliberately does NOT do

- **No "trust the agent's environment to set things correctly".** The
  sandbox is loaded from a file the *user* writes, not from an env var the
  agent could fabricate. Only the *activation* (which sandbox to use) comes
  from the env.
- **No nested sandboxes.** A sandbox is flat. If you need different
  permissions for sub-tasks, run them in separate child processes with
  different `WISE_SANDBOX` values.
- **No write access from inside a sandbox to sandbox configs.** The
  `sandbox.*` commands are themselves denied when a sandbox is active.
- **No silent fallback.** If `WISE_SANDBOX` points at a missing or invalid
  file, the CLI errors out and refuses to run *anything* (fail closed).
- **No "global" sandbox merging.** Exactly one sandbox is active at a time.
- **No regex or numeric predicates in v1.** Tiny DSL only. If you need real
  logic, use the escalation `command` mode and write a real program.

---

## Implementation phases

| Phase | Scope                                                                  |
|-------|------------------------------------------------------------------------|
| 2     | `src/sandbox/mod.rs`: config schema + loader + activation              |
| 2     | Dispatch gate in `main.rs::dispatch`                                   |
| 2     | Resource gate in `Ctx::require_profile`/`require_card`/`require_balance` |
| 3     | `wise sandbox new/list/show/check/edit/delete/shell` commands          |
| 4     | Audit log writer + rate-limit ledger + condition evaluation            |
| 7     | Escalation modes: `deny`, `tty`, `command`                             |

The agent-card flow in [`AGENT.md`](AGENT.md) is one consumer of this
primitive; the broader CLI should treat sandboxing as an optional safety layer,
not its primary identity.

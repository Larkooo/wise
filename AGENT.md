# Agent cards — design + safety plan

> An LLM agent gets its own Wise virtual card so it can pay for things on
> behalf of the user. This is the safety contract before any code touches
> a real PAN.

The user's failure modes here are nasty and irreversible: real money, real
fraud risk, real PCI exposure. The plan is layered defense — no single
layer is enough on its own, but breaking through any one of them still
keeps the blast radius small.

This document describes the **agent-card use case**. The enforcement
primitive that makes it safe is the [`SANDBOX.md`](SANDBOX.md) — a generic
policy layer that the agent's `wise` invocations are filtered through. This
file references the sandbox; it does not duplicate it.

Status: **design locked, Phase 1 (JWE module) in progress.**

---

## Threat model

The things that can hurt the user:

1. **Prompt injection / jailbreak.** A malicious page convinces the agent
   to spend money outside of policy.
2. **Card detail leak.** PAN/CVV ends up in a log file, transcript, training
   set, screenshot, or stolen laptop backup.
3. **Catastrophic mistake.** Agent spends $5000 instead of $5; orders 30
   cards instead of 1; deletes the wrong profile.
4. **Token compromise.** A leaked API token gives full account access — far
   beyond just card use.
5. **Regulatory.** PAN/CVV/PIN are PCI data. Storing them at rest puts the
   *user* in PCI scope they did not sign up for.

The Wise platform helps on a couple of these natively:

- Sensitive card details only flow through **JWE-encrypted** channels — we
  cannot accidentally log PAN in plaintext if we use JWE end-to-end.
- UK/EEA accounts get **SCA-protected** sensitive endpoints, so a leaked
  token alone is not enough to read PAN — there is a second factor.
- Cards have **spend limits** as native primitives — we don't have to
  invent rate limiting.

---

## Defense-in-depth (cheap → expensive)

### L1 — Profile isolation
The agent never touches the user's personal profile. We create or designate
a **dedicated business sub-profile** ("agent ops"). Worst case: the blast
radius is one profile.

Enforced by [`SANDBOX.md`](SANDBOX.md) `profiles = [...]` allow-list.

### L2 — Token isolation
Don't reuse the user's main personal token. Either:
- A *separate* personal token issued from wise.com → Settings → API tokens,
  used only by the agent (Wise allows multiple tokens), or
- (Partner tier) OAuth client-credentials with reduced scope.

Stored under a distinct keyring entry (`wise-agent:<env>`) so it can be
revoked independently of `wise-cli:<env>`.

### L3 — Card scoping
Always **virtual, never physical**. Virtual cards can be re-issued
instantly with one command — no theft risk, no shipping window, no
plastic to chase.

The card is `FROZEN` from the moment of issue. Spend limits are applied
*before* it is ever unfrozen:

- Per-transaction limit (default $20)
- Per-day limit (default $50)
- Per-month limit (default $500)
- Lifetime limit (default $1000)

Enforced by Wise's spend-limits API, plus the sandbox's `cards = [...]`
allow-list.

### L4 — Sensitive detail handling

Three options, in order of preference. **v1 ships option 1 only.**

1. **No-cache.** Fetch via JWE on-demand each time the agent needs to
   charge. Zero-resting-secret. EU users see one SCA challenge per fetch.
2. **Short-lived cache.** Fetch once, store in keyring under a key derived
   from the token, wipe after `--cache 5m` via a tokio task. Some risk
   window, lower friction.
3. **Memory-only daemon.** `wise-agentd` holds details in `mlock`'d memory;
   CLI talks over a unix socket with peer-cred check. Most secure, biggest
   engineering cost.

### L5 — Approval gates
Two modes, controlled by the sandbox `[escalation]` block:

- **Always-confirm** (default `mode = "tty"` for human-supervised agents):
  every card op (create, unfreeze, fetch, change-limit) prompts the user.
- **External approver** (`mode = "command"`): agents running headlessly
  defer to a Telegram bot, push notification, etc. Configured per-sandbox.
- **Pre-authorized window**: `wise card unfreeze --for 10m --max 50usd`
  gives the agent a short timed window — implemented as a sandbox
  condition that auto-refreezes the card on expiry.

### L6 — Audit log
Append-only JSONL at `~/.config/wise/sandboxes/<name>.audit.jsonl`,
written **before** every card-touching call. Mode `0600`. Each line
includes timestamp, command path, justification (`--justify "..."`),
result, and Wise request id.

The agent cannot disable this — it is enforced in the client layer (and
required by the sandbox `conditions."agent.fetch".audit` field), not in
the command handlers, so even a buggy/jailbroken command path goes
through it.

### L7 — Webhooks watchdog
A separate `wise watchdog` daemon subscribes to:
- `cards#transaction-state-change`
- `transfers#state-change`

If it sees a transaction outside policy (over limit, off-hours, blocked
merchant, unfamiliar amount) it freezes the card immediately and alerts
the user. This is the most important safety net because it catches
failures of every other layer.

**v2 only** — needs a public webhook receiver and a state machine, both
non-trivial.

### L8 — Panic button
- `wise card freeze --all` → freeze every card the active sandbox can see.
- A future `wise card kill` → permanent block (`status=BLOCKED`,
  irreversible per Wise). Lives outside of any sandbox.

These are regular CLI commands that the human runs from outside the
sandbox; they don't need to be in the agent's allow-list.

---

## How the agent actually uses the card

The agent never has its own command tree. It runs the regular `wise` CLI
with `WISE_SANDBOX=coding-agent` set in its environment. The sandbox at
`~/.config/wise/sandboxes/coding-agent.toml` looks roughly like:

```toml
name        = "coding-agent"
description = "Pays for SaaS APIs while drafting code"

profiles = [73459809]
cards    = ["tok_..."]
balances = [88888888]

allow = [
  "balance.list",
  "balance.get",
  "card.get",
  "card.freeze",
  "rate.get",
  "currency.list",
  "docs.ask",
  "agent.fetch",
]
deny = [
  "balance.move", "transfer.*",
  "card.unfreeze", "card.permissions.set",
]

[conditions."agent.fetch"]
rate_limit            = "3/hour"
require_justification = true
audit                 = "~/.config/wise/sandboxes/coding-agent.audit.jsonl"

[escalation]
mode = "tty"
```

Inside that sandbox, the agent's only card-touching options are:

| Command                                          | Effect                          |
|--------------------------------------------------|---------------------------------|
| `wise card get <token>`                          | metadata, no PAN                |
| `wise card freeze <token>`                       | force-freeze                    |
| `wise agent fetch <token> --justify "..."`       | one-shot JWE PAN/CVV/expiry     |
| `wise balance list`                              | check available funds           |

Anything else — moving money, creating transfers, unfreezing the card,
changing permissions, touching another profile — is denied at the
dispatch layer. The agent gets a structured `sandbox_denied` error with
a hint to escalate via `--sudo` (which in `mode = "tty"` prompts the
human at the keyboard).

---

## Technical blockers + how we unblock them

### JWE is mandatory
Wise's `/twcard-data/v1/sensitive-card-data/details` only accepts
JWE-encrypted requests and only returns JWE-encrypted responses. From the
docs and Wise's JOSE implementation: **RSA-OAEP-256** for key wrapping +
**A256GCM** for content encryption.

Implementation steps:

1. `GET /twcard-data/v1/clientSideEncryption/fetchEncryptingKey` — fetch
   Wise's RSA public key (PEM).
2. Generate ephemeral 32-byte AES key + 12-byte GCM nonce.
3. RSA-OAEP-256 wrap the AES key with Wise's public key.
4. AES-256-GCM encrypt the request payload with the AAD set to the
   base64url'd protected header (per RFC 7516 §5.1).
5. Compact-serialize as JWE: `header.encryptedKey.iv.ciphertext.tag`.
6. POST it; receive a JWE response and reverse the process using either
   the same CEK (direct encryption) or our registered private key.

In Rust this is `rsa` + `aes-gcm` + `sha2` + `rand` + `base64`. ~300 LOC
of careful code, no `openssl` dep. **In progress as Phase 1.**

### SCA is mandatory in EU/UK
The sensitive details endpoint is SCA-protected for EEA accounts. The
user on this machine is US-based so SCA does not apply *to them*, but
the broader CLI cannot ship to EU users without an SCA factor flow
(PIN / device-fp / facemap, all of which are themselves JWE-encrypted).
Documented as a v2 gap.

---

## Phased plan

| Phase | Scope                                                                                                       | Status  |
|-------|-------------------------------------------------------------------------------------------------------------|---------|
| 0     | Design lock-in — `AGENT.md` + `SANDBOX.md` on disk                                                          | **done** |
| 1     | JWE module (`src/client/jose.rs`) + RFC vectors + `wise jose fetch-key`/`encrypt`/`decrypt` debug commands  | **in progress** |
| 2     | Sandbox primitive (`src/sandbox/mod.rs`): config loading, dispatch gate, resource gate                     | pending |
| 3     | `wise sandbox new/list/show/check/edit/delete/shell` commands                                               | pending |
| 4     | Audit log + rate-limit ledger + per-command conditions                                                      | pending |
| 5     | `wise agent init` (creates the agent profile + card + sandbox config in one wizard)                         | pending |
| 6     | `wise agent fetch` (uses JWE module from phase 1, gated by sandbox)                                         | pending |
| 7     | Escalation modes: `tty` and `command`                                                                       | pending |
| 8     | Watchdog daemon (v2)                                                                                        | pending |

---

## Things this plan deliberately does NOT do

- **No card-storage encryption layer beyond keychain.** macOS Keychain,
  Linux Secret Service, and Windows Credential Manager are already
  encrypted with the user's login credentials. Adding our own AES wrapper
  on top buys nothing and adds key-management surface.
- **No "agent autonomy mode" that bypasses sandbox confirmation.** If
  you want always-on autonomous spending, you do it explicitly with
  `unfreeze --for` windows or by configuring `[escalation] mode = "deny"`
  and pre-authorizing nothing.
- **No silent retries on declined transactions.** Decline → log → exit.
  The agent must surface the decline to the user, not loop.
- **No card-detail caching beyond a single call** in v1. This is the
  single biggest knob and we want to ship the boring/safe version first.

---

## Minimal viable agent flow (what the user does)

```bash
# One-time setup (interactive — confirms each destructive step)
wise agent init
# → creates "Agent Ops" business sub-profile
# → issues virtual USD card, status=FROZEN
# → applies spend limits: $20/tx, $50/day, $500/month, $1000 lifetime
# → writes ~/.config/wise/sandboxes/coding-agent.toml
# → stores agent token in keyring as wise-agent:production
# → prints next-steps

# Each session
WISE_SANDBOX=coding-agent wise card get tok_...    # what's the card doing?
WISE_SANDBOX=coding-agent wise card freeze tok_... # belt-and-braces

# The agent just needs WISE_SANDBOX in its env
# Its prompt template runs commands like:
#   wise balance list
#   wise card get tok_...
#   wise agent fetch tok_... --justify "Stripe checkout for vercel pro plan"
# Anything outside the allow-list errors before it touches the network.

# If anything looks wrong
wise card freeze tok_...     # from the human side, no sandbox active
wise agent rotate            # issue new card, block the old one
```

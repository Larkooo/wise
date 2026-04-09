# wise — CLI for the Wise Platform API

A Rust CLI that wraps the Wise Platform API so scripts, operators, and agents
can authenticate once, then send money, manage profiles, issue cards,
subscribe to webhooks, and ask the docs anything without writing HTTP by hand.
The core product is the general-purpose CLI; sandbox and agent flows are
advanced layers on top, not the repo's primary identity.

## Goals

- **Machine-friendly**: JSON output by default, stable exit codes,
  deterministic flag-based UX, no interactive prompts unless requested.
- **Safe for automation**: the Wise API sandbox environment is the default;
  optional CLI sandboxes add policy controls for automation, and production
  money-moving operations require `--yes` (or `WISE_YES=1`) to proceed.
- **Comprehensive but pragmatic**: cover the high-value 80% of the API surface
  cleanly; stub the long tail of compliance/security/internal endpoints with
  clear "not implemented" errors that point at the docs.
- **Self-documenting**: `wise docs ask "..."` wraps the public docs Q&A
  endpoint so users and agents can look things up live.

## Non-goals (v1)

- JOSE/JWE-encrypted endpoints (sensitive card details, SCA verify-pin/facemap).
  These require client-side key management; we surface them as `not implemented`
  with a doc link.
- FaceTec biometric flows.
- Push provisioning for Apple/Google Pay.
- Hosted-KYC web UI integration (we expose the API surface; the user or agent
  handles the redirect themselves).
- 3DS challenge result UI.

## Architecture

```
src/
  main.rs                 entrypoint, top-level Cli, dispatch
  cli/
    mod.rs                shared types (GlobalArgs, OutputFormat)
    auth.rs               login/logout/status/whoami
    profile.rs            list/get/current
    balance.rs            list/get/create/delete/move/topup
    quote.rs              create/get/update
    recipient.rs          list/create/get/delete + requirements
    transfer.rs           create/list/get/cancel/fund/requirements/receipt
    card.rs               list/get/status/permissions
    card_order.rs         create/list/get/requirements/programs
    webhook.rs            profile + application subscriptions
    rate.rs               current + historical
    activity.rs           list activities
    currency.rs           list currencies
    docs.rs               ask-ai SSE
    simulate.rs           sandbox simulations
  client/
    mod.rs                WiseClient: env, http, auth headers, error mapping
    error.rs              ApiError + WiseError
    sse.rs                Server-Sent Events parser for /_ask-ai
    jose.rs               JWE helpers for sensitive-card experiments
  agent/                  optional manual-paste agent-card flow
  config.rs               TOML config + keyring credential store
  output.rs               JSON / pretty rendering
  sandbox/                optional policy gates for automation
```

### Tech choices

- `clap` v4 derive for the CLI
- `tokio` + `reqwest` (rustls) for async HTTP
- `serde` + `serde_json` for the wire
- `keyring` for credential storage with a plaintext-file fallback
- `eventsource-stream` for the docs ask-ai SSE
- `anyhow` for top-level errors, `thiserror` for typed API errors
- `tracing` + `tracing-subscriber` for `--verbose` logging
- `uuid` for `X-idempotence-uuid` headers
- `directories` for XDG config paths
- `comfy-table` for `--pretty` rendering

### Auth model

The CLI supports four credential modes, picked in this order:
1. `--token <t>` flag (overrides everything, useful for one-off agent calls)
2. `WISE_API_TOKEN` env var
3. OS keychain entry (`wise:<env>`) populated by `wise auth login`
4. Plaintext fallback at `~/.config/wise/credentials.toml` (perms 0600)

A "token" can be one of:
- **Personal token** (Bearer) — simplest, individual users
- **OAuth user access token** (Bearer) — partner integrations
- **OAuth client credentials token** (Bearer) — application-level operations
  (`wise auth login --client-id ... --client-secret ...` exchanges these via
  `POST /oauth/token`)

### Environments

- **Sandbox**: `https://api.wise-sandbox.com`, `https://docs.wise.com`
- **Production**: `https://api.wise.com`, `https://docs.wise.com`

Select the target environment with `--env ...`, `WISE_ENV=...`, or
`wise config set env sandbox|production`. If no environment is selected,
API-affecting commands fail with a clear error instead of silently choosing one.

### Output

Default: machine-readable single-line JSON to stdout. Errors go to stderr as
JSON with shape `{"error": {"code": "...", "message": "...", "details": ...}}`.
Use `--pretty` for indented JSON, `--table` for human tables (where it makes
sense). `--verbose` enables tracing logs to stderr.

## Command tree

```
wise auth
  login [--token T] [--client-id ID --client-secret SECRET] [--env ENV]
  status
  whoami
  logout

wise config
  get <key>
  set <key> <value>
  list
  path

wise profile
  list
  get <profile-id>
  current

wise balance
  list [--profile P] [--type STANDARD|SAVINGS]
  get <balance-id> [--profile P]
  create --currency C [--type STANDARD|SAVINGS] [--name N]
  delete <balance-id>
  move --from F --to T [--amount N --currency C | --quote Q]
  topup <balance-id> --amount N        # sandbox-only
  total --currency C [--profile P]

wise quote
  create --source C --target C [--source-amount N | --target-amount N]
         [--profile P] [--pay-in BANK_TRANSFER|BALANCE] [--pay-out ...]
         [--target-account A]
  get <quote-id> [--profile P]
  update <quote-id> --target-account A [--profile P]
  example --source C --target C ...    # unauthenticated /v3/quotes

wise recipient
  list [--profile P] [--currency C]
  create --currency C --account-holder-name N --type T --details JSON
         [--profile P]
  get <recipient-id>
  delete <recipient-id>
  requirements --quote Q [--profile P]

wise transfer
  create --quote Q --target-account A [--reference R] [--purpose P]
         [--source-of-funds S] [--customer-tx-id ID]
  list [--profile P] [--status S] [--limit N]
  get <transfer-id>
  cancel <transfer-id>
  fund <transfer-id> [--type BALANCE|TRUSTED_PRE_FUND_BULK]
  requirements --quote Q --target-account A
  receipt <transfer-id> [--output PATH]
  payments <transfer-id>

wise card
  list [--profile P]
  get <card-token> [--profile P]
  status <card-token> --status ACTIVE|FROZEN|BLOCKED
  permissions get <card-token>
  permissions set <card-token> --permission P --enabled true|false
  reset-pin-count <card-token>

wise card-order
  programs [--profile P]
  create --program P --type VIRTUAL|PHYSICAL [--cardholder-profile-id ID]
         [--address JSON]
  list [--profile P]
  get <card-order-id>
  requirements <card-order-id>
  cancel <card-order-id>

wise webhook
  list [--profile P | --application]
  get <subscription-id> [--profile P | --application]
  create --name N --url U --trigger T [--version V]
         [--profile P | --application] [--mtls]
  delete <subscription-id> [--profile P | --application]
  test <subscription-id> --application

wise rate
  get --source C --target C [--time TS]
  history --source C --target C --from TS --to TS --group day|hour|minute

wise activity
  list [--profile P] [--monetary-resource T] [--status S] [--since DATE]

wise currency list

wise docs
  ask "<question>" [--no-stream] [--history JSON]

wise simulate
  transfer-state <transfer-id> <state>
  balance-topup <profile-id> --balance B --amount N
  verify-profile <profile-id>
  card-auth ...
  card-clearing ...
  swift-in <profile-id> --amount N --currency C
  bank-tx <profile-id> --amount N --currency C
```

## Scope tiers

### Tier 1 — implement now (must cover the core Wise workflows)
- [x] auth: login (personal token), logout, status, whoami
- [x] config
- [x] profile: list, get, current
- [x] balance: list, get, create, delete, move
- [x] quote: create (auth), get, update, example
- [x] recipient: list, create, get, delete, requirements
- [x] transfer: create, list, get, cancel, fund, requirements, receipt, payments
- [x] card: list, get, status, permissions get/set, reset-pin-count
- [x] card-order: programs, create, list, get, requirements
- [x] rate: get, history
- [x] currency: list
- [x] docs: ask (SSE streaming)
- [x] activity: list

### Tier 2 — implement now if straightforward
- [x] webhook: profile + application CRUD + test
- [x] simulate: transfer-state, balance-topup, verify-profile, swift-in, bank-tx
- [x] balance: topup (alias for simulate balance-topup), total
- [x] auth login --client-id/--client-secret (OAuth client_credentials)

### Tier 3 — stub with helpful "not implemented" errors
- [ ] disputes (Dynamic Flow JS framework — out of scope for CLI)
- [ ] sensitive card details (JOSE/JWE — out of scope v1)
- [ ] SCA verify (PIN/face/device — JOSE — out of scope v1)
- [ ] FaceTec / push provisioning
- [ ] KYC review hosted flow (we expose status, not the redirect)
- [ ] 3DS challenge result
- [ ] batch groups (cover later — useful but big)
- [ ] bulk settlement
- [ ] partner cases
- [ ] address create/list (deferred — usually done via profile flow)

## Progress checklist

- [x] Plan + PROGRESS.md
- [x] Cargo scaffold + dependencies
- [x] WiseClient (env, headers, error mapping, idempotency)
- [x] Config + credential store (keyring + file fallback)
- [x] Output formatting (JSON / pretty / table)
- [x] auth commands
- [x] profile commands
- [x] balance commands
- [x] quote commands
- [x] recipient commands
- [x] transfer commands
- [x] card commands
- [x] card-order commands
- [x] webhook commands
- [x] rate commands
- [x] activity commands
- [x] currency command
- [x] docs ask (SSE)
- [x] simulate commands
- [x] cargo build --release passes
- [x] smoke test: `wise docs ask` round-trip
- [ ] smoke test: `wise auth status` (requires real token — agent task)

## Notes for the agent using this CLI

1. Start with `wise auth login --token $WISE_SANDBOX_TOKEN` (sandbox by default).
2. `wise profile current` prints your default profile id; most commands accept
   `--profile <id>` to override.
3. To send money end-to-end:
   ```
   wise quote create --source GBP --target EUR --source-amount 100
   wise recipient create ...                  # or reuse one from `wise recipient list`
   wise quote update <quote-id> --target-account <recipient-id>
   wise transfer create --quote <quote-id> --target-account <recipient-id> \
       --reference "payout"
   wise transfer fund <transfer-id> --type BALANCE
   ```
4. When you don't know the API shape, ask the docs:
   `wise docs ask "what fields do I need to create a USD recipient?"`
5. All money-moving commands in production require `--yes` (or `WISE_YES=1`).

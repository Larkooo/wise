# wise

`wise` is a Rust CLI for the Wise Platform API.

It wraps the core Wise API surface in a machine-friendly command-line
interface so scripts, operators, and automation can work with profiles,
balances, quotes, recipients, transfers, cards, webhooks, rates, and docs
without hand-writing HTTP requests.

## Status

The CLI already covers the main operational workflows:

- authentication and config
- profiles and balances
- quotes, recipients, and transfers
- cards and card orders
- webhooks, rates, activity, and currencies
- Wise docs lookup from the terminal

The repo also contains optional automation features:

- `--sandbox <name>` for policy-gating `wise` commands
- `wise sandbox ...` for managing sandbox policies
- `wise agent ...` for the opinionated agent-card flow

Those layers are optional. A normal CLI user does not need to enable them.

## Build

```bash
cargo build
```

For an optimized binary:

```bash
cargo build --release
```

## Quick Start

Pick an API environment once:

```bash
wise config set env production
```

Or use the Wise test environment:

```bash
wise config set env sandbox
```

Authenticate with a Wise API token for that environment:

```bash
wise auth login --token <token>
```

You can also override the configured environment per command:

```bash
wise --env production profile list
```

Common examples:

```bash
wise profile list
wise balance list --profile <profile-id>
wise quote create --profile <profile-id> --source USD --target EUR --source-amount 100
wise recipient list --profile <profile-id>
wise transfer list --profile <profile-id>
wise rate get --source USD --target EUR
wise docs ask "How do I fund a transfer from balance?"
```

Production money-moving operations require explicit confirmation:

```bash
wise --env production --yes transfer create ...
```

## Output

The default output is single-line JSON for scripting.

Useful output flags:

- `--pretty` for indented JSON
- `--table` for human-readable tables where supported
- `--verbose` for debug logging to stderr

## Sandboxing

There are two different sandbox concepts in this repo:

- the Wise API sandbox environment, selected explicitly with `--env sandbox`
- the CLI policy sandbox, activated with `--sandbox <name>` or `WISE_SANDBOX`

The CLI policy sandbox is off unless you opt into it. It is intended for
automation and agent-style deployments where you want `wise` itself to enforce
an allow-list, resource scoping, audit logging, and rate limits.

## Development

Run the test suite with:

```bash
cargo test
```

The current implementation and scope notes live in:

- [`PROGRESS.md`](PROGRESS.md)
- [`SANDBOX.md`](SANDBOX.md)
- [`AGENT.md`](AGENT.md)

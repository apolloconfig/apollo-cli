# Apollo CLI

`apollo-cli` is the standalone Rust repository for the `apollo` command-line interface.

This repository currently covers the first Apollo CLI v0 slices:

- top-level v0 command routing
- global flag parsing
- structured placeholder errors
- output formatting foundation
- non-secret profile configuration
- runtime context resolution
- credential-store abstraction and auth commands
- conservative redaction foundation
- local developer workflow and CI entrypoints

It does not yet implement Apollo OpenAPI calls or agent-oriented flows.

## Command groups

The current scaffold exposes the planned v0 top-level groups:

- `auth`
- `profile`
- `app`
- `env`
- `namespace`
- `config`
- `release`
- `api`

## Global flags

The current scaffold parses these global flags before subcommands:

- `--profile`
- `--server`
- `--output json|table`
- `--yes`

## Profile config

The CLI stores non-secret profile metadata in `config.toml` under the OS config directory:

- macOS: `~/Library/Application Support/apollo/config.toml`
- Linux: `$XDG_CONFIG_HOME/apollo/config.toml` or `~/.config/apollo/config.toml`
- Windows: `%APPDATA%\apollo\config.toml`

The config file stores:

- `active_profile`
- profile name
- profile `server`
- profile `output`
- optional `operator`
- optional credential lookup metadata, such as backend/key names

Tokens are intentionally not part of the supported config schema.

Example:

```toml
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
operator = "apollo-bot"

[profiles.dev.credential]
backend = "native"
key = "dev"
```

## Profile commands

- `apollo profile list`
- `apollo profile show`
- `apollo profile use <name>`

Runtime context resolution follows this order:

1. explicit flags such as `--profile`, `--server`, `--output`
2. environment variables such as `APOLLO_PROFILE`, `APOLLO_SERVER`, `APOLLO_OUTPUT`
3. active profile config
4. command defaults

## Auth commands

- `apollo auth login --token-stdin`
- `apollo auth login --token-stdin --store-token-in-file`
- `apollo auth status`
- `apollo auth logout`

Credential storage uses an internal store abstraction with these logical providers:

- `native`: default credential backend, implemented through the OS credential store via the Rust `keyring` crate
- `env`: read-only CI/headless provider through `APOLLO_TOKEN`
- `file`: explicit fallback enabled only with `--store-token-in-file`
- in-memory provider for unit tests

Native backend selection follows the OS behavior of the underlying credential store:

- macOS: Keychain Services
- Windows: Credential Manager
- Linux desktop: freedesktop Secret Service compatible providers
- Linux headless/CI: use `APOLLO_TOKEN` or explicitly opt into file fallback

File fallback writes token material outside `config.toml` under the CLI credentials directory and uses restrictive file permissions on Unix. The profile config stores only non-secret credential lookup metadata.

## Redaction and Errors

The output layer applies conservative redaction to human and JSON output before rendering. Token-like fields, `Authorization: Bearer ...` headers, and `consumer token ...` text are rendered as `[REDACTED]`.

Structured JSON errors include:

- `code`: stable error code
- `category`: stable category
- `message`: human-readable message
- optional non-sensitive details such as `command`, `profile`, `path`, or `follow_up_issue`

Current error categories:

- `authentication_failed`
- `permission_denied`
- `invalid_input`
- `not_found`
- `conflict`
- `precondition_failed`
- `network`
- `server`
- `confirmation_required`
- `unsupported_operation`

## Local development

Build the CLI:

```bash
cargo build
```

Run the help output:

```bash
cargo run -- --help
```

Run tests:

```bash
cargo test
```

Format the repository:

```bash
cargo fmt
```

Lint the repository:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Repository layout

- `src/cli.rs`: CLI definition and flag parsing
- `src/config.rs`: profile config loading, saving, and context resolution
- `src/command.rs`: top-level command routing
- `src/credential.rs`: credential-store abstraction and providers
- `src/error.rs`: structured CLI error model
- `src/output.rs`: output rendering abstractions
- `src/redaction.rs`: conservative redaction utilities
- `tests/auth.rs`: integration coverage for auth commands and credential behavior
- `tests/cli.rs`: integration coverage for help, flags, and placeholder errors
- `tests/profile.rs`: integration coverage for profile commands and context resolution
- `tests/redaction.rs`: integration coverage for redaction behavior

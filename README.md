# Apollo CLI

`apollo-cli` is the standalone Rust repository for the `apollo` command-line interface.

This repository currently covers the first Apollo CLI v0 slices:

- top-level v0 command routing
- global flag parsing
- output formatting foundation
- non-secret profile configuration
- runtime context resolution
- credential-store abstraction and auth commands
- conservative redaction foundation
- representative Apollo Portal OpenAPI calls under `/openapi/v1/*`
- raw OpenAPI passthrough through `apollo api`
- confirmation protection for mutating commands
- local developer workflow and CI entrypoints

The package is intentionally self-contained and movable. It lives in this standalone repository for
review, but avoids repo-specific assumptions so the crate can be split, vendored, or republished
without Apollo server-side changes.

It does not implement generated OpenAPI SDK bindings, agent sessions, MCP, or server-side schema
changes in this slice.

## Command groups

- `auth`
- `profile`
- `app`
- `env`
- `namespace`
- `config`
- `release`
- `api`

Representative v0 commands:

```bash
apollo init
apollo profile add dev
apollo profile add prod --use
apollo app list
apollo app get sample-app
apollo env list --app sample-app
apollo namespace list --env DEV --app sample-app
apollo namespace get --env DEV --app sample-app application
apollo namespace create --env DEV --app sample-app application --comment "app settings" --yes
apollo config list --env DEV --app sample-app
apollo config get --env DEV --app sample-app timeout
apollo config set --env DEV --app sample-app timeout 3000 --type 1 --yes
apollo config delete --env DEV --app sample-app timeout --yes
apollo config diff --env DEV --app sample-app --target-env FAT
apollo config apply --env DEV --app sample-app --target-env FAT --yes
apollo release list --env DEV --app sample-app
apollo release create --env DEV --app sample-app --title "release title" --yes
apollo release rollback --env DEV 123 --yes
apollo api get /openapi/v1/apps
apollo api post /openapi/v1/apps --body '{"app":{"appId":"sample-app"}}' --yes
```

These commands use existing Apollo Portal OpenAPI endpoints only. Deprecated Portal WebAPI
endpoints are intentionally not used.
`apollo namespace create` registers the AppNamespace first and then creates the namespace in the
requested environment and cluster. It creates a private AppNamespace by default. Pass `--public` only
when the namespace should be public. File namespace formats are inferred from `.json`, `.yml`,
`.yaml`, and `.xml` suffixes; other names default to `properties`. `--comment` is stored on the
AppNamespace. Public AppNamespace registration sends Apollo's `appendNamespacePrefix=true` default;
pass `--no-append-namespace-prefix` when the namespace name must be stored without Apollo's public
namespace prefix behavior.

`apollo config set` sends Apollo's `OpenItemDTO.type` field. The default is `0` for a string item.
Apollo Portal also uses `1` for number, `2` for boolean, and `3` for JSON; these values are validated
client-side before the OpenAPI request is sent.

## Global flags

The current scaffold parses these global flags before subcommands:

- `--profile`
- `--server`
- `--output json|table`
- `--yes`

## Guided setup

Use `apollo init` for first-time setup. It creates a profile, writes non-secret profile metadata to
`config.toml`, and can store an Apollo OpenAPI token through the credential-store abstraction.
For Apollo versions with Portal user access tokens, user-token auth is the recommended mode for
interactive users, AI agents, and personal automation.

For local Apollo assembly testing:

```bash
apollo --output json init --store-token-in-file
apollo profile show
apollo env list --app sample-app
```

By default, `apollo init` creates a `local` profile with:

- `server = "http://127.0.0.1:8070"`
- no persisted `output` unless `--output` is passed
- `auth_mode = "user-token"`
- no `operator`; user-token OpenAPI requests use the token owner as the operator
- `active_profile = "local"`

Use `apollo profile add` to add more environments without hand-editing config:

```bash
printf '%s\n' "$DEV_TOKEN" | apollo \
  --server https://apollo-dev.example.com \
  --output json \
  profile add dev \
  --token-stdin

apollo profile add prod --server https://apollo-prod.example.com --use
apollo profile add legacy --server https://apollo-legacy.example.com --auth-mode consumer-token --operator alice
```

`profile add` does not switch the active profile by default. Pass `--use` when the newly added
profile should become active. Existing profiles are protected from accidental replacement; pass
`--overwrite` to replace one intentionally.

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
- profile `auth_mode`, either `user-token` or `consumer-token`
- optional `operator`
- optional credential lookup metadata, such as backend/key names

Tokens are intentionally not part of the supported config schema. Prefer `apollo init`,
`apollo profile add`, and `apollo auth login` over editing this file by hand.

If `auth_mode` is missing, the CLI treats the profile as `consumer-token` for backward
compatibility with existing configs.

Example:

```toml
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
auth_mode = "user-token"

[profiles.dev.credential]
backend = "native"
key = "dev"
```

## Profile commands

- `apollo init`
- `apollo profile add [name]`
- `apollo profile list`
- `apollo profile show`
- `apollo profile use <name>`

Runtime context resolution follows this order:

1. explicit flags such as `--profile`, `--server`, `--output`
2. environment variables such as `APOLLO_PROFILE`, `APOLLO_SERVER`, `APOLLO_OUTPUT`
3. active profile config
4. command defaults

## Auth commands

- `apollo auth login`
- `apollo auth login --token-stdin`
- `apollo auth login --token-stdin --store-token-in-file`
- `apollo auth status`
- `apollo auth whoami`
- `apollo auth capabilities`
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

OpenAPI commands support two token modes:

- `user-token`: recommended for interactive users, AI agents, and local automation. Tokens start
  with `apollo_pat_` and are sent as `Authorization: Bearer <token>`.
- `consumer-token`: compatibility mode for existing integrations and older Apollo deployments.
  Tokens are sent as the raw `Authorization: <token>` header value.

`apollo init` and `apollo profile add` default to `user-token`. Use `--auth-mode consumer-token`
when configuring legacy consumer-token credentials. Mutating commands require a configured
`operator` only in `consumer-token` mode; user-token requests use the owning Portal user.

`apollo config get`, `apollo config list`, `apollo config diff`, and `apollo config apply` read
configuration item values before rendering, diffing, or syncing. Use `user-token` mode for these
commands. Legacy consumer-token mode cannot safely verify namespace-level read scope from the
available `/openapi/v1/apps/authorized` response, so the CLI rejects these item-read workflows
instead of relying on app-level visibility alone.

For local or CI use, `APOLLO_TOKEN` takes precedence and is never written to disk:

```bash
APOLLO_TOKEN="$TOKEN" apollo --server http://localhost:8070 app list --output json
```

When `APOLLO_TOKEN` starts with `apollo_pat_`, the CLI automatically treats it as `user-token`;
otherwise it uses `consumer-token` compatibility mode.

`apollo auth logout` removes the credential referenced by the selected profile. It cannot remove
`APOLLO_TOKEN` from the parent shell environment. When `APOLLO_TOKEN` is still set, logout reports
that environment credentials will continue to apply; run `unset APOLLO_TOKEN` to disable that
temporary credential.

For interactive use, configure a profile and store the token with hidden input:

```bash
apollo --profile dev auth login
apollo --profile dev app list
```

`auth login` stores the token in the OS credential store by default. If the native store is not
available in an interactive terminal, the CLI asks whether to use the local file fallback instead.

For scripts or manual paste-with-enter workflows, `--token-stdin` reads one token line:

```bash
printf '%s\n' "$TOKEN" | apollo --profile dev auth login --token-stdin
apollo --profile dev auth login --token-stdin
printf '%s\n' "$LEGACY_CONSUMER_TOKEN" | apollo --profile legacy auth login --auth-mode consumer-token --token-stdin
```

Use user-token self-check commands after login:

```bash
apollo --profile dev auth whoami
apollo --profile dev auth capabilities
```

These commands call `/openapi/v1/user-tokens/current` and
`/openapi/v1/user-tokens/current/capabilities`. They require `user-token` auth mode and do not
create, rotate, or revoke tokens; user token creation remains a Portal self-service flow.

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

## OpenAPI behavior

The first v0 implementation uses a small generic HTTP client instead of a generated SDK. This keeps
the CLI independent from the Apollo server repository while still constraining all built-in resource
commands to `/openapi/v1/*`.

Path and payload mapping follows the current Apollo Portal OpenAPI contract, including:

- `GET /openapi/v1/apps`
- `GET /openapi/v1/envs`
- `GET /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces`
- `GET|PUT|DELETE /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/items/{key}`
- `POST /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/items/diff`
- `POST /openapi/v1/apps/{appId}/appnamespaces` for AppNamespace registration
- `POST /openapi/v1/namespaces`
- `POST /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/items/synchronize`
- `GET /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/releases/active`
- `POST /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/releases`
- `PUT /openapi/v1/envs/{env}/releases/{releaseId}/rollback`

Mutating commands require `--yes`. Without it, the CLI returns `confirmation_required` before
opening a network connection.

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

Run focused OpenAPI command integration tests with the local mock HTTP server:

```bash
cargo test --test openapi
```

If you have a local Apollo Portal running, you can also smoke-test against it:

```bash
APOLLO_TOKEN="$TOKEN" cargo run -- --server http://localhost:8070 --output json env list --app sample-app
APOLLO_TOKEN="$TOKEN" cargo run -- --server http://localhost:8070 --output json app list
APOLLO_TOKEN="$USER_TOKEN" cargo run -- --server http://localhost:8070 --output json auth whoami
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
- `src/http.rs`: generic OpenAPI HTTP client and path helpers
- `src/output.rs`: output rendering abstractions
- `src/redaction.rs`: conservative redaction utilities
- `tests/auth.rs`: integration coverage for auth commands and credential behavior
- `tests/cli.rs`: integration coverage for help, flags, and structured errors
- `tests/openapi.rs`: integration coverage for OpenAPI paths, auth headers, and confirmation guards
- `tests/profile.rs`: integration coverage for profile commands and context resolution
- `tests/redaction.rs`: integration coverage for redaction behavior

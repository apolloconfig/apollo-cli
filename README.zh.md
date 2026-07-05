# Apollo CLI

[English](README.md)

`apollo-cli` 是 `apollo` 命令行工具的独立 Rust 仓库。

当前仓库覆盖 Apollo CLI v0 的第一批能力：

- 顶层 v0 命令路由
- 全局参数解析
- 输出格式化基础
- 非敏感 profile 配置
- 运行时上下文解析
- 凭据存储抽象和 auth 命令
- 保守的脱敏基础能力
- `/openapi/v1/*` 下的代表性 Apollo Portal OpenAPI 调用
- 通过 `apollo api` 透传原始 OpenAPI 请求
- 变更类命令的确认保护
- 本地开发流程和 CI 入口

这个包刻意保持自包含、可移动。它当前位于这个独立仓库中便于评审，但不会假设仓库特有环境，因此后续可以在不修改 Apollo 服务端的情况下拆分、vendored 或重新发布 crate。

本阶段不实现生成式 OpenAPI SDK 绑定、agent session、MCP 或服务端 schema 变更。

## 命令分组

- `auth`
- `profile`
- `app`
- `env`
- `namespace`
- `config`
- `release`
- `api`

代表性的 v0 命令：

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

这些命令只使用已有的 Apollo Portal OpenAPI 端点。CLI 有意不使用已废弃的 Portal WebAPI 端点。

`apollo namespace create` 会先注册 AppNamespace，再在指定环境和 cluster 中创建 namespace。默认创建私有 AppNamespace。只有在 namespace 应该是公共 namespace 时才传 `--public`。文件类型 namespace 会根据 `.json`、`.yml`、`.yaml` 和 `.xml` 后缀推断格式；其他名称默认使用 `properties`。`--comment` 会存储在 AppNamespace 上。公共 AppNamespace 注册会发送 Apollo 的 `appendNamespacePrefix=true` 默认值；如果 namespace 名称必须按原样存储、不使用 Apollo 的公共 namespace 前缀行为，请传 `--no-append-namespace-prefix`。

`apollo config set` 会发送 Apollo 的 `OpenItemDTO.type` 字段。默认值为 `0`，表示字符串配置项。Apollo Portal 还使用 `1` 表示数字、`2` 表示布尔值、`3` 表示 JSON；这些值会在发送 OpenAPI 请求前由客户端校验。

## 全局参数

当前脚手架会在子命令之前解析以下全局参数：

- `--profile`
- `--server`
- `--output json|table`
- `--yes`

## 引导式初始化

首次使用时推荐执行 `apollo init`。它会创建 profile，将非敏感 profile 元数据写入 `config.toml`，并且可以通过凭据存储抽象保存 Apollo OpenAPI token。

对于支持 Portal 用户访问 token 的 Apollo 版本，交互式用户、AI agent 和个人自动化推荐使用 user-token 鉴权模式。

本地 Apollo assembly 测试示例：

```bash
apollo --output json init --store-token-in-file
apollo profile show
apollo env list --app sample-app
```

默认情况下，`apollo init` 会创建一个 `local` profile：

- `server = "http://127.0.0.1:8070"`
- 除非显式传入 `--output`，否则不持久化 `output`
- `auth_mode = "user-token"`
- 不配置 `operator`；user-token OpenAPI 请求使用 token 所属用户作为操作人
- `active_profile = "local"`

可以用 `apollo profile add` 增加更多环境，避免手工编辑配置：

```bash
printf '%s\n' "$DEV_TOKEN" | apollo \
  --server https://apollo-dev.example.com \
  --output json \
  profile add dev \
  --token-stdin

apollo profile add prod --server https://apollo-prod.example.com --use
apollo profile add legacy --server https://apollo-legacy.example.com --auth-mode consumer-token --operator alice
```

`profile add` 默认不会切换 active profile。如果新建 profile 应该立刻成为 active profile，请传 `--use`。已有 profile 会受到误替换保护；如果确实要替换，请传 `--overwrite`。

## Profile 配置

CLI 将非敏感 profile 元数据存储在操作系统配置目录下的 `config.toml` 中：

- macOS：`~/Library/Application Support/apollo/config.toml`
- Linux：`$XDG_CONFIG_HOME/apollo/config.toml` 或 `~/.config/apollo/config.toml`
- Windows：`%APPDATA%\apollo\config.toml`

配置文件会存储：

- `active_profile`
- profile 名称
- profile `server`
- profile `output`
- profile `auth_mode`，取值为 `user-token` 或 `consumer-token`
- 可选 `operator`
- 可选凭据查找元数据，例如 backend/key 名称

Token 不属于受支持的配置 schema，CLI 有意不把 token 放进这里。建议使用 `apollo init`、`apollo profile add` 和 `apollo auth login`，而不是手工编辑这个文件。

如果缺少 `auth_mode`，CLI 会把该 profile 当作 `consumer-token` 处理，以兼容已有配置。

示例：

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

## Profile 命令

- `apollo init`
- `apollo profile add [name]`
- `apollo profile list`
- `apollo profile show`
- `apollo profile use <name>`

运行时上下文按以下顺序解析：

1. 显式参数，例如 `--profile`、`--server`、`--output`
2. 环境变量，例如 `APOLLO_PROFILE`、`APOLLO_SERVER`、`APOLLO_OUTPUT`
3. active profile 配置
4. 命令默认值

## Auth 命令

- `apollo auth login`
- `apollo auth login --token-stdin`
- `apollo auth login --token-stdin --store-token-in-file`
- `apollo auth status`
- `apollo auth whoami`
- `apollo auth capabilities`
- `apollo auth logout`

凭据存储使用内部 store 抽象，包含以下逻辑 provider：

- `native`：默认凭据后端，通过 Rust `keyring` crate 使用操作系统凭据存储
- `env`：通过 `APOLLO_TOKEN` 提供只读的 CI/headless provider
- `file`：只有显式传 `--store-token-in-file` 时才启用的文件回退
- 用于单元测试的内存 provider

Native 后端选择遵循底层操作系统凭据存储行为：

- macOS：Keychain Services
- Windows：Credential Manager
- Linux desktop：兼容 freedesktop Secret Service 的 provider
- Linux headless/CI：使用 `APOLLO_TOKEN`，或显式选择文件回退

文件回退会把 token material 写到 `config.toml` 之外的 CLI 凭据目录中，并在 Unix 上使用受限文件权限。Profile 配置只存储非敏感凭据查找元数据。

OpenAPI 命令支持两种 token 模式：

- `user-token`：推荐用于交互式用户、AI agent 和本地自动化。Token 以 `apollo_pat_` 开头，并以 `Authorization: Bearer <token>` 形式发送。
- `consumer-token`：用于兼容已有集成和旧版本 Apollo 部署。Token 会作为原始 `Authorization: <token>` header 值发送。

`apollo init` 和 `apollo profile add` 默认使用 `user-token`。配置旧版 consumer-token 凭据时请使用 `--auth-mode consumer-token`。变更类命令只在 `consumer-token` 模式下要求配置 `operator`；user-token 请求会使用所属 Portal 用户。

`apollo env list`、`apollo namespace list`、`apollo namespace get`、`apollo config get`、`apollo config list`、`apollo config diff` 和 `apollo config apply` 会读取 env、namespace 或配置项数据，这些数据的授权范围可能比 app 更窄。请在这些命令中使用 `user-token` 模式。旧版 `consumer-token` 模式无法从可用的 `/openapi/v1/apps/authorized` 响应中安全校验 env/namespace 级读取范围，因此 CLI 会拒绝这些 scoped-read 工作流，而不是只依赖 app 级可见性。

本地或 CI 使用时，`APOLLO_TOKEN` 优先级最高，并且永远不会写入磁盘：

```bash
APOLLO_TOKEN="$TOKEN" apollo --server http://localhost:8070 app list --output json
```

当 `APOLLO_TOKEN` 以 `apollo_pat_` 开头时，CLI 会自动将其视为 `user-token`；否则使用 `consumer-token` 兼容模式。

`apollo auth logout` 会删除所选 profile 引用的凭据。它不能从父 shell 环境中删除 `APOLLO_TOKEN`。如果 `APOLLO_TOKEN` 仍然存在，logout 会提示环境凭据仍会继续生效；执行 `unset APOLLO_TOKEN` 可以禁用这个临时凭据。

交互式使用时，先配置 profile，再用隐藏输入保存 token：

```bash
apollo --profile dev auth login
apollo --profile dev app list
```

`auth login` 默认把 token 存到操作系统凭据存储。如果 native store 在交互式终端中不可用，CLI 会询问是否改用本地文件回退。

脚本或手工粘贴并回车的场景下，`--token-stdin` 会读取一行 token：

```bash
printf '%s\n' "$TOKEN" | apollo --profile dev auth login --token-stdin
apollo --profile dev auth login --token-stdin
printf '%s\n' "$LEGACY_CONSUMER_TOKEN" | apollo --profile legacy auth login --auth-mode consumer-token --token-stdin
```

登录后可使用 user-token 自检命令：

```bash
apollo --profile dev auth whoami
apollo --profile dev auth capabilities
```

这些命令会调用 `/openapi/v1/user-tokens/current` 和 `/openapi/v1/user-tokens/current/capabilities`。它们要求使用 `user-token` 鉴权模式，并且不会创建、轮转或撤销 token；用户 token 的创建仍然是 Portal 自助流程。

## 脱敏和错误

输出层会在渲染前对人类可读输出和 JSON 输出应用保守脱敏。类 token 字段、`Authorization: Bearer ...` header 和 `consumer token ...` 文本都会渲染为 `[REDACTED]`。

结构化 JSON 错误包含：

- `code`：稳定错误码
- `category`：稳定错误分类
- `message`：人类可读错误信息
- 可选的非敏感详情，例如 `command`、`profile`、`path` 或 `follow_up_issue`

当前错误分类：

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

## OpenAPI 行为

第一版 v0 实现使用一个小型通用 HTTP client，而不是生成式 SDK。这样可以让 CLI 与 Apollo 服务端仓库解耦，同时仍然保证所有内置资源命令都限定在 `/openapi/v1/*`。

路径和 payload 映射遵循当前 Apollo Portal OpenAPI contract，包括：

- `GET /openapi/v1/apps`
- `GET /openapi/v1/envs`
- `GET /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces`
- `GET|PUT|DELETE /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/items/{key}`
- `POST /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/items/diff`
- `POST /openapi/v1/apps/{appId}/appnamespaces` 用于 AppNamespace 注册
- `POST /openapi/v1/namespaces`
- `POST /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/items/synchronize`
- `GET /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/releases/active`
- `POST /openapi/v1/envs/{env}/apps/{appId}/clusters/{clusterName}/namespaces/{namespaceName}/releases`
- `PUT /openapi/v1/envs/{env}/releases/{releaseId}/rollback`

变更类命令要求传 `--yes`。如果没有传，CLI 会在建立网络连接之前返回 `confirmation_required`。

## 本地开发

构建 CLI：

```bash
cargo build
```

运行 help 输出：

```bash
cargo run -- --help
```

运行测试：

```bash
cargo test
```

使用本地 mock HTTP server 运行聚焦的 OpenAPI 命令集成测试：

```bash
cargo test --test openapi
```

如果本地运行了 Apollo Portal，也可以直接对它做 smoke test：

```bash
APOLLO_TOKEN="$TOKEN" cargo run -- --server http://localhost:8070 --output json env list --app sample-app
APOLLO_TOKEN="$TOKEN" cargo run -- --server http://localhost:8070 --output json app list
APOLLO_TOKEN="$USER_TOKEN" cargo run -- --server http://localhost:8070 --output json auth whoami
```

格式化仓库：

```bash
cargo fmt
```

运行 lint：

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## 仓库结构

- `src/cli.rs`：CLI 定义和参数解析
- `src/config.rs`：profile 配置加载、保存和上下文解析
- `src/command.rs`：顶层命令路由
- `src/credential.rs`：凭据存储抽象和 provider
- `src/error.rs`：结构化 CLI 错误模型
- `src/http.rs`：通用 OpenAPI HTTP client 和路径 helper
- `src/output.rs`：输出渲染抽象
- `src/redaction.rs`：保守脱敏工具
- `tests/auth.rs`：auth 命令和凭据行为的集成覆盖
- `tests/cli.rs`：help、参数和结构化错误的集成覆盖
- `tests/openapi.rs`：OpenAPI path、鉴权 header 和确认保护的集成覆盖
- `tests/profile.rs`：profile 命令和上下文解析的集成覆盖
- `tests/redaction.rs`：脱敏行为的集成覆盖

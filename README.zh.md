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

`--yes` 表示在不显示交互提示的情况下显式批准一次 OpenAPI 变更请求。它不会跳过目标计划构建、参数校验、脱敏或操作信息输出。

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

如需在隔离的 CI 或 smoke 环境中保存 `config.toml` 和文件型凭据，可将
`APOLLO_CLI_HOME` 设置为绝对目录。日常交互式使用仍建议保留平台默认路径。

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
- 可选的非敏感详情，例如 `command`、`profile`、`path` 或 `operation`

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

进程退出状态在以下层级保持稳定：

- `0`：成功
- `1`：运行期或操作失败，包括鉴权、校验、网络/服务端和确认失败
- `2`：命令行解析或用法错误

当自动化调用方需要比进程退出状态更具体的失败原因时，应使用结构化 JSON 中的 `error.code` 和 `error.category`。

## 变更安全

在执行内置 namespace、config、release 或 raw API 变更前，CLI 会根据选中的 profile/server 和命令目标构建一份脱敏操作计划。计划会按操作类型包含可用字段，例如 app、env、cluster、namespace、配置 key/数量、release ID，或经过净化的 raw OpenAPI method 和 path。计划不会包含配置值、请求 body、query 值、token 或 Authorization header。

在交互式 table 模式中，未传 `--yes` 的变更会把计划和默认拒绝的 `[y/N]` 提示写到 stderr。只有输入 `y` 或 `yes` 才会执行；输入 `n`、`no`、空行或遇到 EOF 都会拒绝。在非交互模式和 JSON 模式中，变更必须显式传入 `--yes`；否则 CLI 返回 `confirmation_required`，其 `operation` 字段包含脱敏计划。拒绝发生在任何 OpenAPI 请求发送之前。

namespace 创建只会在首次批准后发送只读预检请求。如果 Apollo 解析出的最终 namespace 名称发生变化，例如为公共 namespace 添加组织前缀，CLI 会展示解析后的计划，并在发送任何变更请求前再次要求批准。如果批准后选中的 profile、server 或输出模式发生变化，CLI 会在发送 OpenAPI 请求前中止，并要求调用方重新检查新的运行上下文。

传入 `--yes` 时，table 模式仍会在请求前输出计划。成功的 JSON 输出仍是一个完整 JSON 文档，并保留现有顶层 `status` 和 `data` 字段，同时新增顶层 `operation` 计划。

### 配置同步约定

`config diff` 和 `config apply` 采用保守合并约定：只存在于源端的 key 会被创建，源端 value 或 comment 不同的 key 会被更新，相同 key 保持不变，只存在于目标端的 key 会被保留。因此，空源端会得到成功的 no-op，而不会清空目标端。CLI 当前不提供 `--prune`；删除必须通过独立、显式的配置删除流程完成。如果某个 Portal 版本通过同步 diff 接口返回删除操作，CLI 会拒绝该计划，并且不会调用 `items/synchronize`。

table 和 JSON 输出都会给出源端与目标端 scope，以及 `create`、`update`、`delete`、`unchanged` 计数。JSON 约定还会返回 `strategy: "merge"` 和 `targetOnlyBehavior: "preserve"`。diff 结果、apply 计划和 apply 结果都不会包含配置 value。

单独执行的 `config diff` 仅供参考，不会生成可供后续调用消费的计划制品。`config apply` 会自行捕获完整分页的源端快照，用这份快照调用 `items/diff`，同时捕获完整分页的目标端状态，并根据返回的变更集生成详细变更计划。批准后，CLI 会重新读取目标端，并使用同一份已捕获的源端快照再次评估；如果目标端配置状态或评估结果发生变化，命令会返回 `stale_plan`，且不会发送同步请求。首次批准有意发生在这些预检读取之前，第二次详细批准则确认实际变更计数。如果所有变更计数均为零，命令会返回确定性的 `data.result: "no-op"` 成功结果，并且不会调用 `items/synchronize`。

该 stale-plan 检查属于乐观、best-effort 防护。当前 Apollo `items/synchronize` OpenAPI 约定没有目标 revision、ETag 或条件写入前置条件，因此最终检查之后发生的目标端写入仍可能与同步竞争。要彻底消除这个窗口，需要先修改 Apollo OpenAPI contract，再由服务端在更新配置的同一原子操作中校验目标 revision。对写入互斥有严格要求的调用方，在当前 CLI 工作流之外仍需自行协调。

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

变更类命令的确认和操作计划行为见[变更安全](#变更安全)。

## 可执行文件发布

手动运行发布 workflow 后，预编译的 `apollo` 可执行文件会发布到
[GitHub Releases 页面](https://github.com/apolloconfig/apollo-cli/releases)：

| 平台 | Rust target | 压缩格式 |
|---|---|---|
| Linux x86-64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| macOS Apple 芯片 | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86-64 | `x86_64-pc-windows-msvc` | `.zip` |

压缩包统一命名为 `apollo-<tag>-<target>.<extension>`，其中包含可执行文件、许可证以及中英文
README。每个 Release 还会提供 `SHA256SUMS`，并为全部上传的压缩包和 checksum 文件生成 GitHub
构建来源证明。

例如，可以使用 GitHub CLI 下载并校验一个版本：

```bash
version=v0.1.0
mkdir "apollo-${version}"
gh release download "${version}" \
  --repo apolloconfig/apollo-cli \
  --dir "apollo-${version}"
(cd "apollo-${version}" && sha256sum --check SHA256SUMS)
gh attestation verify \
  "apollo-${version}/apollo-${version}-x86_64-unknown-linux-gnu.tar.gz" \
  --repo apolloconfig/apollo-cli
```

macOS 的 checksum 校验命令请使用 `shasum -a 256 -c SHA256SUMS`。

维护者发布时，先把目标 package version 和更新后的 `Cargo.lock` 合并到默认分支。然后进入
**Actions → Release → Run workflow**，保持选择默认分支，并填写不带前导 `v` 的 SemVer。
也可以通过 GitHub CLI 启动同一条发布流程：

```bash
gh workflow run release.yml \
  --repo apolloconfig/apollo-cli \
  --ref main \
  -f version=0.1.0
```

workflow 会拒绝格式错误、与 `Cargo.toml` 不一致、不是从默认分支启动，或者 tag/Release 已存在
的版本。随后重新执行格式化、Clippy 和测试，为五个原生 target 构建并冒烟验证可执行文件，生成
checksum 与构建来源证明。全部检查通过后，才会在本次 workflow 的精确 commit 上创建
`v<version>` tag，自动生成 release notes，核对草稿 Release 的完整附件集合并公开发布。版本中含有
SemVer 预发布后缀时，会发布为 prerelease。

发版前，维护者还必须在目标默认分支 commit 上运行 **Actions → Apollo mutation smoke → Run
workflow**，并确认执行成功；该 workflow 也会每周定时运行。它使用固定 Apollo revision 构建
Portal、ConfigService、AdminService 和一次性 H2 数据库，再执行下文所述的真实变更约定。Apollo
固定 revision 保存在 `scripts/mutation-smoke.sh` 中，更新它必须作为代码变更接受 review。

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

从干净 checkout 运行可重复的真实 Portal 变更 smoke（需要 Git、curl、jq、awk、
`sha256sum` 或 `shasum`、JDK 17 和稳定版 Rust 工具链）：

```bash
./scripts/mutation-smoke.sh
```

该命令会拉取固定 Apollo revision、构建并启动单进程 Portal + H2 assembly、构建 CLI、创建隔离的
`APOLLO_CLI_HOME` profile 和一次性 app/namespace，并基于真实服务端状态验证 config diff/apply、
缺少确认时拒绝、权限失败、release 创建/列表以及回滚。测试会确认保守合并保留目标端独有 key，
并比较 no-op 前后的完整目标状态。User token 和配置值只保存在权限为 `0700` 的临时目录中，失败
诊断会动态脱敏，assembly 进程与临时目录始终会被清理。若本地已有同一固定 commit 的干净 Apollo
checkout，可以复用：

```bash
APOLLO_SMOKE_APOLLO_SOURCE=/absolute/path/to/apollo ./scripts/mutation-smoke.sh
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
- `scripts/mutation-smoke.sh`：固定 Apollo Portal + H2 变更 smoke 与状态断言

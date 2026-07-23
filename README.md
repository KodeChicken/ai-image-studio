# AI Image Studio

[中文](#中文) | [English](#english)

## 中文

AI Image Studio 是一个可独立部署的多供应商 AI 图片生成平台。它以多轮创作会话为核心，统一管理 Provider、模型能力、SSE 任务事件以及 Local/S3 图片文件。

### 当前能力

- 用户名密码登录、默认管理员 Bootstrap、仅默认管理员首次登录强制改密
- 管理员用户管理与用户级 Provider 隔离
- 支持任意自定义 Host 的 OpenAI Compatible、Google Gemini Image、xAI Grok Image Provider 与动态模型发现
- 基于官方模型目录的动态参数 Schema
- 同一会话中的连续问答、生图、当前分支切换/重新生成和历史图片自动上下文选择
- 浏览器默认 SSE 流式任务事件与 `Last-Event-ID` 续传，断线时显示恢复/轮询状态；上游支持时实时展示短期局部预览
- 生成中任务可取消，停止上游请求并把会话助手消息持久化为取消终态
- 失败任务保留错误摘要，可在原助手消息上重试同一任务并继续 SSE 进度
- 输入图片和生成结果真实文件持久化，不依赖上游临时链接
- 上传后任务创建失败会恢复创作草稿，并安全清理未被采用的图片文件
- Local 与 S3 Compatible 存储，可混合读取历史数据
- Light/Dark/System 主题、历史作品和 Prompt 风格模板
- Redis 可选任务队列、独立 Worker、数据库兜底取任务与自动重试
- 用户用量/分币种成本中心，以及管理员任务统计和脱敏 Provider 请求日志
- Provider 真实测试生图（可编辑提示词和默认参数）、管理员模型价格维护与普通用户只读查看
- 用户/Session/IP 分层限流，以及数据库与 Local/S3 定时一致性扫描和孤儿文件宽限期清理
- 受控版本检查与 Host Updater 委托式升级/回滚管理；Web 容器不访问 Docker Socket
- Vue 3 + TypeScript 前端和 Rust + Axum 模块化单体后端

### 快速启动

要求：Docker Desktop 或 Docker Engine + Compose。

```bash
cp .env.example .env
docker compose up --build
```

Windows PowerShell：

```powershell
Copy-Item .env.example .env
docker compose up --build
```

打开 `http://127.0.0.1:3100`。

首次启动默认账号：

```text
用户名：admin
密码：123456
```

默认管理员首次登录后必须修改密码；普通用户登录后不会被强制改密，仍可从账户菜单主动修改。生产部署前还必须在 `.env` 中替换 `SESSION_SECRET`、`CREDENTIAL_MASTER_KEY` 和 PostgreSQL 密码。

### 本地开发

启动 PostgreSQL 后配置环境变量，然后运行：

```bash
cargo run --package ai-image-studio -- migrate
cargo run --package ai-image-studio -- serve
```

默认样例使用 Redis Worker 模式，还需在另一个终端运行：

```bash
cargo run --package ai-image-studio -- worker
```

不使用 Redis 时，将 `TASK_EXECUTION_MODE=inline`，任务由 API 进程执行，无需启动 Worker。

前端开发服务器：

```bash
cd frontend
pnpm install
pnpm dev
```

Vite 默认监听 `http://127.0.0.1:5173`，并将 `/api` 转发到 `http://127.0.0.1:3000`。

### 验证命令

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 需要 Docker，验证真实 Redis 队列与独立 Worker
pwsh -File backend/tests/redis_worker_integration.ps1

# 需要 Docker，验证 MinIO 配置切换与 Local/S3 混合读取
pwsh -File backend/tests/storage_admin_integration.ps1

cd frontend
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

### 存储语义

`image_assets.storage_driver` 区分 `local` 与 `s3`。数据库保存的是本地相对路径或 S3 Object Key，不保存宿主机绝对路径、公开 URL 或临时签名 URL。切换主写入驱动不会影响读取旧存储中的图片。

### Host Updater

在线升级需要把独立的 `ai-image-studio-host-updater` 安装为宿主机服务。它只监听专用 Unix Socket 或回环地址，通过 Bearer + HMAC 验证 Web 请求，并调用固定执行器完成备份、Cosign/Digest 校验、Migration、候选健康检查、切换和失败恢复。Web 容器不会获得 Docker Socket。

Release Manifest 对外文件使用 snake_case；Web API 返回浏览器时仍使用 camelCase。内部 HTTPS 服务使用私有 CA 时，将 CA 的 PEM 文件挂载到容器并通过绝对路径 `HTTP_CA_CERT_FILE` 配置，TLS 校验不会被关闭。仓库演练已从 Web 管理 API 经 Unix Socket 跑通 Host Updater、正式执行器和部署历史终态回写。

安装、配置、S3 备份证据和隔离环境故障演练见 [`docs/Host_Updater部署与故障演练.md`](./docs/Host_Updater部署与故障演练.md)。

完整方案、数据库关系和 UI 原型位于 [`docs/`](./docs/README.md)。

---

## English

AI Image Studio is a self-hosted, multi-provider image generation workspace built around multi-turn creative conversations. It provides one model for providers, model capabilities, SSE task events, and durable Local/S3 image storage.

### Current capabilities

- Password authentication, bootstrap administrator, and mandatory first-login password change for the default administrator only
- Administrator user management and per-user provider isolation
- OpenAI-compatible providers with arbitrary custom hosts, plus Google Gemini Image and xAI Grok Image with dynamic model discovery
- Dynamic parameter schemas backed by a versioned official model catalog
- Multi-turn image conversations with branch switching/regeneration and automatic, bounded historical image context
- SSE task events by default, including visible reconnect/polling states, `Last-Event-ID` recovery, and temporary native partial previews when supported upstream
- Active generations can be cancelled, stopping upstream work and persisting a terminal cancelled assistant message
- Failed tasks retain an error summary and can retry the same task with resumed SSE progress
- Durable input and generated image files instead of expiring upstream URLs
- Failed task creation restores the composer draft and safely removes unused uploaded files
- Local and S3-compatible storage with mixed historical reads
- Light/Dark/System themes, history gallery, and prompt-based style templates
- Optional Redis queue, independent worker, durable database fallback, and automatic retries
- Per-user usage/cost views plus administrator task analytics and redacted provider request logs
- Real provider test generations with editable prompts/default parameters, administrator-managed model pricing, and read-only pricing for ordinary users
- Layered user/session/IP rate limits plus scheduled database/Local/S3 consistency scans and grace-period orphan cleanup
- Controlled release checks and delegated Host Updater upgrade/rollback management; the web container never accesses the Docker socket
- Vue 3 + TypeScript frontend and Rust + Axum modular-monolith backend

### Quick start

Requirements: Docker Desktop or Docker Engine with Compose.

```bash
cp .env.example .env
docker compose up --build
```

On Windows PowerShell:

```powershell
Copy-Item .env.example .env
docker compose up --build
```

Open `http://127.0.0.1:3100`.

Default first-run credentials:

```text
Username: admin
Password: 123456
```

The default administrator must change the password after the first login. Ordinary users are not forced to change their password on login, but can still change it from the account menu. Before production use, replace `SESSION_SECRET`, `CREDENTIAL_MASTER_KEY`, and the PostgreSQL password in `.env`.

### Local development

After starting PostgreSQL and configuring the environment:

```bash
cargo run --package ai-image-studio -- migrate
cargo run --package ai-image-studio -- serve
```

The default sample configuration uses Redis worker mode. Run this in another terminal:

```bash
cargo run --package ai-image-studio -- worker
```

Set `TASK_EXECUTION_MODE=inline` to run tasks inside the API process without Redis or a separate worker.

Run the frontend development server:

```bash
cd frontend
pnpm install
pnpm dev
```

Vite listens on `http://127.0.0.1:5173` and proxies `/api` to `http://127.0.0.1:3000`.

### Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Requires Docker; verifies the real Redis queue and independent worker
pwsh -File backend/tests/redis_worker_integration.ps1

# Requires Docker; verifies MinIO switching and mixed Local/S3 reads
pwsh -File backend/tests/storage_admin_integration.ps1

cd frontend
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

### Storage semantics

`image_assets.storage_driver` distinguishes `local` and `s3`. The database stores a relative local path or S3 Object Key—never a host absolute path, public URL, or temporary signed URL. Changing the primary write driver does not prevent historical assets from being read from their original storage.

### Host Updater

Online upgrades require the independent `ai-image-studio-host-updater` host service. It binds only to a dedicated Unix socket or loopback address, authenticates web requests with Bearer + HMAC, and invokes one fixed executor for backups, Cosign/digest verification, migrations, candidate health checks, switching, and automatic recovery. The web container never receives the Docker socket.

The downloadable Release Manifest uses snake_case, while the Web API serializes its browser response as camelCase. For internal HTTPS endpoints backed by a private CA, mount the CA PEM and configure its absolute path through `HTTP_CA_CERT_FILE`; TLS verification remains enabled. The repository drill now covers the complete path from the Web admin API through the Unix socket, Host Updater, fixed executor, and deployment-history synchronization.

The Chinese operations guide covers installation, configuration, S3 backup evidence, and isolated failure drills: [`docs/Host_Updater部署与故障演练.md`](./docs/Host_Updater部署与故障演练.md).

The detailed engineering design, database relationships, and UI prototype remain in Chinese under [`docs/`](./docs/README.md).

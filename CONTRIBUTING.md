# 参与贡献

感谢参与 AI Image Studio。提交改动前请先确认变更直接服务于当前需求，避免混入无关重构。

## 开发环境

1. 复制 `.env.example` 为 `.env`，按需修改本地配置。
2. 使用 `docker compose up -d db redis` 启动 PostgreSQL 与 Redis。
3. 后端执行 `cargo run --package ai-image-studio`。
4. 前端进入 `frontend/` 后执行 `pnpm install` 和 `pnpm dev`。

完整启动方式见根目录 [README.md](./README.md)。

## 提交前检查

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pwsh -File backend/tests/core_api_integration.ps1
pwsh -File backend/tests/redis_worker_integration.ps1
pwsh -File backend/tests/storage_admin_integration.ps1
cd frontend
pnpm typecheck
pnpm lint
pnpm test
pnpm build
pnpm test:e2e
```

涉及数据库结构时必须新增向前迁移，不修改已经发布的 Migration。涉及 UI 时应同步核对 `docs/ui-prototype.html` 和 `docs/UI原型设计说明.md`。

## Pull Request

- 一个 PR 只解决一个清晰问题。
- 说明行为变化、验证命令和可能的迁移/回滚影响。
- 不提交 API Key、密码、Cookie、真实用户图片或其他敏感数据。
- 新增 Provider 参数必须给出官方文档或 Provider 契约依据。

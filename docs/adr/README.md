# 架构决策记录

本目录记录已经影响代码结构、数据兼容性或运维边界的重要决策。ADR 只描述决策及取舍，具体接口和字段仍以工程方案与 Migration 为准。

- [0001：使用 Rust 与 Axum](./0001-use-rust-and-axum.md)
- [0002：使用 Vue 3 与 TypeScript](./0002-use-vue3-and-typescript.md)
- [0003：Provider Adapter 架构](./0003-provider-plugin-architecture.md)
- [0004：图片文件存储在 PostgreSQL 之外](./0004-store-images-outside-postgresql.md)
- [0005：首版采用模块化单体](./0005-start-with-modular-monolith.md)
- [0006：以会话为中心组织生图](./0006-conversation-first-image-generation.md)
- [0007：默认使用 SSE 推送任务事件](./0007-use-sse-as-default-task-stream.md)
- [0008：可插拔 Local/S3 存储](./0008-pluggable-local-s3-storage.md)

# 变更日志

本文件记录对用户可见的重要变化。版本格式遵循语义化版本。

## [未发布]

### 新增

- 基于 Konva 的全屏非破坏性图片编辑器，支持原图像素裁剪、独立成品尺寸、Cover/Contain/Free/Stretch、变换、背景、撤销重做和刷新恢复。
- 编辑文档与派生 Asset 数据模型、PNG/JPEG/WebP 高清确定性导出，以及显式模型能力门控的 AI 扩图任务上下文。
- OpenAI 图片原生 SSE、模型级动态参数及 GPT Image 2 自定义尺寸校验。
- Local/S3 混合图片资产读取、存储一致性扫描和 Host Updater 演练。
- 会话当前分支浏览、从历史消息继续、重新生成和同级分支切换。
- 个人设置三态主题、账户菜单完整关闭行为和退出登录确认。
- 创作台生成取消入口，以及任务、助手消息和 SSE 事件一致的 `cancelled` 终态。
- 创作会话标题即时搜索，以及历史作品会话/Provider/模型/日期/尺寸完整筛选。
- 失败任务原消息重试、可见 SSE 恢复状态，以及任务创建失败时的草稿恢复和上传 Asset 补偿清理。
- Host Updater 真实 Docker 隔离演练，覆盖固定镜像 Digest 成功链，以及 Digest、Migration、候选、正式切换四类失败恢复；PostgreSQL Dump、S3 备份和升级后历史图片均做真实恢复与校验。
- Web 管理 API 经 Bearer + HMAC、Unix Socket、Host Updater 和正式执行器完成升级并幂等回写部署历史的真实 Docker 整链演练。

### 修复

- 创作台和历史作品统一进入“编辑成品”，宽高默认不联动，4K 原图可精确保留 `261px` 等用户输入值且不受预览尺寸影响。
- Release Manifest 同时兼容 snake_case 发布契约与 Web camelCase 响应，并支持通过 `HTTP_CA_CERT_FILE` 信任内部 HTTPS 根 CA。
- Release Manifest 检查安全跟随受限的 HTTPS 重定向，兼容 GitHub Release 下载地址且不向跨域目标转发 Token。
- Host Updater 正确解析执行器的 camelCase 进度消息，执行升级时不再收紧既有图片挂载目录权限。

## [0.1.0] - 2026-07-21

### 新增

- Vue 3 创作台、历史作品、Provider、用量、用户和系统管理页面。
- Rust/Axum API、多轮图片会话、默认 SSE、任务恢复与取消。
- OpenAI Compatible、Gemini Image、Grok Image Adapter。
- PostgreSQL、Redis Worker、Local/S3 存储和 Docker Compose 部署。

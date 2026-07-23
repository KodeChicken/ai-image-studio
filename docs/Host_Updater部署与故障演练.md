# Host Updater 部署与故障演练

Host Updater 是默认随 Docker Compose 启动的独立控制 sidecar。Web 应用只负责管理员鉴权、二次输密、Manifest 预检和任务展示；Docker、备份目录及发布切换权限只交给 Updater 容器。无需在 VPS 额外安装 systemd 服务。

## 1. 已实现边界

- 默认只监听共享命名卷中的 Unix Socket，不向宿主机或公网发布端口。
- 首次启动自动生成 32 字节随机 Token；Web 只读挂载 Socket/Token 卷。
- `POST /v1/jobs` 和 `GET /v1/jobs/{id}` 同时校验 Bearer Token、时间戳和 HMAC-SHA256，请求时间偏差最多 60 秒。
- Job ID 幂等；同一时间只允许一个升级或回滚任务。
- Job 状态写入宿主机独立目录，服务重启后不会把中断任务误报为成功。
- 只调用 `HOST_UPDATER_EXECUTOR_PATH` 指定的绝对路径执行器，不接收请求方提供的命令、脚本路径或环境变量。
- 可配置执行器 SHA-256，防止已安装脚本被静默替换。
- 固定执行器执行数据库和图片备份、S3 备份证据校验、镜像 Digest 校验、SQLx Migration、候选容器健康检查、正式切换、失败恢复及历史版本保留。
- 成功结果包含版本、镜像、Digest、Schema 和备份清单；Web 同步后写入 `deployment_history`。

## 2. 宿主机依赖

生产宿主机需要：

- Docker Engine 和 Docker Compose v2
- 可用的 `/var/run/docker.sock`
- 本仓库的 Compose 项目

`curl`、`jq`、Docker CLI、Compose v2、备份工具和固定执行器均已放入 Updater 镜像，不要求逐项安装到 VPS。Host Updater 不放进 Web 容器，也不通过 TCP 暴露到公网。

## 3. 首次部署与旧部署迁移

```bash
git pull --ff-only
docker compose up -d --build
docker compose ps
```

成功标准：`db`、`redis`、`updater`、`app`、`worker` 正常运行，`migrate` 成功退出。检查 Updater：

```bash
docker compose logs --tail=100 updater
docker compose exec updater curl --fail --silent \
  --unix-socket /run/ai-image-studio-updater/host-updater.sock \
  http://localhost/health
```

内部 Token 位于 Compose 命名卷，只由 Updater 写入，Web 只读访问，不需要人工生成或复制。`app` 与 `worker` 都没有 Docker Socket；可用以下命令核对：

```bash
docker inspect "$(docker compose ps -q app)" --format '{{json .Mounts}}'
docker inspect "$(docker compose ps -q worker)" --format '{{json .Mounts}}'
docker inspect "$(docker compose ps -q updater)" --format '{{json .Mounts}}'
```

Compose 固定使用项目名 `ai-image-studio`。如果旧版曾安装 systemd Updater，新 Compose 验证正常后可停止旧服务，避免保留不再使用的高权限进程：

```bash
sudo systemctl disable --now ai-image-studio-host-updater
```

这不会删除旧的二进制和配置；确认无需回退后再由管理员自行清理。

### 3.1 私有 GitHub/GHCR

公开仓库和公开 Package 无需附加凭据。当前仓库若为私有，需要一次性完成：

```bash
docker login ghcr.io
```

登录凭据默认保存在 VPS 的 `/root/.docker`，Compose 以只读方式挂给 Updater。另在 `.env` 填写一个具有仓库 Release 读取权限的 Token：

```env
UPDATE_MANIFEST_TOKEN=replace-with-release-read-token
```

该 Token 用于 Web 与 Updater 读取私有 `release-manifest.json`，不需要随版本修改。不要把 Token 提交到 Git。

### 3.2 GitHub 正式发布配置

仓库的 [`.github/workflows/release.yml`](../.github/workflows/release.yml) 会在推送 `vMAJOR.MINOR.PATCH` Tag 时自动：

1. 重新执行后端和前端检查。
2. 使用 Buildx 分别构建 `linux/amd64` 应用镜像和 Updater 镜像并推送到 GHCR。
3. 获取两个镜像的不可变 Digest。
4. 根据 `backend/migrations` 的最高版本生成 `release-manifest.json`。
5. 创建 GitHub Release，上传 Manifest。

该流程不需要额外的签名密钥或 GitHub Actions Secrets。它与 Sub2API 的简化发布方式一致，信任 GitHub Release、HTTPS 和 Registry；执行器通过 `image@sha256:...` 固定并校验实际拉取的镜像内容。

确认 GHCR Package 对 VPS 可读。公开 Package 无需登录；私有 Package 按 3.1 节完成一次性登录。

正式发布示例：

```bash
git tag v0.1.1
git push origin v0.1.1
```

不要重复使用或强制移动已发布 Tag。需要修复时发布新的 Patch 版本。

## 4. Release Manifest

每个发布版本必须提供 HTTPS Manifest：

```json
{
  "version": "0.2.0",
  "image": "ghcr.io/codechicken/ai-image-studio:v0.2.0",
  "image_digest": "sha256:...",
  "updater_image": "ghcr.io/codechicken/ai-image-studio-updater:v0.2.0",
  "updater_image_digest": "sha256:...",
  "schema_target": 10,
  "schema_min_supported": 9,
  "schema_max_supported": 12,
  "rollback_compatible_to": "0.1.0",
  "requires_backup": true,
  "destructive_migration": false,
  "minimum_updater_version": "0.1.0",
  "release_notes": "..."
}
```

`schema_min_supported` 和 `schema_max_supported` 是应用可读取的数据库兼容窗口；只声明 `schema_target` 不能安全判断旧镜像能否读取升级后的 Schema。

下载文件以以上 snake_case 字段作为发布契约，正式执行器直接按此格式校验。Web 后端同时兼容读取历史 camelCase Manifest，但返回浏览器的管理 API 仍按前端约定序列化为 camelCase。

执行器不会仅信任可变 Tag，而是把 Manifest 中的镜像地址和 Registry 返回的 Digest 组合为 `image@sha256:...` 后拉取；Digest 不存在或内容不匹配时 Docker 会拒绝继续。该方案信任 GitHub Release、HTTPS 和 GHCR 账号安全，不额外维护独立签名密钥。

## 5. Local 与 S3 备份

Local 模式在停写后生成：

- `database.dump`
- `images.tar.gz`
- `images-manifest.json`，逐文件记录相对 Key、大小和 SHA-256
- `backup-manifest.json`

S3 是主存储，不等于已经备份。S3 模式要求独立备份任务在 `S3_BACKUP_REFERENCE_FILE` 中写入最近一次不可变快照、对象版本或跨 Bucket 备份引用。执行器会检查文件存在、非空且未超过 `S3_BACKUP_MAX_AGE_SECONDS`，否则拒绝升级。

## 6. 启动、停止与日志

```bash
docker compose up -d
docker compose restart updater
docker compose logs -f updater
```

`app` 会等待 Updater healthcheck 通过再启动。健康接口只证明 Updater 进程、Unix Socket 和 Token 已就绪；升级是否可执行还取决于磁盘、数据库、备份证据、Manifest 和 Registry 登录态。

## 7. 执行顺序

1. 校验配置、数据库健康、Schema 兼容窗口和磁盘空间。
2. 在旧服务继续运行时按 Digest 拉取预构建镜像；下载或 Digest 校验失败不会造成业务停机，也不会在 VPS 编译源码。
3. 停止 `app` 与 `worker`，保持 PostgreSQL/Redis 运行，形成一致停写点。
4. 备份数据库，并备份 Local 图片或验证 S3 独立备份证据。
5. 升级时运行目标镜像内的 SQLx Migrator，并核对最终 Schema。
6. 在同一 Compose 网络启动临时候选容器，通过容器 DNS 检查 `/api/v1/ready`，不占用额外宿主机端口。
7. 候选成功后把目标 Digest 标记为本地 `ai-image-studio:active`，写入 `data/updater/release.env`，切换正式 `app/worker` 并再次健康检查。
8. 写入宿主机 `history.json`、Release 记录和数据库 `deployment_history`。
9. 只保留当前版本及之前三个版本的宿主机记录和备份。

任何错误都会停止候选/新应用、恢复上一份 `release.env` 并把活动别名重新指向旧 Image ID；Migration 已开始时还会恢复同批次数据库备份和 Local 图片备份，再启动旧应用并执行健康检查。因为 Compose 始终使用稳定活动别名，宿主机以后普通执行 `docker compose up -d` 不会被旧 `.env` 回退版本。

## 8. 真实 Docker 隔离成功链

[`executor_real_docker_drill.sh`](../host-updater/tests/executor_real_docker_drill.sh) 不替换 `docker`、PostgreSQL 或对象存储命令。它会启动隔离的 PostgreSQL、Redis 和 MinIO，运行正式 `execute-update.sh`，并使用调用方提前放入隔离 Registry、由 Digest 固定的真实镜像。

先构建目标镜像并将其推送到隔离 Registry；传给脚本的 Digest 必须是 Registry 返回的 Manifest Digest。然后运行：

```bash
TARGET_IMAGE_TAG=localhost:5055/ai-image-studio:v0.2.0 \
TARGET_IMAGE_DIGEST=sha256:... \
bash host-updater/tests/executor_real_docker_drill.sh
```

如需验证两个不同应用镜像之间的升级，可额外传入 `CURRENT_IMAGE_TAG`、`CURRENT_IMAGE_DIGEST` 和 `CURRENT_SCHEMA_VERSION`；未传时使用目标镜像作为初始运行镜像，只验证执行器真实成功链。

脚本会自动完成并断言：

1. 从本地 HTTPS Release Manifest 读取固定 Digest。
2. 在隔离应用仍然 Ready 时按 Digest 拉取镜像，并由 Docker 验证 Registry Manifest Digest。
3. 停止隔离应用，生成真实 PostgreSQL Custom Dump。
4. 把上传到主 MinIO Bucket 的图片镜像到独立备份 Bucket，并写入新鲜 S3 备份引用。
5. 使用目标镜像运行 SQLx Migration，启动候选容器并检查 Ready。
6. 切换正式 `app/worker`，核对 `release.env`、`history.json` 和 `backup-manifest.json`。
7. 把数据库 Dump 恢复到新数据库，确认上传资产记录存在。
8. 分别从备份 Bucket 和升级后应用读取历史图片，与原图做 SHA-256 比对。
9. 删除本次演练创建的隔离容器、网络、Volume 和临时目录。

成功标记如下：

```text
HOST_UPDATER_REAL_DOCKER_DRILL_OK pinned_digest=1 https_manifest=1 postgres_backup_restore=1 s3_backup_restore=1 migration=1 candidate_ready=1 active_ready=1 historical_s3_image=1
```

还可以用同一套真实依赖验证 Web 控制面整链。`HOST_UPDATER_BIN` 必须指向当前代码构建出的 Linux 可执行文件：

```bash
DRILL_TRIGGER=web \
TARGET_IMAGE_TAG=localhost:5055/ai-image-studio:v0.2.0 \
TARGET_IMAGE_DIGEST=sha256:... \
HOST_UPDATER_BIN=/usr/local/bin/ai-image-studio-host-updater \
bash host-updater/tests/executor_real_docker_drill.sh
```

该模式额外验证管理员登录和强制改密、HTTPS Manifest 预检、Web 创建任务、Bearer + HMAC、Unix Socket、状态轮询以及 `deployment_history.source_job_id` 幂等回写。成功标记为：

```text
HOST_UPDATER_WEB_REAL_DOCKER_DRILL_OK unix_socket=1 hmac=1 web_job=1 deployment_sync=1 pinned_digest=1 postgres_backup_restore=1 s3_backup_restore=1 migration=1 candidate_ready=1 active_ready=1 historical_s3_image=1
```

同一脚本还支持四类真实失败场景：

| `DRILL_SCENARIO` | 目标镜像 | 预期失败点 |
|---|---|---|
| `digest_failure` | Manifest 使用不存在的 Digest | `docker pull image@digest` |
| `migration_failure` | 故障镜像 | 目标镜像 `migrate` |
| `candidate_failure` | 故障镜像 | 候选容器 Ready |
| `active_failure` | 故障镜像 | 正式应用 Ready |

后三种场景使用仓库内的 [`failure-image.Dockerfile`](../host-updater/tests/fixtures/failure-image.Dockerfile) 构建；它只替换测试入口，内部仍调用正式 `ai-image-studio` 二进制。示例：

```bash
docker build \
  --file host-updater/tests/fixtures/failure-image.Dockerfile \
  --build-arg BASE_IMAGE=ai-image-studio:real-drill \
  --tag localhost:5055/ai-image-studio-failure:v0.2.0 .

# 推送后使用 Registry 返回的 Digest 执行；CURRENT_* 指向可正常运行的旧镜像。
DRILL_SCENARIO=migration_failure \
TARGET_IMAGE_TAG=localhost:5055/ai-image-studio-failure:v0.2.0 \
TARGET_IMAGE_DIGEST=sha256:... \
CURRENT_IMAGE_TAG=localhost:5055/ai-image-studio:v0.1.0 \
CURRENT_IMAGE_DIGEST=sha256:... \
bash host-updater/tests/executor_real_docker_drill.sh
```

失败场景只有在任务被拒绝、进入 recovery、Schema 未变化、旧版本重新 Ready、数据库与 S3 备份实际恢复且历史图片仍与原图一致时才输出：

```text
HOST_UPDATER_REAL_DOCKER_FAILURE_OK scenario=migration_failure rejected=1 schema_unchanged=1 old_release_ready=1 postgres_backup_restore=1 s3_backup_restore=1 historical_s3_image=1
```

隔离环境已经覆盖执行器成功链、四类失败恢复，以及 Web 管理 API 到 Updater 服务的成功整链；它们仍不替代生产数据副本、真实发布 Registry、Updater 进程中断和不同版本间真实回滚验收。

## 9. 隔离环境故障演练

提交前还要运行不接触真实 Docker/数据库的固定执行器状态机演练：

```bash
bash host-updater/tests/executor_success_drill.sh
bash host-updater/tests/executor_failure_drill.sh
```

成功演练使用隔离临时目录和固定假命令，验证 Migration 成功、候选 Ready、正式切换、`release.env`、`history.json`、备份保留以及不兼容回滚阻断。失败演练在 Migration 阶段注入失败，验证数据库、Local 图片和旧版本恢复。两者都是可重复的控制流证据，不能替代真实 Registry Digest 和真实备份恢复演练。

不要直接在生产环境首次验证恢复能力。复制 Compose 项目、数据库和图片目录到隔离宿主机或独立 VM，使用独立端口和测试凭据完成以下演练：

1. Digest 不匹配：修改测试 Manifest 的 `image_digest`，确认任务失败且 Migration 未运行。
2. Migration 失败：使用包含故意失败 Migration 的测试镜像，确认 `database.dump` 被恢复，旧应用重新 Ready。
3. 候选失败：使用 `/api/v1/ready` 返回 `not_ready` 的测试镜像，确认候选被删除，数据库/图片和旧应用恢复。
4. 正式切换失败：让测试镜像只在候选端口可用、正式启动失败，确认 `release.env` 回到旧 Digest。
5. Updater 重启：任务运行时终止 Updater，重启后确认该任务标记为 `failed/updater_restarted`，不会伪报成功。
6. 回滚窗口：连续部署四个测试版本，确认只允许回滚到最近三个保留版本，且 Schema 必须位于目标版本声明的兼容窗口内。

每次演练至少核对：

- 管理页面任务终态和错误步骤
- `/var/lib/ai-image-studio-updater/history.json`
- 对应 `backup-manifest.json`、数据库 SHA-256 和图片清单
- `deployment_history.source_job_id`
- 正式 `/api/v1/ready`
- 随机历史图片能否读取

只有备份和恢复都实际演练通过，才应在生产环境启用“立即升级”和“回滚”。

# Host Updater 部署与故障演练

Host Updater 是独立运行在宿主机上的最小控制服务。Web 应用只负责管理员鉴权、二次输密、Manifest 预检和任务展示；Docker、备份目录、Cosign 公钥及发布切换权限只交给 Host Updater。

## 1. 已实现边界

- 仅允许监听 `127.0.0.1` 或 `::1`。
- `POST /v1/jobs` 和 `GET /v1/jobs/{id}` 同时校验 Bearer Token、时间戳和 HMAC-SHA256，请求时间偏差最多 60 秒。
- Job ID 幂等；同一时间只允许一个升级或回滚任务。
- Job 状态写入宿主机独立目录，服务重启后不会把中断任务误报为成功。
- 只调用 `HOST_UPDATER_EXECUTOR_PATH` 指定的绝对路径执行器，不接收请求方提供的命令、脚本路径或环境变量。
- 可配置执行器 SHA-256，防止已安装脚本被静默替换。
- 固定执行器执行数据库和图片备份、S3 备份证据校验、镜像 Digest/Cosign 校验、SQLx Migration、候选容器健康检查、正式切换、失败恢复及历史版本保留。
- 成功结果包含版本、镜像、Digest、Schema 和备份清单；Web 同步后写入 `deployment_history`。

## 2. 宿主机依赖

生产宿主机需要：

- Docker Engine 和 Docker Compose v2
- `curl`、`jq`、`tar`、`sha256sum`
- `cosign`
- 运行中的 AI Image Studio Compose 项目

Host Updater 自身不放进 Web 容器，也不通过 TCP 暴露到公网。

## 3. 构建和安装

```bash
cargo build --release --package ai-image-studio-host-updater

sudo install -m 0755 \
  target/release/ai-image-studio-host-updater \
  /usr/local/bin/ai-image-studio-host-updater

sudo install -d -m 0755 /usr/local/libexec/ai-image-studio
sudo install -m 0755 \
  host-updater/scripts/execute-update.sh \
  /usr/local/libexec/ai-image-studio/execute-update.sh

sudo install -d -m 0700 /etc/ai-image-studio-updater
sudo install -m 0600 \
  host-updater/config/host-updater.env.example \
  /etc/ai-image-studio-updater/host-updater.env
sudo install -m 0600 \
  host-updater/config/executor.env.example \
  /etc/ai-image-studio-updater/executor.env

sudo install -m 0644 \
  host-updater/systemd/ai-image-studio-host-updater.service \
  /etc/systemd/system/ai-image-studio-host-updater.service
```

生成至少 32 字节随机 Token：

```bash
openssl rand -hex 32
```

同一个值分别配置到：

- Web 应用 `.env` 的 `HOST_UPDATER_TOKEN`
- `/etc/ai-image-studio-updater/host-updater.env` 的 `HOST_UPDATER_TOKEN`

Linux Compose 部署推荐专用 Unix Socket。Web 应用 `.env` 配置为：

```env
HOST_UPDATER_URL=
HOST_UPDATER_SOCKET=/run/ai-image-studio-updater/host-updater.sock
HOST_UPDATER_SOCKET_DIR=/run/ai-image-studio-updater
UPDATE_MANIFEST_URL=https://releases.example.internal/release-manifest.json
# 私有 CA 才需要；必须是容器内可读的绝对 PEM 路径。
HTTP_CA_CERT_FILE=/app/config/internal-release-ca.crt
```

`HOST_UPDATER_SOCKET_DIR` 只把该专用 API Socket 只读挂载到 Web 容器，不是 Docker Socket。Host Updater 的 `HOST_UPDATER_SOCKET_GID=10001` 与镜像内 `USER 10001:10001` 对齐。

`HTTP_CA_CERT_FILE` 会把私有根 CA 加入 Web 应用的 HTTP 客户端信任集合，不会关闭证书链或主机名校验。应挂载 CA 证书而不是把服务器叶子证书同时当作 CA。宿主机执行器访问同一内部 Manifest 时，还要把该 CA 安装到宿主机系统信任库，或在 Host Updater 服务环境中设置 `CURL_CA_BUNDLE=/absolute/path/to/ca.crt`。

Windows 本机开发且 Web 不在 Linux 容器内时，可不配置 Socket，改用 `HOST_UPDATER_URL=http://127.0.0.1:3199/`。不得把 Updater TCP 端口开放到公网。

固定执行器内容后写入哈希：

```bash
sha256sum /usr/local/libexec/ai-image-studio/execute-update.sh
```

把第一列填入 `HOST_UPDATER_EXECUTOR_SHA256`。

## 4. Release Manifest

每个发布版本必须提供 HTTPS Manifest：

```json
{
  "version": "0.2.0",
  "image": "ghcr.io/codechicken/ai-image-studio:v0.2.0",
  "image_digest": "sha256:...",
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

正式镜像必须用与 `COSIGN_PUBLIC_KEY` 匹配的密钥签名。执行器按 `image@sha256:...` 拉取并执行 `cosign verify`，不会仅信任可变 Tag。

## 5. Local 与 S3 备份

Local 模式在停写后生成：

- `database.dump`
- `images.tar.gz`
- `images-manifest.json`，逐文件记录相对 Key、大小和 SHA-256
- `backup-manifest.json`

S3 是主存储，不等于已经备份。S3 模式要求独立备份任务在 `S3_BACKUP_REFERENCE_FILE` 中写入最近一次不可变快照、对象版本或跨 Bucket 备份引用。执行器会检查文件存在、非空且未超过 `S3_BACKUP_MAX_AGE_SECONDS`，否则拒绝升级。

## 6. 启动

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now ai-image-studio-host-updater
curl -i http://127.0.0.1:3199/health
sudo journalctl -u ai-image-studio-host-updater -f
```

健康接口只证明 Updater 进程存在；升级是否可执行还取决于执行器配置、磁盘、数据库、备份证据、Manifest 和签名检查。

## 7. 执行顺序

1. 校验配置、数据库健康、Schema 兼容窗口和磁盘空间。
2. 停止 `app` 与 `worker`，保持 PostgreSQL/Redis 运行，形成一致停写点。
3. 备份数据库，并备份 Local 图片或验证 S3 独立备份证据。
4. 按 Digest 拉取镜像并执行 Cosign 验证。
5. 升级时运行目标镜像内的 SQLx Migrator，并核对最终 Schema。
6. 在同一 Compose 网络启动临时候选容器，检查 `/api/v1/ready` 的数据库和 Redis 状态。
7. 候选成功后写入 `release.env`，切换正式 `app/worker` 并再次健康检查。
8. 写入宿主机 `history.json`、Release 记录和数据库 `deployment_history`。
9. 只保留当前版本及之前三个版本的宿主机记录和备份。

任何错误都会停止候选/新应用、恢复上一份 `release.env`；Migration 已开始时还会恢复同批次数据库备份和 Local 图片备份，再启动旧应用并执行健康检查。

## 8. 真实 Docker 隔离成功链

[`executor_real_docker_drill.sh`](../host-updater/tests/executor_real_docker_drill.sh) 不替换 `docker`、`cosign`、PostgreSQL 或对象存储命令。它会启动隔离的 PostgreSQL、Redis 和 MinIO，运行正式 `execute-update.sh`，并使用调用方提前放入隔离 Registry 的真实签名镜像。

先构建目标镜像，将其推送到隔离 Registry，并用测试私钥签名；传给脚本的 Digest 必须是 Registry 返回的 Manifest Digest，公钥必须与签名私钥匹配。然后运行：

```bash
SIGNED_IMAGE_TAG=localhost:5055/ai-image-studio:v0.2.0 \
SIGNED_IMAGE_DIGEST=sha256:... \
COSIGN_BIN=/usr/local/bin/cosign \
COSIGN_PUBLIC_KEY=/tmp/ai-image-studio-drill/cosign.pub \
bash host-updater/tests/executor_real_docker_drill.sh
```

如需验证两个不同应用镜像之间的升级，可额外传入 `CURRENT_IMAGE_TAG`、`CURRENT_IMAGE_DIGEST` 和 `CURRENT_SCHEMA_VERSION`；未传时使用目标镜像作为初始运行镜像，只验证执行器真实成功链。`TARGET_IMAGE_TAG/TARGET_IMAGE_DIGEST` 是 `SIGNED_IMAGE_TAG/SIGNED_IMAGE_DIGEST` 的等价新名称，便于失败场景传入未签名目标。

脚本会自动完成并断言：

1. 从本地 HTTPS Release Manifest 读取固定 Digest。
2. 停止隔离应用，生成真实 PostgreSQL Custom Dump。
3. 把上传到主 MinIO Bucket 的图片镜像到独立备份 Bucket，并写入新鲜 S3 备份引用。
4. 按 Digest 拉取镜像并执行真实 `cosign verify`。
5. 使用目标镜像运行 SQLx Migration，启动候选容器并检查 Ready。
6. 切换正式 `app/worker`，核对 `release.env`、`history.json` 和 `backup-manifest.json`。
7. 把数据库 Dump 恢复到新数据库，确认上传资产记录存在。
8. 分别从备份 Bucket 和升级后应用读取历史图片，与原图做 SHA-256 比对。
9. 删除本次演练创建的隔离容器、网络、Volume 和临时目录。

成功标记如下：

```text
HOST_UPDATER_REAL_DOCKER_DRILL_OK signed_digest=1 https_manifest=1 postgres_backup_restore=1 s3_backup_restore=1 migration=1 candidate_ready=1 active_ready=1 historical_s3_image=1
```

还可以用同一套真实依赖验证 Web 控制面整链。`HOST_UPDATER_BIN` 必须指向当前代码构建出的 Linux 可执行文件：

```bash
DRILL_TRIGGER=web \
TARGET_IMAGE_TAG=localhost:5055/ai-image-studio:v0.2.0 \
TARGET_IMAGE_DIGEST=sha256:... \
COSIGN_BIN=/usr/local/bin/cosign \
COSIGN_PUBLIC_KEY=/tmp/ai-image-studio-drill/cosign.pub \
HOST_UPDATER_BIN=/usr/local/bin/ai-image-studio-host-updater \
bash host-updater/tests/executor_real_docker_drill.sh
```

该模式额外验证管理员登录和强制改密、HTTPS Manifest 预检、Web 创建任务、Bearer + HMAC、Unix Socket、状态轮询以及 `deployment_history.source_job_id` 幂等回写。成功标记为：

```text
HOST_UPDATER_WEB_REAL_DOCKER_DRILL_OK unix_socket=1 hmac=1 web_job=1 deployment_sync=1 signed_digest=1 postgres_backup_restore=1 s3_backup_restore=1 migration=1 candidate_ready=1 active_ready=1 historical_s3_image=1
```

同一脚本还支持五类真实失败场景：

| `DRILL_SCENARIO` | 目标镜像 | 预期失败点 |
|---|---|---|
| `digest_failure` | Manifest 使用不存在的 Digest | `docker pull image@digest` |
| `signature_failure` | 已推送但未签名的独立 Repository | `cosign verify` |
| `migration_failure` | 已签名的故障镜像 | 目标镜像 `migrate` |
| `candidate_failure` | 已签名的故障镜像 | 候选容器 Ready |
| `active_failure` | 已签名的故障镜像 | 正式应用 Ready |

后三种场景使用仓库内的 [`failure-image.Dockerfile`](../host-updater/tests/fixtures/failure-image.Dockerfile) 构建；它只替换测试入口，内部仍调用正式 `ai-image-studio` 二进制。示例：

```bash
docker build \
  --file host-updater/tests/fixtures/failure-image.Dockerfile \
  --build-arg BASE_IMAGE=ai-image-studio:real-drill \
  --tag localhost:5055/ai-image-studio-failure:v0.2.0 .

# 推送并用同一测试私钥签名后执行；CURRENT_* 指向可正常运行的旧镜像。
DRILL_SCENARIO=migration_failure \
TARGET_IMAGE_TAG=localhost:5055/ai-image-studio-failure:v0.2.0 \
TARGET_IMAGE_DIGEST=sha256:... \
CURRENT_IMAGE_TAG=localhost:5055/ai-image-studio:v0.1.0 \
CURRENT_IMAGE_DIGEST=sha256:... \
COSIGN_BIN=/usr/local/bin/cosign \
COSIGN_PUBLIC_KEY=/tmp/ai-image-studio-drill/cosign.pub \
bash host-updater/tests/executor_real_docker_drill.sh
```

失败场景只有在任务被拒绝、进入 recovery、Schema 未变化、旧版本重新 Ready、数据库与 S3 备份实际恢复且历史图片仍与原图一致时才输出：

```text
HOST_UPDATER_REAL_DOCKER_FAILURE_OK scenario=migration_failure rejected=1 schema_unchanged=1 old_release_ready=1 postgres_backup_restore=1 s3_backup_restore=1 historical_s3_image=1
```

隔离环境已经覆盖执行器成功链、五类失败恢复，以及 Web 管理 API 到 Updater 服务的成功整链；它们仍不替代生产数据副本、真实发布 Registry、Updater 进程中断和不同版本间真实回滚验收。

## 9. 隔离环境故障演练

提交前还要运行不接触真实 Docker/数据库的固定执行器状态机演练：

```bash
bash host-updater/tests/executor_success_drill.sh
bash host-updater/tests/executor_failure_drill.sh
```

成功演练使用隔离临时目录和固定假命令，验证 Migration 成功、候选 Ready、正式切换、`release.env`、`history.json`、备份保留以及不兼容回滚阻断。失败演练在 Migration 阶段注入失败，验证数据库、Local 图片和旧版本恢复。两者都是可重复的控制流证据，不能替代真实签名镜像和真实备份恢复演练。

不要直接在生产环境首次验证恢复能力。复制 Compose 项目、数据库和图片目录到隔离宿主机或独立 VM，使用独立端口和测试凭据完成以下演练：

1. Digest 不匹配：修改测试 Manifest 的 `image_digest`，确认任务失败且 Migration 未运行。
2. 签名失败：使用未签名镜像，确认 Cosign 阶段拒绝继续。
3. Migration 失败：使用包含故意失败 Migration 的测试镜像，确认 `database.dump` 被恢复，旧应用重新 Ready。
4. 候选失败：使用 `/api/v1/ready` 返回 `not_ready` 的测试镜像，确认候选被删除，数据库/图片和旧应用恢复。
5. 正式切换失败：让测试镜像只在候选端口可用、正式启动失败，确认 `release.env` 回到旧 Digest。
6. Updater 重启：任务运行时终止 Updater，重启后确认该任务标记为 `failed/updater_restarted`，不会伪报成功。
7. 回滚窗口：连续部署四个测试版本，确认只允许回滚到最近三个保留版本，且 Schema 必须位于目标版本声明的兼容窗口内。

每次演练至少核对：

- 管理页面任务终态和错误步骤
- `/var/lib/ai-image-studio-updater/history.json`
- 对应 `backup-manifest.json`、数据库 SHA-256 和图片清单
- `deployment_history.source_job_id`
- 正式 `/api/v1/ready`
- 随机历史图片能否读取

只有备份和恢复都实际演练通过，才应在生产环境启用“立即升级”和“回滚”。

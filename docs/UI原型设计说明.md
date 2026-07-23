# AI Image Studio UI 原型设计说明

可交互原型：[打开 `ui-prototype.html`](./ui-prototype.html)

## 1. 原型目标

原型用于确定前端信息架构、核心页面布局和多轮生图交互，不作为最终视觉稿。前端实现时应优先保持以下产品语义：

- 创作的第一层对象是“会话”，不是孤立的生图表单。
- 用户在当前会话分支继续追问时，系统自动关联相关历史文本和必要图片；无需手工选择历史图片。
- 生图默认展示 SSE 流式阶段，图片持久化完成后才显示为可下载结果。
- Provider、模型能力和参数由后端动态返回。
- OpenAI Compatible 与 Grok 通过各自模型列表接口刷新候选模型，Gemini 使用 Gemini Models API；候选发现与参数能力判定是两个独立步骤。
- Provider 配置归当前登录用户所有，管理员的用户管理页也不得读取其他用户的 API Key。
- Local/S3 是管理员可检查、可测试的存储配置，不向浏览器暴露 S3 Secret。
- 管理员页面必须同时受后端 RBAC 保护，不能只依赖前端隐藏菜单。
- Light/Dark 主题由用户偏好控制，正式实现同时支持跟随系统。

## 2. 信息架构

```text
AI Image Studio
├── 创作台 /studio
│   ├── 会话列表
│   ├── 多轮消息时间线
│   ├── 图片结果与自动上下文关联
│   ├── 消息输入区
│   └── 生成参数面板
├── 历史作品 /history
├── 用量与成本 /usage
├── 我的 Provider /providers
├── 用户管理 /admin/users               [仅管理员]
├── 运营监控 /admin/operations           [仅管理员]
├── 存储与系统设置 /settings/storage     [仅管理员]
    ├── Local/S3 主写入驱动
    ├── 混合存储统计与健康检查
    └── 待迁移 Asset
└── 版本与升级 /settings/updates         [仅管理员]
```

## 3. 桌面端布局

```text
┌──────┬──────────────────┬─────────────────────────────────────┬──────────────────┐
│ 主导航 │ 会话列表           │ 多轮消息与图片结果                       │ 生成参数           │
│ 72px │ 286px            │ 自适应                                │ 320-760px 可拖动   │
│      │ + 新建会话         │ 顶部：会话名 / 当前轮次                   │ Provider / 模型    │
│      │ + 搜索            │ 中部：用户消息 / 助手消息 / 图片          │ 基础 / 高级参数      │
│      │ + 最近会话         │ 底部：Prompt / 上传参考图 / 发送           │ 可横向拖动调整宽度    │
└──────┴──────────────────┴─────────────────────────────────────┴──────────────────┘
```

会话列表和生成参数面板只在创作台显示；切换到历史作品、用量、Provider 或管理员页面时，两侧创作区域都隐藏，业务页面使用释放后的内容宽度。桌面端参数面板默认宽度为 `340px`，可拖动左侧分隔条在 `320px` 到当前窗口允许的最大宽度之间调整；向左拖动放大、向右拖动缩小。面板按自身宽度而非整个窗口宽度切换布局：默认单列，中等宽度两列，较宽时基础参数四列、高级参数三列。屏幕宽度小于 `1220px` 时隐藏右侧参数面板，参数从抽屉打开；小于 `860px` 时隐藏会话栏，从顶部按钮打开；小于 `560px` 时使用单栏移动布局。

## 4. 核心页面

### 4.1 创作台

主要组件：

- `ConversationList`：搜索、创建和切换会话；单条会话只提供修改标题操作，并支持拖动排序，不在列表中提供删除、归档等更多操作。
- `MessageBubble`：展示用户/助手文本、失败状态和分支入口。
- `ImageResultCard`：预览、下载和查看生成信息。
- `StreamingStatus`：显示排队、Provider 处理、下载、校验、持久化等真实阶段。
- `MessageComposer`：输入本轮要求和上传本轮参考图；选择或粘贴图片后，在聊天框内部显示真实图片缩略图和独立移除按钮，不以文件名标签占用聊天框外空间；不提供历史图片选择器或固定上下文条数控件。
- `ParameterPanel`：根据 Model Capability 动态渲染参数。

参数面板分为基础参数与高级参数：

- 模型发现：模型选择框只显示当前 Provider 的模型，标注“已验证可生图/待分类”，界面入口统一命名为“刷新模型列表”；后端仍通过当前 Provider 的标准模型列表接口完成发现。
- 基础参数：Provider、模型、宽高比/尺寸、质量等级、生成数量和创作风格。
- 宽高比和分辨率都提供 `Auto`，默认选择 `Auto`，由模型或 Provider 根据 Prompt 与能力选择合适输出；用户明确选择后才发送固定值。
- 创作风格：电影感、摄影、插画等均是 Prompt 模板，位于基础创作区；选择后由 Prompt Builder 追加模板文本，不作为 Provider 参数提交。
- “管理模板”打开模板管理弹窗，支持选择、新建、修改名称与 Prompt 文本并保存；正式实现只操作当前用户的 `prompt_templates(template_type=style)`。点击“新建模板”必须清空草稿、突出当前选中状态，并在编辑区明确展示“新建模板”和创建说明，不能出现点击后无视觉反馈的状态。
- Prompt 辅助：自动增强属于平台 Prompt Builder 能力，位于风格模板旁，不伪装成上游 API 参数。
- 高级参数：输出格式、输出压缩、背景、内容审核级别和流式局部预览数量；具体控件仍按模型 Schema 增减。
- Seed、Inference Steps、Guidance Scale、Negative Prompt、Reference Strength 只在具体 Adapter 明确支持时出现；OpenAI GPT Image 未声明支持时直接隐藏。
- 高级参数的数据类型、范围、默认值、枚举值、是否必填和是否支持均来自模型 `parameter_schema`。
- 当前模型不支持的参数默认隐藏；未支持、禁用或未填写的字段不得发送给 Provider。

关键交互：

1. 用户发送消息后立即创建用户消息、助手占位消息和任务。
2. 收到 `task.created` 后，助手消息进入 `streaming`。
3. 收到任务 ID 后展示“取消生成”；点击后按钮进入“取消中”，调用 `/tasks/{id}/cancel`，并立即移除局部预览。取消接口必须同步把对应助手消息写为 `cancelled`，刷新会话后显示“生成已取消”，不得继续显示流式动画。
4. 收到 `task.failed` 后在原助手消息中显示持久化错误摘要和“重试”按钮；重试复用原任务 ID、Provider、模型、Prompt、参数和输入 Asset，不创建重复消息或会话分支。刷新页面后仍能从会话详情取得 `taskId` 和错误摘要继续重试。
5. 多文件上传中途失败或消息/任务创建校验失败时，页面保留本轮提示词与文件草稿，并补偿删除本轮新上传但未被采用的 Asset；如果服务端已经创建任务，重新加载会话展示其失败助手消息，不恢复成可重复提交的旧草稿。删除接口只接受当前用户未引用 Asset，已关联任务的图片不得清理。
6. `task.progress` 只展示后端提供的真实阶段；无精确百分比时展示不确定进度条。
7. 收到 `image.partial` 后在当前助手消息中用最新局部帧替换上一帧，并明确标记“最终原图仍在生成”；不得把它混入历史作品。
8. 收到 `image.completed` 后用已持久化 Asset 替换局部预览并加入助手消息。
9. 收到 `task.completed`、`task.failed` 或 `task.cancelled` 后停止动画；断线时显示“正在恢复”，并使用 `Last-Event-ID` 续传。手动重试从重试状态事件之后的新游标恢复，不回放上一轮失败终态。
10. Context Builder 根据当前分支、本轮指令和模型输入限制自动选择相关文本与必要的历史 Asset；图片作为独立多模态输入发送，不拼入 Prompt 文本，也不发送会话中的全部图片。
11. 会话标题通过行内输入框修改；拖动会话行后提交新的 `sort_order`，除此之外单条会话不展示其他管理操作。

顶栏不显示常驻“SSE 已连接”。SSE 连接只在存在生成任务时建立，连接、恢复和终态仅通过所属助手消息的任务状态展示；任务结束后连接关闭。

任务仍然是后端状态机、重试和 SSE 续传的必要资源，但首版不设置独立“任务”一级页面。当前任务阶段直接展示在所属创作会话中；已完成结果进入历史作品。

### 4.2 历史作品

- 默认按 `last_message_at` 和图片创建时间倒序。
- 支持按会话、Provider、模型、日期和尺寸筛选。
- 日期筛选使用 `createdFrom`（包含）和 `createdTo`（不包含）的半开区间；页面选择的结束日期转换为次日零点，完整包含结束日。尺寸筛选要求宽高同时填写并执行精确匹配。
- 会话、Provider 和模型选项只来自当前用户；选择 Provider 后模型列表只保留该 Provider 的模型。倒序日期、不完整尺寸和越界尺寸必须在页面或服务端返回明确校验错误。
- 图片卡片显示生成模型、所属会话和持久化状态。
- 首版历史卡片只展示已持久化结果，不提供直接复用、编辑或删除操作。

### 4.3 Provider

- Provider 卡片展示健康状态、模型数量和协议类型。
- Provider 及其模型属于当前登录用户，服务端查询必须带 `owner_id` 范围。
- 模型参数区域完全依赖 `capabilities` 和 `parameter_schema`，不在 Vue 组件中写死。
- “刷新模型列表”由后端调用当前协议的模型列表：OpenAI Compatible/Grok 使用对应 Models API，Gemini 使用 Gemini Models API。列表响应只能证明模型对当前凭据可见，不能单独证明它支持哪些图片参数。
- 已知模型能力来自版本化官方 Adapter 目录；Provider 扩展元数据与用户手工覆盖按明确规则合并。未知模型必须先分类或显式验证。
- “测试连接”使用后端管理接口执行，不从浏览器直接请求 Provider。
- Provider 卡片显示最近一次后端测试得到的“尚未测试/连接正常/连接异常”，不得把“已配置凭据”当作健康状态。
- 每个候选模型提供显式“验证”操作，执行前必须确认“会发起一次真实最小生图并可能产生费用”。
- 模型价格从 Provider 页进入配置，按单张图片和币种保存；价格只用于平台成本估算，历史用量保存价格快照。

### 4.4 用户管理

- 仅 `admin` 角色可访问，并由后端 RBAC 校验。
- 展示用户名、角色、状态、Provider 数量、图片占用和最后登录时间。
- 支持创建用户、禁用/启用、修改角色和重置密码，不展示任何 Provider Secret。
- 默认管理员通过配置首次 Bootstrap：`admin / 123456`；首次登录必须修改密码，配置变化不得覆盖已存在管理员。

### 4.5 存储与系统设置

- 页面仅管理员可见，普通用户请求管理接口时服务端返回 `403`。
- 显示当前 `STORAGE_DRIVER`、Local 路径、S3 非敏感配置和健康状态。
- `image_assets.storage_driver` 是每条图片的存储类型字段，允许 `local` 与 `s3` 数据同时存在。
- Local Asset 保存相对 `storage_key`，根目录来自配置；S3 Asset 保存 `storage_container`（Bucket）和 `storage_key`（Object Key）。数据库不保存本地绝对路径、外部临时链接或 S3 签名 URL。
- Local 与 S3 切换用于表达目标配置；正式实现保存前必须二次确认，并提示需要重启。
- “保存配置”调用 `PUT /admin/storage`，只写入非敏感目标配置并显示“重启后生效”；环境变量作为首次启动默认值，Secret 始终由环境变量或 Secret Manager 提供。
- S3 Secret 只允许由环境变量或 Secret Manager 注入。页面只能显示“已配置/未配置”，不能回显原值。
- “测试连接”必须执行临时对象的写入、读取、Head 和删除全流程。
- 从 Local 切到 S3 时展示待迁移 Asset 数量；旧 Local Asset 未迁移前提示不得卸载本地卷。
- 管理员可以立即执行数据库/Local/S3 一致性扫描，并查看最近扫描记录；定时任务默认每日执行。
- “扫描并清理”必须二次确认，只删除超过安全宽限期、数据库无记录且符合平台 Asset Key 格式的对象，未知文件不得自动删除。

### 4.6 用量与成本

- 所有已改密用户可访问，只展示自己的成功任务、图片数量和模型用量。
- 成本只根据 `model_pricing` 中当前有效价格计算；没有价格时显示“未配置”，不伪造为 0。
- USD、CNY 等不同币种分别汇总，禁止相加为无意义的单一总数。
- 最近用量使用 `beforeId` 游标分页，每页默认 50 条；加载更多只追加记录，不替换顶部汇总。用户删除历史作品后，用量和价格快照仍保留并计入统计。

### 4.7 运营监控

- 仅管理员可见，展示最近 30 天任务数、成功率、P50/P95/P99、重试、Provider 表现和 Local/S3 占用。
- 请求日志只展示 Trace ID、Provider、模型、状态和耗时，不展示 API Key、Prompt 或图片 Base64。
- 不新增独立“任务”一级页；活动任务状态仍在所属创作会话内展示。

### 4.8 版本与升级

- 仅管理员可见；展示当前应用版本、SQLx Schema 版本、Release Manifest、升级任务和最近三个可回滚版本。
- 升级/回滚需要管理员二次输密和专用请求头，Web 应用只把任务委托给可选 Host Updater。
- 未配置 Host Updater 时按钮禁用；Web 容器不得直接挂载 Docker Socket。

### 4.9 账户菜单

- 点击左下角头像打开账户菜单，而不是无响应的装饰头像。
- 菜单提供个人设置、管理员用户管理、Light/Dark 切换、修改密码和退出登录入口。
- 点击页面外部或按 `Esc` 关闭菜单；正式实现中的退出登录必须显示确认并调用后端注销接口。
- “个人设置”弹窗展示当前账号、显示名称和角色，并允许选择 `light`、`dark`、`system` 主题偏好；保存失败时保留弹窗并提示错误，不能只改变本地主题。
- 快速主题切换同样必须等待 `/users/me/preferences` 保存成功；退出确认取消时不得发送注销请求，确认后才调用 `/auth/logout` 并返回登录页。

## 5. 会话分支设计

用户在历史助手消息上点击“重新生成”或“从这里继续”时，新消息携带 `parent_message_id`：

```text
消息 1 ── 消息 2 ── 消息 3
                  └── 消息 3B ── 消息 4B
```

消息链采用“上一条助手消息 → 本轮用户消息 → 本轮助手消息”的交替父子关系。页面进入会话时选择 `sequence_no` 最新的叶子作为当前分支，只渲染从根节点到该叶子的祖先链，不把后端返回的全部消息平铺。

- “从这里继续”：把输入框锚定到选中的历史助手消息，显示可取消的锚点提示；下一次发送显式提交该助手消息 ID，不能退回服务端默认的全局最新消息。
- “重新生成”：定位助手消息对应的原用户消息，复用原文本和该用户消息已关联的输入 Asset，以原用户消息的父助手为 `parent_message_id` 创建同级分支；不得清空用户尚未发送的输入框草稿。
- 分支切换：同一父节点的同角色消息视为同级分支，消息旁显示“分支 N / M”和前后切换按钮；切换到某个同级节点后，默认展示其子树中 `sequence_no` 最新的叶子。
- 普通连续追问：即使用户正在浏览旧分支，前端也必须提交当前可见分支末端的助手消息 ID，确保 Context Builder 只读取所选分支。

切换、继续和重新生成都不复制图片文件；Asset 仍通过现有关系表复用，只有新的消息、任务和任务输入快照入库。正式前端实现位于 `StudioView.vue`，分支树计算位于 `conversationBranches.ts`。

## 6. 状态文案

| 后端阶段 | 页面文案 | 页面行为 |
|---|---|---|
| `queued` | 等待生成资源 | 不显示虚假百分比 |
| `provider.processing` | 模型正在生成 | 在所属助手消息中显示任务阶段和耗时 |
| `provider.downloading` | 正在接收生成结果 | 禁止提前显示上游 URL |
| `storage.validating` | 正在校验图片 | 展示格式与安全校验 |
| `storage.persisting` | 正在保存原图 | 成功前不可下载 |
| `completed` | 已完成 | 展示持久化图片卡片 |
| `failed` | 生成失败 | 展示错误摘要和重试按钮 |
| 客户端 `stream.reconnecting` | 正在恢复连接 | 携带最后一个持久化事件 ID 续传，只在当前任务中显示 |
| 客户端 `stream.polling` | 正在确认任务状态 | SSE 多次恢复失败后轮询任务，终态后立即关闭 |

## 7. 主题与视觉规范

- 原型顶栏提供 Light/Dark 切换；正式实现支持 `light`、`dark`、`system` 三种用户偏好。
- 主题偏好保存在 `users.theme_preference`；未登录时可用本地偏好，登录后以服务端用户偏好为准。
- Dark：背景 `#0B0C10`，主面板 `#12141A`，主文字 `#F5F2EB`。
- Light：背景 `#F4F3F8`，主面板 `#FFFFFF`，主文字 `#201A2B`。
- 品牌紫：`#A78BFA` / `#7C3AED`；成功与在线状态：`#5EEAD4`。
- 主圆角：`16px`；表单圆角：`11px`；卡片边框使用低对比白色透明线。
- 动效仅用于流式状态、页面切换和操作反馈；尊重 `prefers-reduced-motion` 的实现应在正式代码补充。

## 8. 前端实现映射

| 原型区域 | Vue 组件建议 | 数据来源 |
|---|---|---|
| 会话栏 | `ConversationList.vue` | `GET /conversations`、`PATCH /conversations/{id}`、`PUT /conversations/order` |
| 消息时间线 | `MessageBubble.vue` | `GET /conversations/{id}` |
| 输入区 | `MessageComposer.vue` | `POST /conversations/{id}/messages` |
| 流式状态 | `StreamingStatus.vue` | SSE task events |
| 图片卡片 | `ImageCard.vue` | `image.partial` 短期预览 / `image.completed` 正式 `content_url` |
| 参数面板 | `ParameterPanel.vue` | Model Capability |
| 模型发现 | `ModelDiscovery.vue` | `POST /providers/{id}/models/discover` |
| Provider 管理 | `ProviderManagement.vue` | `GET/POST/PATCH/DELETE /providers` |
| 风格模板 | `StyleTemplatePicker.vue` | `prompt_templates(template_type=style)` |
| 模板管理弹窗 | `StyleTemplateManager.vue` | `GET/POST/PATCH /prompt-templates` |
| 账户菜单与个人设置 | `DefaultLayout.vue` | `/users/me`、`/users/me/preferences`、`/users/me/change-password`、`/auth/logout` |
| 主题切换 | `ThemeToggle.vue` | `GET/PATCH /me/preferences` |
| 用户管理 | `UserManagement.vue` | `/admin/users` |
| 存储设置 | `StorageSettings.vue` | `/admin/storage` |
| 用量与成本 | `UsageView.vue` | `/usage` |
| 运营监控 | `OperationsView.vue` | `/admin/analytics`、`/admin/request-logs` |
| 版本与升级 | `UpdatesView.vue` | `/admin/updates/status`、`/admin/updates/jobs` |

## 9. 原型验收点

- 主导航可以切换创作台、历史作品、用量、Provider、用户管理、运营监控和设置，不设置重复的任务入口。
- 会话列表仅在创作台显示，可以切换会话、行内修改标题并拖动排序；单条会话没有删除或归档等其他操作。
- 生成面板展示基础与高级参数，且明确当前模型不支持的参数不会被提交；桌面端可以拖动左侧分隔条调整宽度，基础和高级控件按面板宽度自动切换单列、两列或多列布局。
- 模型框展示多个从当前 Provider 发现并验证的图片模型，“刷新模型列表”操作给出结果反馈。
- 风格模板位于高级设置之外，并明确其本质是 Prompt 模板。
- 宽高比和分辨率默认选中 Auto，并能切换为具体值。
- “管理模板”可以打开弹窗，完成模板选择、新建、编辑、保存和关闭反馈；“新建模板”有可见选中态、清空后的表单和明确的“创建模板”动作。
- 点击左下角头像可以打开账户菜单，个人设置弹窗、管理员页面跳转、修改密码和退出确认均有实际交互。
- Light/Dark 可以即时切换并持久化；个人设置还可以恢复为跟随系统。
- 用户管理与存储设置标记为管理员专属，正式实现中普通用户无法访问页面和接口。
- 设置页可以切换 Local/S3 表单、保存非敏感配置并触发测试连接反馈；保存结果明确提示需要重启。
- 设置页可以同时展示 Local/S3 Asset 数量，证明两类历史数据可混合存在。
- Prompt 发送按钮可以模拟 SSE 任务创建反馈。
- 输入区只保留本轮“参考图”上传；待发送图片在聊天框内以可移除缩略图展示，多张图片自动排列，发送成功后清空，任务创建失败时随文本草稿恢复。没有“引用历史图片”或“上下文 20 条”按钮，上下文由服务端按当前分支和模型预算自动构建。
- 创作会话搜索按标题即时过滤；存在搜索词时禁止拖动会话，清空搜索后恢复完整列表和原始排序。
- 桌面、平板和手机宽度下不存在横向内容溢出。

## 10. 参考项目与设计取舍

本原型只借鉴信息层级和交互模式，不复制参考项目代码或视觉资产。调研日期为 2026-07-21。

| 项目 | 借鉴点 | 本项目取舍 |
|---|---|---|
| [`lidge-jun/ima2-gen`](https://github.com/lidge-jun/ima2-gen/tree/main) | 大幅结果预览、历史缩略图、按 Provider/模型族分组的模型选择 | 保留会话时间线，在模型菜单中增加发现来源与验证状态；不引入首版节点画布 |
| [`alasano/gpt-image-playground`](https://github.com/alasano/gpt-image-playground) | Size、Quality、Background、Output Format、Moderation 等官方参数直接可见，结果和成本紧邻 | 基础参数常显，高级参数折叠；参数由 Schema 动态生成，不在组件里绑定单一 OpenAI 模型 |
| [`open-webui/open-webui`](https://github.com/open-webui/open-webui) | 会话优先的信息架构、清晰的模型切换和响应式侧栏 | 继续以多轮会话作为一级对象，Provider 管理与用户凭据保持后端隔离 |
| [`invoke-ai/InvokeAI`](https://github.com/invoke-ai/InvokeAI) | 高级能力分组、媒体画布优先、图库管理 | 借鉴折叠分组与大图优先；节点工作流留到后续版本，不增加首版复杂度 |
| [`lllyasviel/Fooocus`](https://github.com/lllyasviel/Fooocus) | 低学习成本、结果优先、Prompt 紧邻生成区 | 保持默认参数精简，把专业参数放入按能力出现的高级设置 |

最终原则：主视区优先展示会话与图片结果；右侧只常显高频参数；模型发现、能力 Schema、风格 Prompt 模板三者在数据和 UI 上保持独立。

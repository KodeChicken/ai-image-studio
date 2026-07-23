# ADR-0003：Provider Adapter 架构

- 状态：已接受
- 日期：2026-07-21

## 背景

OpenAI Compatible、Gemini 与 Grok 的鉴权、请求字段、模型发现和响应格式不同。

## 决策

业务层使用统一图片请求/结果模型，各 Provider Adapter 负责协议映射、白名单参数和响应校验。模型能力来自版本化官方目录或用户显式覆盖，不凭名称猜测未知模型。

## 备选方案

在任务服务中按 Provider 写条件分支更直接，但会快速形成难以测试的耦合逻辑。

## 影响

新增 Provider 必须实现 Contract Test；上游私有字段不得泄漏为平台通用契约。

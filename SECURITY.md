# 安全策略

## 支持范围

当前仍处于首个开发版本，仅最新主分支接受安全修复。正式发布后将在此补充受支持版本表。

## 报告漏洞

请优先使用 GitHub 仓库的 **Security → Report a vulnerability** 私密报告入口。不要在公开 Issue 中披露可利用细节、凭据、真实用户数据或未修复漏洞。

报告建议包含：

- 受影响版本或提交；
- 复现步骤和影响范围；
- 必要的最小日志或请求样例（先完成脱敏）；
- 已知缓解方式。

维护者确认后会协调修复、回归验证和披露时间。高危问题修复完成前，请勿公开完整利用方法。

## 敏感信息

Provider API Key、S3 凭据、Session Secret、Credential Master Key、Host Updater Token/HMAC Secret 均不得提交到仓库。若怀疑泄漏，应立即轮换对应凭据并使相关会话失效。

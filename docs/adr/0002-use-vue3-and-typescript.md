# ADR-0002：使用 Vue 3 与 TypeScript

- 状态：已接受
- 日期：2026-07-21

## 背景

创作台包含多区域状态、动态模型参数、SSE 任务事件和管理员页面，需要稳定的类型约束与组件生态。

## 决策

前端使用 Vue 3、TypeScript、Vite、Pinia、Vue Router 和 Naive UI。

## 备选方案

React 生态更大；纯 HTML 更轻，但无法低成本维护复杂交互和类型化 API。

## 影响

动态 Parameter Schema 可以映射为统一组件；构建与提交必须通过 TypeScript 和 ESLint。

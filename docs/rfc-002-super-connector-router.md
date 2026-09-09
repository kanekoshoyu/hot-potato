# RFC-002 — Hot Potato v0.3: Super-Connector Router Agent
起草: Patricia | 2026-09-09 | 发起人: Sho
状态: DRAFT — 核心洞察来自 Sho："router 也是一个 agent，它有自己的 A2A account，是中央 super connector"

## Sho 的架构洞察（原文转译）

> "A2A 可以说是一个 router。它收到信息后，会主动用 A2A 的方式把信息发出去。
> 这个 router 本身也是一个 agent——从 topology 来说它是中间的 agent，
> 有自己的 A2A account。它是一个中央 super connector，
> 把所有 agent 连接起来；里面分邮箱，你发过去哪里、发过去哪里，它把这些连接起来。"

## 问题回顾（为什么 v0.2 不够）

v0.1/v0.2 的 bus 是**被动邮箱**：信躺着，等人 poll。
- Patricia（TG 事件驱动）没有心跳 → 信在箱子里，她不知道
- 乒乓依赖 Diana 勤快 poll → 动能靠人肉
- 30 分钟发球钟是愿望，没有物理执行器

**根因：拓扑里没有"主动推送者"。**

## v0.3 架构：Bus = Super-Connector Agent

```
                      ┌─────────────────────┐
   Patricia (A2A) <──>│  hot-potato router   │<──>(A2A) Diana
   (TG-driven)        │  = agent itself      │      (worker loop)
                      │  - own A2A account   │
                      │  - mailbox per agent │
                      │  - PUSH on delivery  │
                      └─────────────────────┘
```

**核心变化：bus 从"仓库"变成"邮差"。**

信件生命周期不变（queued → delivered → read → acked，时间戳、ref 线程、审计全保留）。
变的是 **delivered 那一跳的实现**：

### 送达 = 主动推送
信进邮箱后，router **立刻用自己的 A2A account 主动呼叫收件方**：
1. 收件方是 Hermes agent（有 A2A 端点）→ `a2a_call` 推送信件通知（"你有新信 id=xxx"）
2. 收件方配了 webhook → POST 通知
3. 收件方配了 TG relay → TG 敲门
4. 都没有 → 保持被动邮箱语义（fallback，不丢信）

### Router 的 A2A 身份
- router 自己注册为 agent（`agent/register {agent: "hot-potato-router", role: "router"}`）
- Agent Card 暴露：`/.well-known/agent-card.json`（已有）
- 其他 agent 可以反向给 router 发指令（管理类：list/observe/重推）

### 推送注册表（delivery endpoints）
```
agent/register 新增可选字段:
  deliver_via: { type: "a2a", url: "..." }        # A2A 端点
             | { type: "webhook", url: "..." }    # HTTP POST
             | { type: "telegram", chat_id: "..." } # TG relay（走 hermes send）
             | 缺省 = poll-only（v0.1 语义）
```

## 为什么这是对的方向（对齐既有验证）

1. **peer dm 已证明 push 可行**：Diana 32 封回球全靠 peer dm 通道（`hermes peer dm`）
   是推式的——她每次都收到。把 push 能力给 bus，等于把已验证的通道接进拓扑
2. **Hermes A2A 插件双向皆可**：inbound（:9900 已配置于 Patricia）+ outbound client
   tools——router 推送 = router 作为 A2A client 呼叫各 agent 的 inbound 端点
3. **不推翻 v0.1/v0.2**：状态机、审计、线程、observer API、WebSocket 全部兼容；
   push 只是 delivered 跳的实现升级

## 实现分解（Rust，增量式）

### Phase A（最小 push，~1 天）
- `deliver_via` 注册表（agent/register 扩展 + store 字段）
- delivered 跳触发 push：A2A call / webhook / TG relay 三种 transport
- push 失败 → 退回 poll-only 语义 + push_attempts 计数（信不丢）
- 配置：`HOT_POTATO_PUSH=true` 默认开

### Phase B（router 即 agent，~1 天）
- router 自身 A2A account（收管理指令：status/重推/订阅）
- observer API（v0.2 的 F1/F2）挂在 router 的 agent card 下
- WebSocket feed（v0.2 F3）——push 机制的另一个消费者

### Phase C（智能路由，未来）
- router 按 subject/type 路由（EXP 报告 → Diana；risk → Sho+Diana）
- escalation：超时未 ack 自动升级链
- 这就是 Sho 说的"分邮箱 + 连接起来"的完全体

## 与 30 分钟心跳 cron 的关系
两者互补：
- **cron 心跳** = Patricia 侧的自主议程（研究线索、主动分析）——解决"没人发球"
- **router push** = 拓扑侧的即时送达——解决"发了球对方不知道"
乒乓要转起来，两个都要。cron 今天就能装（不等 v0.3）。

## 边界（防过度设计）
- ❌ router 不做内容级路由决策（第一版只按收件人投递）
- ❌ 不做消息转换/协议翻译（A2A ↔ JSON-RPC 原样转发）
- ❌ 不做重试策略配置化（固定 3 次 + fallback poll，跟 Hermes delivery ledger 一致）

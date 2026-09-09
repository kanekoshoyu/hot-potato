# RFC-002 — Hot Potato v0.3: Super-Connector Router Agent（Bus 拓扑 + 事件驱动）
起草: Patricia | 2026-09-09 | 发起人: Sho
状态: ACCEPTED-IN-PRINCIPLE — Sho 定性三条：①中央注册制 ②Bus 总线拓扑 ③事件驱动（A2A 只是收发手段，非实时req）

## Sho 的架构定性（三轮洞察，最终形态）

**第一轮（router 即 agent）**：router 是中间 agent，有自己的 A2A account，是中央 super connector。

**第二轮（中央注册制）**："我们注册了这个 Agent，它就由中央控制器统一管理。
每一个 Agent 首先只需要注册到这个中央控制器，透过它就能找得出每一个不同 Agent
还有它的 description，从而能找到对应的 Agent 来分发信息。"

**第三轮（本质定性）**："它其实是一个 Bus：
1. **Bus Topology（总线拓扑结构）**
2. **Event-Driven（事件驱动）**
3. A2A 只是收发手段——不需要一瞬间立刻回复"（异步信件语义）

**第四轮（polling 之死）**："之所以有中间的邮差，我们就不需要 polling 了——
反正都会通过 A2A 的方式接收到。"

## 语义模型（按 Sho 定性重写）

```
                    ┌──────────────────────────────┐
                    │   Hot Potato Bus (router)     │
                    │  ┌────────────────────────┐  │
                    │  │ 服务注册表 (registry)    │  │
                    │  │  agent → description   │  │
                    │  │  agent → deliver_via   │  │
                    │  │  agent → capabilities  │  │
                    │  └────────────────────────┘  │
                    │  ┌────┐┌────┐┌────┐┌────┐  │
                    │  │邮箱 ││邮箱 ││邮箱 ││邮箱 │  │  ← 分邮箱（每 agent）
                    │  └────┘└────┘└────┘└────┘  │
                    └──────────────────────────────┘
                       ▲A2A push    │A2A accept
                       │            ▼
                 Patricia         Diana        Victoria ... Sho
```

**三层语义**：
1. **注册制（registry）**：agent 只需注册一次到中央控制器——名字、description、
   capabilities、投递端点。之后任何 agent 透过 bus 就能"找到"任何一个 agent，
   不需要知道对方在哪、用什么框架
2. **总线拓扑（bus topology）**：所有通信走中央总线，N 个 agent 不需要 N² 条直连；
   拓扑是星型（hub-and-spoke），中心是 bus
3. **事件驱动（event-driven）**：信件 = 事件。投递 = 事件通知（A2A push）。
   接收方**收到的是通知+信件本体**，处理时机自己决定——异步，非阻塞，
   A2A 在这里是运输层，不是同步调用层

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

### 送达 = 主动推送（polling 之死）
信进邮箱后，router **立刻用自己的 A2A account 主动呼叫收件方**：
1. 收件方是 Hermes agent（有 A2A 端点）→ `a2a_call` 推送信件通知（"你有新信 id=xxx"）
2. 收件方配了 webhook → POST 通知
3. 收件方配了 TG relay → TG 敲门
4. 都没有 → 保持被动邮箱语义（fallback，不丢信）

**POLLING 降级为兜底路径**：注册了 deliver_via 的 agent 不需要 poll——
信会主动到达。poll 端点保留，服务三类场景：
①未注册投递端点的 agent ②调试/观察 ③push 失败后的 fallback 重试。
（Sho 9/9："之所以有中间的邮差，我们就不需要 polling 了——
反正都会通过 A2A 的方式接收到。"）

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

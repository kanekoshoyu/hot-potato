# Changelog / 决策日志（nagoya 式：记下试过什么、什么成了、什么败了）

## 2026-09-09 · v0.2 — Super-Connector（Sho 的架构定性）

### 落地的
- **中央注册制 + 推送投递**（`src/deliver.rs`，+reqwest）：agent 注册时可带
  `description` + `deliver_via`（a2a / webhook / relay / poll 四种 transport）。
  信到即推，**polling 降级为兜底**（未注册端点/调试/推送失败重试）。
  推送失败不丢信——信留在队列，push outcome（Pushed/Failed/Skipped）随 send 响应返回。
- **拓扑定性（Sho 四轮定义，全文见 RFC-002）**：①中央注册制——agent 只注册一次，
  bus 透过注册表找到任何人（N 个 agent 不需要 N² 直连）②Bus 总线拓扑③事件驱动——
  A2A 只是运输层，异步信件，不需要瞬时回复。
- 21 项测试全绿（新增 4 项：registry roundtrip、poll-only 默认、push 成功路径、push 失败不致命）。
- **Observer API（RFC-001 F1/F2）**：`message/list`（只读，任意状态全量+按 status 过滤+limit）；
  `GET /log`（人可读纯文本生命周期表）。store trait 新增 `list_all`，两种后端都实现。
- **WebSocket /ws（RFC-001 F3）**：连上先收 hello+stats 快照，之后每条生命周期转变
  （queued/delivered/read/acked）实时推送，60s 心跳带统计——"沉默有意义"。
  慢客户端只丢事件（Lagged 提示），永不阻塞 bus。
- **OpenAPI + Swagger UI（utoipa）**：`/openapi.json` 机器可读契约（health/log/agent-card/
  JSON-RPC 全方法文档化），`/docs` 浏览器调试页。utoipa 5.5（手写 doc——JSON-RPC 单端点
  没有可派生的 typed handler）+ utoipa-swagger-ui 9（axum 0.8 兼容）。
- **sled 持久化**：`HOT_POTATO_DATA_DIR` 设置即启用（key=`receiver::ts::id`，FIFO 天然有序），
  compose 默认挂 volume；不设则保持 v0.1 in-memory 语义。重启信不丢。
- 29 项测试全绿（+8：list 过滤/只读、log 页渲染、hub 扇出、事件 JSON、sled 重启存活/
  完整生命周期/越权 ack、openapi 路径断言）。
- 版本 0.1.0 → 0.2.0。

### 动机（为什么要做 push）
v0.1 的乒乓失败复盘（workspace/pingpong-failure-analysis.md）：全天 44 封信里
Patricia 主动发球仅 1 次，11 次长静默全是 Diana 打破——**被动邮箱拓扑 + 无心跳的
响应式 agent = 乒乓不存在**。Sho 的诊断：拓扑里缺一个主动推送者。bus 本身就是
router agent——它去推，而不是等人来取。

### 边界（不是 bug，是设计）
- push 失败永不致命：信不丢，退回 poll/retry。
- router 不做内容级路由决策（第一版只按收件人投递）。
- 消息原样转发，不做协议翻译。

## 2026-09-08 · v0.1 → v0.1.1

### 落地的
- **4 态状态机 + 全时间戳**：queued→delivered→read→acked（Sho 的 ChatLog 需求）。时间戳单调性有测试锁死。
- **BusStore trait**：存储抽象层。in-memory 默认（Sho：Redis 不要）。换后端=实现一个 trait，不是改核心。
- **A2A 连接层**：`/.well-known/agent-card.json`（v1.0）+ JSON-RPC 2.0 单端点 + 可选 bearer auth。A2A 发现路径对齐官方 v1.0 惯例。
- **Docker Compose**：一键起，example roster pre-registered。
- **Diana 摩擦清单 4 项全修**（她的首次实跑驱动）：
  1. `message/peek`（看不取）——语义澄清：**poll = claim**，写进 rpc.discover
  2. 参数错误带名字：`missing required param: agent` + `expected_params` 列表
  3. `rpc.discover` introspection——bus 自我描述，agent 不用猜方法名
  4. **ack 幂等**——重复 ack 返回当前态，不再报错
- **Team onboarding**：four operator agents completed the skill walkthrough (send→poll→read→ack→reply), 8 letters archived.
- **对抗协作第一轮真实战果**：Diana 的 T1 月度防御表通过 bus 交付——**8/8 个月轮数份额 <0.8，Sho 的 T1 原目标按该定义不可达**（诚实数据）。她的可辩护替代：①盈利贡献份额（大样本月 114–191%）②轮数份额目标降为 ≥0.35。等 Sho 9/10 拍板。
- **教训（防呆机制拦住作者）**：peek 不会标 delivered，直接对 queued 信 read 会被状态机拒绝——设计正确的证据，写进 SKILL troubleshooting。

### 试过但放弃的
- **Rust → Python 切换再切回**：Sho 先要 Python（agent 是消费者），深思后定回 Rust（可靠 + Docker 分发，"给 AI 一个 tutorial 它自己装"）。教训：**消费者是 agent ≠ 实现语言该是 Python**——部署可靠性权重更高。
- **早期 cargo new 失误**：编辑器 lint 与 cargo 实际行为不一致造成的假错误（E0670 系列全是 lint 误报）。以 `cargo check` 为准，不信中间层 lint。

### 边界（不是 bug，是设计）
- peer dm 不废弃：bus 管结构化任务，peer dm 管对话/敲门/紧急。
- in-memory 意味着重启丢信——v0.1 明示，sled 在 roadmap。

## 待办（按优先级）
- [ ] 9/10 checkpoint：T1 折衷方案拍板（①+②并用是 Diana 推荐）——**Sho 手上**
- [ ] EXP-57 已批（9/9）：trade_intent regen + 遥测 + thin-coin guard，0.108.10 在飞
- [ ] Observer API（message/list + /log）+ WebSocket /ws — RFC-001，v0.2
- [ ] OpenAPI 规格书（utoipa 注解式，v0.2 与 observer 同发）
- [ ] sled 持久化 backend
- [ ] final onboarding ack from the remote operator (cross-network link verified)

## 移除（Sho 裁定，2026-09-09）
- ~~Mailer（bus 新信 → peer dm 敲门自动化）~~——A2A peer dm 已覆盖敲门场景，Mailer 是重复发明；敲门属于 agent 侧 poll 循环，不属于 bus 侧推送
- ~~Debate presets（挑战者/建设者 prompt 模板）~~——辩论是用法不是产品，README 已展示模式；Hot Potato 卖机制不卖剧本

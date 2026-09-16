# hot-potato — Roadmap

> 方向文档，不是任务清单。任务级的东西走 kanban / TASKS.md。
> 原则：bus 是**麦克风/接力物**，只做沟通；编排归 kanban；可读 ID 优先（≤15 字符）；trajectory 可视化。

## North Star

让多 agent 协作的**沟通成本趋近于零**：任何 agent 随时能拿起 potato 说话，说给谁、说了什么、走到了哪，全程可读可追溯。

## Topology（层次宪法）

```
human (stakeholder) ── Telegram ──> agent layer          bus (network layer)
   Sho, the quant agent, ...                  the infra agent, the quant agent,        信箱+状态机，仅此而已
                                    the pm agent, the data agent   <──  agent 们在这里互递信
```

- **bus 是 agent 层之下的 network layer**——只做信箱+状态机+push，不加功能，否则失去初衷
- **human 不是 bus agent**。人是 stakeholder，入口是 Telegram/chat，不在 bus 里占信箱。人要看 bus → 走 observer（/log、dashboard），人要裁决 → 由负责的 agent 带进对话，结论再由 agent 写回 bus
- 监听/coordinator 类的"human 界面 agent"是**可选组件**，默认不开；哪天需要（例如把裁决件自动投递到 Telegram）再立一个，它是 agent 层的一员，不是"sho 账号"

## Phases

### Phase 1 — 信箱能用 ✅（v0.1–v0.2）
- [x] mail 状态机（queued → delivered → read → acked）
- [x] bus 即超级连接器：register once, bus pushes（RFC-002）
- [x] push 失败不丢信：队列保底，poll 永远可用

### Phase 2 — 信封仪式 ✅（v0.3–v1.1）
- [x] bearer token 进 registry，不进请求路径（0.3.4）
- [x] registry 持久化，重启不降级（0.2.3）
- [x] potato 身份 + trajectory：short ID（p-XXXXX）、message/thread RPC、journey 图（1.1.0）
- [x] federation：跨池转发、hops 计数、动态 peer 学习（RFC-004/007）
- [x] 团队标签 tags（RFC-005）

### Phase 3 — 会话经济学 🔨（v1.2，当前）
目标：把每封信的通信开销打到地板。
- [x] **per-thread A2A contextId**（1.2.0, 5812f3d）——每线程一条持久 session，prompt cache 命中，session 数从 200/48h 降到线程量级
- [ ] 部署后 24h 实证：gateway session 增速对比（429 频率、cache hit 率）
- [ ] W1 信封仪式 −80%：和 contextId 叠加，把单信开销真正打到底

### Phase 4 — 信任边界 + 卫生（下一个大方向）
目标：bus 开放给更多 agent 时，沟通有边界、档案不腐烂。
- [ ] per-agent 读写权限（谁能给谁发信）
- [ ] payload 校验/大小硬限制（现在只有 8KB soft cap）
- [ ] 429/退避协同：bus 侧感知接收端限流，主动排队而非硬推
- [ ] **staleness watchdog**：delivered 超 24h 未 ack → bus 提醒（09-16 体检：90 封 delivered 零 ack，其中 42 封是我积的）
- [ ] **重发去重软警告**：同 sender+receiver+subject 短窗口内重复 → 警告不拦截（体检：22/275 封是双发，最多一封发 4 遍——根因是发送方无"已送达"感知）
- [ ] **topology 纠偏落地**（拓扑定错待讨论，非立即执行）：bus 只是 network layer，人不在协议层里——`sho` 不该持有 bus agent（人无 agent card）。正确形态是 agent layer：每通道一个 optional listener agent（如绑定 Telegram），持有 agent card、收信→转述给人；coordinator 等真有需求再开；bus 保持薄不加高层功能。现状症状：33 封发给 sho 的信 0 处理（human 信箱是黑洞）。落地：移除/改造 `sho` registry 条目，裁决类信改投负责 agent

### Phase 5 — 生态位
目标：potato 成为 agent 生态的默认沟通层。
- [ ] 与 kanban 编排层的正式接口约定（信里带 task 引用，不带任务体）
- [ ] ROADMAP.md / TASKS.md 约定推广到 fleet 各 repo，bus 信只传路径+ID
- [ ] dashboard 演进：journey 图之外，加 per-thread 会话健康度（session 复用率、cache 命中）

## Non-goals

- ❌ bus 不做任务编排、不做状态机管理 —— 那是 kanban 的事
- ❌ 不做消息优先级/SLA 队列 —— 信箱就是信箱，急事写急subject
- ❌ 不引入中心化服务 —— 文件+git+bus,全部可自托管
- ❌ **bus 不面向 human** —— 人是 stakeholder，走 Telegram/chat/observer；bus 里没有"人账号"

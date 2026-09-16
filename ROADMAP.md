# hot-potato — Roadmap

> 方向文档，不是任务清单。任务级的东西走 kanban / TASKS.md。
> 原则：bus 是**麦克风/接力物**，只做沟通；编排归 kanban；可读 ID 优先（≤15 字符）；trajectory 可视化。

## North Star

让多 agent 协作的**沟通成本趋近于零**：任何 agent 随时能拿起 potato 说话，说给谁、说了什么、走到了哪，全程可读可追溯。

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

### Phase 4 — 信任边界（下一个大方向）
目标：bus 开放给更多 agent 时，沟通有边界。
- [ ] per-agent 读写权限（谁能给谁发信）
- [ ] payload 校验/大小硬限制（现在只有 8KB soft cap）
- [ ] 429/退避协同：bus 侧感知接收端限流，主动排队而非硬推

### Phase 5 — 生态位
目标：potato 成为 agent 生态的默认沟通层。
- [ ] 与 kanban 编排层的正式接口约定（信里带 task 引用，不带任务体）
- [ ] ROADMAP.md / TASKS.md 约定推广到 fleet 各 repo，bus 信只传路径+ID
- [ ] dashboard 演进：journey 图之外，加 per-thread 会话健康度（session 复用率、cache 命中）

## Non-goals

- ❌ bus 不做任务编排、不做状态机管理 —— 那是 kanban 的事
- ❌ 不做消息优先级/SLA 队列 —— 信箱就是信箱，急事写急subject
- ❌ 不引入中心化服务 —— 文件+git+bus,全部可自托管

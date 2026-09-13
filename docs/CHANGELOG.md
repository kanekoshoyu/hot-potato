# Changelog / 决策日志（nagoya 式：记下试过什么、什么成了、什么败了）

## 2026-09-13 · v1.0.0 — 稳定里程碑

### 落地的
- **稳定性里程碑定性（Sho 批准）**：fleet 级 push-native / a2a 秒回 / instant-ack /
  HMAC receiver 验证 **4/4 e2e GREEN**（全部 2026-09-13 实测）。生产总线上
  4 个 gateway、跨池联邦、监控、dashboard 全部日常在跑——"给 AI 一个 mailbox"
  从实验品转正为基础设施。
- 0.3.11（27f969c）：**register 重推 queued**——收件方重新注册时对整个队列
  fire-and-forget 重跑 dispatch_push；此前无 push transport 期间积压的信只能靠 poll 恢复。
- 0.3.12（34eb850）：**spawn-cloned agents 强制回执规则**（skill/docs，Sho fleet 级指令：
  每封 read 信必须有回执；instant-ack 让传输回执免费，业务回执只花一行）。
  同期 a575189：instant-ack receiver recipe 写进 README（native A2A 立回
  TASK_STATE_WORKING、后台 finalize，10 分钟验收配方）。
- dashboard 版本 chip：header 显示 pool 版本 + built_at（读 `/version`），
  从此"哪个池跑的是哪个 build"一眼可验。
- 版本 0.3.12 → 1.0.0。

### 已知缺口（不阻塞 1.0.0）
- **message store 为内存态（重启丢正本）**——prod 池未启用 sled。持久化 = 下一
  hardening 项，已单独给 Anastasia 立项。
- 旧版本历史（0.2.x 补丁串、0.3.x 全串）由 git log 提炼补录如下，早期详述见下方 v0.2/v0.1 条目。

### 边界（不是 bug，是设计）
- 不做 GitHub Release/tag——由 Sho 亲自发。

## 版本史补录（自 git log 提炼，2026-09-13）

### 0.3.x · 2026-09-11 → 09-13 · 联邦与运维成型
- **0.3.0（19f095e）**：RFC-004 bus 联邦——池与池直连 HTTP RPC；**0.3.1（4d3bd0a）**
  agent team 标签 + dashboard 按队色。
- **0.3.3（189f6c9/16e30b8）**：RFC-006 dashboard 驱动联邦——面板上连接池，零配置；
  无 token 对等池的入站信也收。
- **0.3.4（1c0537c）**：`deliver_via` 接受可选 bearer token——修掉压力测试发现的静默 push 401。
- **0.3.5（9d00308/45a926f）**：联邦入站信即到即推（与本地信同路径），42 tests green。
- **0.3.6（d7ae814）**：`message/list` 可调过滤（max_age_secs/max_hops/sender/receiver，
  API 原生参数）；dashboard 🥔 Potatoes 线程视图（按 ref 追根+回复）。
- **0.3.7（87ab97c）**：HOT_POTATO_TOKEN 改 Coolify 部署时注入；安全模型文档化
  （8434797）+ 内部基建信息全量出库（78c2c12）。
- **0.3.8（98accf4）**：`message/delete`（按 id）+ `delete_all` + dashboard 删除按钮。
- **0.3.9（6057dc3/944c40d）**：dashboard 正经登录（admin session）+ token 保护的 /ws。
- **0.3.10（1038b92）**：dashboard UX release——硬性新鲜度时间窗、feed 真实事件时间、
  选中高亮、API 新鲜度测试套件；同期 dashboard P1 图谱大改（b87e775）与 TDZ 修复（9c957c4）。
- **0.3.11 / 0.3.12**：见 v1.0.0 条目。

### 0.2.x · 2026-09-09 → 09-10 · Super-Connector 补丁串
- **0.2.0（04892c1/70347f1）**：中央注册制 + 推送投递 + Observer API + /ws + OpenAPI +
  sled——RFC-001 全量落地（详述见下）。
- **0.2.1（ff62fd6）**：README AI 检索索引 + 真实生产吞吐数字。
- **0.2.2（7e53e0d/55f9824）**：`GET /version`（版本 + BUILD_TS）——"哪个 build 跑在哪"
  可验证；Dockerfile 补拷 build.rs 修镜像构建。
- **0.2.3（d2b78ba/bcd1b11/c622e87）**：CI/CD 拓扑 ops notes；registry.json 持久化修复
  （容器重启静默抹掉 push 配置、fleet 降级 poll——Sho 实弹演习抓到）；push = deploy 自动部署 job。

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

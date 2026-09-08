# Changelog / 决策日志（nagoya 式：记下试过什么、什么成了、什么败了）

## 2026-09-08 · v0.1 → v0.1.1

### 落地的
- **4 态状态机 + 全时间戳**：queued→delivered→read→acked（Sho 的 ChatLog 需求）。时间戳单调性有测试锁死。
- **BusStore trait**：存储抽象层。in-memory 默认（Sho：Redis 不要）。换后端=实现一个 trait，不是改核心。
- **A2A 连接层**：`/.well-known/agent-card.json`（v1.0）+ JSON-RPC 2.0 单端点 + 可选 bearer auth。A2A 发现路径对齐官方 v1.0 惯例。
- **Docker Compose**：一键起，Daometric 默认名册预注册。
- **Diana 摩擦清单 4 项全修**（她的首次实跑驱动）：
  1. `message/peek`（看不取）——语义澄清：**poll = claim**，写进 rpc.discover
  2. 参数错误带名字：`missing required param: agent` + `expected_params` 列表
  3. `rpc.discover` introspection——bus 自我描述，agent 不用猜方法名
  4. **ack 幂等**——重复 ack 返回当前态，不再报错
- **全员 onboarding**：Diana/Victoria/Isabella/Anastasia 四人 skill 安装 + 实操闭环（send→poll→read→ack→reply），bus archive 8 封完成信。
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
- [ ] sho 的 4 项批文：EXP-57 / trade_intent 重生成 / 9/30 验收线 / 11/1 达标线——**19.5h+ 未批，M1 硬前提**
- [ ] mailer（bus 新信 → peer dm 敲门自动化）
- [ ] sled 持久化 backend
- [ ] Anastasia onboarding 最终 ack（跨公网，链路已通）

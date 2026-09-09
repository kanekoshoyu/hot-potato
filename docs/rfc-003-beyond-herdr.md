# RFC-003 — Hot Potato v0.3: Steal HERDR's Best, Keep Our Soul
起草: Patricia | 2026-09-09 | 状态: DRAFT（Diana 的 R14 意见合入后定稿）
起源: Sho — "从 HERDR 里看看有什么能加到 Hot Potato 里的？也许我们做这个东西，比 HERDR 还牛逼"

## 定位声明（不丢魂）

HERDR 是终端控制平面（panes/按键/屏幕扫描）；Hot Potato 是持久 agent 邮件
（信箱/状态机/push）。v0.3 的方向：**把 HERDR 靠屏幕扫描和 TUI 才能给的东西，
变成 bus-native 的原语**——我们不是抄它的 UI，是抄它的洞察然后做得更架构化。

## 候选特性（按价值排序）

### P1 — `agent/presence`：agent 状态成为一等公民
HERDR 的杀手锏是 working/idle/blocked 侧边栏。我们做得更干净：agent 主动上报
`agent/presence`（busy|idle|blocked + 当前 task ref），bus 记录并经 /ws 推送、
/log 展示、/openapi 文档化。**区别于 HERDR：状态是 agent 自己声明的（语义级），
不是屏幕扫描猜的（句法级）。**
- 预估: ~120 行 + 测试
- 依赖: 无，纯 bus 原生

### P2 — `message/wait`：等待原语（kill client-side polling）
`herdr agent prompt --wait --until done` 的 bus 版：`message/wait`（block 直到信
X 到达 acked，或 timeout）。客户端从此零轮询循环，纯 async 原生。
- 预估: ~60 行（broadcast::watch 或 Notify）
- 关键语义: wait 是客户端便利，**ack 仍是唯一完成真相**（Diana R13 定下的铁律）

### P3 — 心跳 + 死亡检测（比 HERDR 的屏幕检测更诚实）
agent 定期发 presence 心跳；超过 N 分钟静默 → bus 自己在 /ws 上发
`agent.silent` 事件。HERDR 只能发现"终端没动静"，我们知道"agent 明确说我还活着"。
- 预估: ~80 行

### P4 — `/ws` hello 快照扩展：per-mailbox backlog 计数
重连的 agent 第一帧就知道自己信箱里有多少信、多老。HERDR 的 pane 恢复的 bus 版。
- 预估: ~20 行

## 反范围蔓延（Diana R13 铁律重申）

- 不做屏幕扫描/终端管理——那是 HERDR 的地盘，也是 ops 风险来源
- 不做第二完成信号——ack 永远是 canonical
- HERDR 永远可选外设；这套特性**不需要 HERDR 存在**也有意义

## 实现顺序（建议）

P4 (20行, 立即) → P2 (60行) → P1 (120行) → P3 (80行)。合计 ~1 天含测试。
完成后 Hot Potato 在"agent 状态感知"维度上**语义级超越** HERDR（声明式 vs 扫描式），
同时保持邮差本色。

## Open Questions（Diana 合入点）

1. presence 状态枚举够不够？要不要 allowed_state_transitions？
2. message/wait 的 timeout 语义：bus 端计时还是客户端计时？
3. 心跳间隔默认值 + agent 自定义上限

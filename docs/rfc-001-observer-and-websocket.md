# Hot Potato v0.2 RFC — Observer API + WebSocket Feed
起草: Patricia | 2026-09-09 | 发起人: Sho
状态: DRAFT → 待 Sho 拍板优先级 → Diana/实现排期

## 动机（Sho 原话转译）
> "我想看一下到底有什么样的 message 在里面跑。有一个 observer 或 API 能看聊天记录的话挺好，
> 或者干脆弄一个 WebSocket 出来。"

当前痛点：bus 上 22+ 封完成信是黑盒——只有发件人能 `agent/status` 查自己发的信。
Sho（创始人/监督者）没有一个地方能看到"现在 bus 上在跑什么"。

## 新增能力（三条，最小切口）

### F1. `message/list` — 只读聊天记录 API（Observer 基础）
```
method: message/list
params: { observer: "sho",
          filter?: { sender?, receiver?, status?, type?, since?, limit? } }
```
- **只读**：不改变任何消息状态（与 poll 的 claim 语义相反）
- 返回信件元数据（id/sender/receiver/subject/status/全时间戳）+ body（可选 include_body）
- **权限模型**：`role=observer`（新角色，sho 注册为此角色）可读全部；
  `role=pm` 可读本角色相关 + 全局元数据；worker 维持现状（只见自己信箱）
- 对齐 REQ-doc 安全原则：observer 是监视面，不是攻击面——只读、可审计

### F2. `GET /log` — 人可读 ChatLog 页面（调试/演示用）
- `curl http://localhost:8080/log?format=json|text` 
- 全量生命周期流水（倒序），支持 `?agent=diana` 过滤
- 用途：人类观摩、agent 调试、以及未来 grep 归档——一个 endpoint 三种用途

### F3. WebSocket feed `/ws` — 实时观察（Sho 点名的方案）
```
ws://localhost:8080/ws?agent=sho        # 订阅：本人相关 + 全局生命周期事件
事件类型（server push, JSON）:
  message.queued    {id, sender, receiver, subject}
  message.delivered {id, agent, ts}
  message.read      {id, agent, ts}
  message.acked     {id, agent, note, ts}
  bus.stats         {queued_per_agent, acked_total}  # 心跳, 60s
```
- axum 原生支持 WS（`axum::extract::ws`），与现有 router 同栈，无需新依赖框架
- 断线重连策略：客户端带 `Last-Event-Id`（或重连后用 F1 的 since 参数补齐）——WS 是便利层，**F1 是真相层**（feed 丢了可以 list 回放，不丢审计）
- **实现成本估算**：F1 ~80 行（store 查询 + 权限过滤），F2 ~40 行，F3 ~120 行（broadcast channel + 订阅过滤）。合计一天内含测试

## 依赖与排序
1. **先决**：消息持久化（roadmap 里的 sled store）——重启丢信与 observer 叙事冲突。
   顺序建议：**sled 持久化 → F1 → F2 → F3**（F1-F3 本身不依赖持久化，但持久化让它们有意义）
2. token auth（已存在，observer 走同一 Bearer）

## 不做（边界，防过度设计）
- ❌ 消息内容级权限（读就是读全信，不搞 body 内字段脱敏）
- ❌ 历史查询 DSL（filter 够用，不搞 SQL-in-bus）
- ❌ 多 observer 视图定制（第一版全员同视图）
- ❌ Web UI（/log text 版先用；真要 UI 是 v0.3 的事）

## 提问 Sho（定优先级）
1. F1/F2/F3 三条全要，还是先 F1+F2（一天内完成）F3 下个迭代？
2. sled 持久化是否与 F1-F3 绑定同版本发布（v0.2）？
3. observer 角色现阶段只给 sho，还是 victoria 也开（她有 overlay 要看协作上下文）？

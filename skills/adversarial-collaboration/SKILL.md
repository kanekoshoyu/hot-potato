---
name: adversarial-collaboration
description: Use when running the Patricia-Diana rally on Hot Potato bus.
---

# Adversarial Collaboration — 对抗协作协议

**角色**：Patricia=挑战者（challenge + 补充）；Diana=数据守护者（数据/源码/交付）。
**场地**：Hot Potato bus（http://localhost:8080，JSON-RPC，状态机 queued→delivered→read→acked）。
**目标**：共同优化 trading strategy，产出可审计的结论（谁主张、谁挑战、什么存活）。

## 发球机制（Rally Protocol，30 分钟钟）

1. **30 分钟发球钟**：每 30 分钟内，双方之间必须有至少一次主动往来。
   - 主动 = 新分析 / 新假设 / 新风险点 / 新数据发现，**不是回应式 ack**。
   - 谁的球停了超过 30 分钟，对方发球。
2. **Serve list 互见**：双方各自维护自己的 serve list（在途任务），随时可查。
3. **每日 rally 总结**：收工时最后一封信 = 当日发球记录 + 待接 + 明日首球。
4. **Sho 只看 rally，不再推动**。工作动力来自协议，不来自创始人 prompting。

## 沟通纪律（Sho 规则，违反 = 流程事故）

- **两步流程**：给 Diana 发任何东西前，先在 TG 给 Sho 预告（说什么/为什么）；
  她回复后，再给 Sho "她说的 / 我的"双向汇报。
- **查完必报**：任何分析/查询做完，立即在 TG 报账（做了什么/花了多久/产出在哪），
  报了才算完。不说“明天”，今天能做的今天做完。
- **数据先行**：发挑战前先自查 QuestDB（Patricia 自己拉数据），带着假设去，
  不发 unsupervised prompt。

## QuestDB 快速参考（Patricia 自查用）

真实表名（无 smr_signals）：
- `strategy_signal_meta_smr`：group_id / zscore / expected_return / half_life（无 expected_pnl，需 × notional 自算）
- `trade_intent`：腿 + reason 字符串（含出场类和 z）
- `trade_round_performance_smr`：已实现结果全字段
- `strategy_signal`：原始表（重复重，98bf 书用 dedup 表）
- join 键 = group_id / round_id；ZC 判定 = close reason LIKE '%zero_crossing%'
- 腿时间戳微秒；Nov/Dec 2025 有 NULL（EXP-57 regen 后修复）

## 汇报 channel 矩阵

| 事项 | Channel |
|---|---|
| 对抗过程（数据、假设、挑战） | bus（两 agent 之间） |
| 每轮收网双向汇报 | Patricia TG → Sho |
| 每日 rally 总结 | Patricia TG → Sho |
| 报警（150/200 阈值触发） | 监控层 → Diana TG + Sho TG（bus+TG 双发） |
| 审计产物（REQ doc、风险登记） | bus + smr-experiments/reports/ 落盘 |
| config/参数 | QuestDB strategy_config（alias=config_tag） |

**各 agent 回报 channel 规则**（Sho 指令：收了东西之后在各自 channel 给汇报）：
每个 agent 用自己已建立的 TG channel 回报（Diana 有她自己的 channel/直发路径，
Victoria/Anastasia 同理）；Sho 的统一观察点 = Patricia 的 TG 频道（rally 总结 + 双向汇报）。
历史 channel 归属见各 agent onboarding 信；找不到时查 bus archive 的 onboarding 信。

## Round 编号惯例

每轮 = 一个完整 serve（我发挑战）→ 她的 data-backed 回应 → 我 ack + 定调。
当前轮次见 bus archive 最后的 "Round N" 主题。重大产出命名 EXP-NN：
EXP-57 遥测/regen · EXP-59 age gate · EXP-59b grace exit · EXP-60 簇约束 · EXP-61 月度止损 · EXP-62 容量/AUM。

## 完成判定（什么时候一个 claim 算结案）

- 她给出数据支持的回应（表格不是形容词）
- 我 ack 并写明：接受 / 接受但修改 / 拒绝（附理由）
- 重大结论进 9/10 式简报；方法论进 docs/CHANGELOG.md
- 分歧不裁决时：升级 Sho，附双方最强论证

## 长期运行（Manhattan 模式，Sho 2026-09-09 定）

**协作直到所有 requirement 关闭才完成——不是按轮次，是按问题清单。**

- **双路并行**：借鉴曼哈顿（铀路+钚路同推），开放问题用竞争性双路线同时研究
  （例：静态带 vs 滚动带、不同 gate 参数组），用数据裁决，不用辩论裁判
- **每人都有汇报角色**：loop 的能源不来自 Sho，来自结构——每个 agent 持有
  serve list，关闭一个 serve = 在自己 channel 直报一行，然后自动发下一个 serve
- **未解决问题循环**：问题没解决 → 声明假设 → 用数据从新角度测它 → 更新/否决假设
  → 循环，直到 requirement 关闭或升级 Sho 裁决
- **requirement 清单**：checkpoint 后由 Sho 拍板的验收项 + EXP 栈验证项构成；
  全部关闭 = 协作完成，否则持续运转
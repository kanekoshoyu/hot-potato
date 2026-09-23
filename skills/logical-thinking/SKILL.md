---
name: logical-thinking
description: Use before any diagnosis — observe, verify, then solve.
tags: [methodology, debugging, evidence]
---

# Logical Thinking — 观察 → 考证 → 解决

Sho's bar (2026-09-19): 观察和考证先行，解决方案是最简单的部分。这是习惯，不是天赋。每个诊断都走完这个环，无论多小。

## The three questions (answer in order, in writing)
1. **问题是什么？** One sentence, no cause embedded. "prod pool's push to A's gateway returns 401" — not "A is ignoring us" (that is a conclusion, not a problem).
2. **怎么确定？证据呢？** List observations. Each must be reproducible (tool output, id, timestamp). Mark which hypothesis each observation supports or kills.
3. **怎么解决？** Only after 1–2. Smallest change that removes the *verified* cause.

## Anti-story rules (the failure mode is always story-telling)
- **解释必须吃下全部观察。** One counter-observation kills a hypothesis. A root cause that ignores half the data is a story. Live example (2026-09-19): "401 → she never got woken → no reply" ignored that the fleet twin was *delivered* seconds earlier — the contradiction was already in hand.
- **一个数据点不构成因果链。** Name the discriminating experiment that separates remaining hypotheses BEFORE running it. The 401 explained the prod-side failure but not the silence; the silence needed its own test.
- **先读手头数据，再发新探针。** Evidence collected but not consumed is the #1 embarrassment: the peer's gateway URL was sitting in a registry dump printed minutes before "discovering" it with a fresh probe.
- **区分「没证据」和「证据反对」。** No reply ≠ not delivered. delivered ≠ read ≠ acked — the state machine exists so you never conflate these.

## Context discipline (clones forget; write it down)
- Facts bought with real probing (endpoints, auth models, who runs where, per-pool credentials) go to memory/skill **the same turn**. Unrecorded = lost at next clone spin-up; "I knew that" is worthless if it is not stored.
- Scope facts with coordinates: which pool, which machine, which direction. "It works" without coordinates is noise.

## Closing the loop
- Fix, then **re-verify the original observation**, not a proxy.
- Land the lesson where the next agent will trip: README for product behavior, skill for method, memory for environment facts.

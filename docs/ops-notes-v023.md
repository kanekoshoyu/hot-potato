# Hot Potato 运维笔记（v0.2.3 时代，2026-09-10 夜）

## CI/CD（Sho 指令：push = deploy）
- hot-potato: push main → GitHub Actions test → self-hosted runner `daometric-dev`
  自动 docker rebuild + restart + 验证 /version + dashboard
- runner 装在 ~/actions-runner（user mode `./run.sh`，无 systemd；
  注册命令要 `-X POST` 拿 fresh registration token，1h 过期）
- deploy job 的 runs-on label = `[self-hosted, daometric-dev]`（不是 daometric，踩过）
- albatross: Diana 建的 version-bump CD（改 apps/*/Cargo.toml version → Coolify webhook）
  - secrets 已配齐（COOLIFY_TRADER_WEBHOOK 等 4 个）；research webhook 曾失败
- daometric.com 网站：astro build → rsync nginx；runner `daometric-host` + nginx
  都在 Anastasia 机（169.58.177.121）——不在本机！

## v0.2.3 Registry 持久化（bcd1b11）
- bug：容器重启清空 in-memory registry → 全舰队静默退化 poll 模式
  （Sho 的 5 秒 drill 抓到；信不丢但不推，无告警）
- 修复：register 后原子写 /data/registry.json；启动 with_persistence() rehydrate
- 日志确认：`registry: rehydrated 5 agent(s)`
- 29/29 tests；实测两次重启 × drill 全绿

## a2a notifier 语义（重要）
- push 是同步 SendMessage：gateway 等 agent 处理，HttpTransport timeout=120s
- delivered 状态可能延迟几十秒——**验收别卡 5 秒**，卡"状态最终翻转"
- 连接拒绝秒回 → "push failed" 日志；Skipped（not in registry/poll-only）无日志
  → 排障用 decoy agent 注册坏 url 来确认 registry lookup 是否工作

## 部署拓扑事实
- 本机（46.250.228.187）= dev potato :8080 + 舰队 gateway 127.0.0.1:990x（loopback only）
- Anastasia 机 = prod potato + trader + Coolify + daometric.com nginx/runner
- Coolify API key 只在 Diana 机的 apps/research/.env（本机没有）
- trader 0.88.3 卡点 = Anastasia 机 Coolify 构建（night watch: /tmp/night-watch.log）

## gh CLI 测试结论（9/10 晚）
- E2E 全通：branch→PR#1→CI 44s→merge→删分支，全部 gh 命令
- git 身份坑：容器/新机会报 identity unknown，需配 user.name/email

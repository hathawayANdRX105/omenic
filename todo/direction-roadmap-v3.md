# omenic 方向路线 v3 —— 先抄再自研

> 2026-09-12。方向调整:底层框架抄 OMP(oh-my-pi)的 agent loop 与 subagent 编排;记忆借鉴 jcode 的图结构 + 短期/长期双层;GUI 以 tolaria 为蓝本重写为 Dioxus;网络设计参考 Claude Code;DSH 插件系统**本期明确不做**。
> 前序:`enhancement-roadmap-v2.md`(2026-09-02,其中 M1 安全项仍有效);`todo/handoff/` 各交接稿。
> 调研底稿:OMP agent loop、jcode 记忆、tolaria GUI 三份深度调研已并入本文 §1/§2/§3(全部结论带 file:line 或文件引用,可复核)。

## 0. 决策记录

| 维度 | 参考项目 | 采纳什么 | 本期 |
|---|---|---|---|
| 底层框架 | **OMP** `~/projects/harness/oh-my-pi` | agent loop 纯函数化 + 事件流 + subagent 编排语义 | ✅ 核心主线 |
| 内存管理 | **jcode** `~/projects/harness/jcode` | 图结构记忆(HashMap 邻接表)+ 短期 pending / 长期图双层 + 注入管线 | ✅ 最小子集 |
| 插件 | DSH `~/projects/harness/deepseek-harness` | 「一切皆插件」(Cordis) | ❌ 延后(体量大,见 §4) |
| 网络 | Claude Code | 流式/重试/降级的公开行为 | ✅ 与 LLM 流式层同批先做(见 §5) |
| GUI | **tolaria** `~/projects/tolaria`(markdown 知识库桌面应用,TypeScript) | 布局/交互范式 → Dioxus 重写 | ✅ web GUI 主线 |

**策略**:「先抄再实现自己的部分」——先按参考项目把语义结构原样搬过来(每处标注出处),跑通、有测试后,再做自己的替换设计。禁止边抄边重构。

---

## 1. OMP 移植:agent loop + subagent 编排

### 1.1 调研结论(关键前提)

OMP 的 agent loop 与编排**不在 Rust 里,而在 TypeScript (Bun) 层**;Rust crates(pi-ast/pi-iso/pi-natives 等)全是底层原语,无 agent 语义。所以值得抄的是**语义结构**,不是代码:

- Agent loop 核心:`packages/agent/src/agent-loop.ts`(2950 行),入口 `agentLoop(prompts, context, config, signal, streamFn) → EventStream<AgentEvent, AgentMessage[]>`(agent-loop.ts:526)
- LLM 流式层:`packages/ai`(`streamSimple`、`AssistantMessageEvent`、ai/src/types.ts:1283)
- 编排/harness:`packages/coding-agent`(agent-session.ts 单文件 9730 行,是反面教材;task/ 目录 13k 行是编排精华)

### 1.2 值得原样抄的 8 条设计(按价值排序)

1. **loop 是纯函数**:所有宿主策略走 config 钩子(`beforeModelCall` / `transformContext` / `getSteeringMessages` / `beforeToolCall` / `afterToolCall` / `onTurnEnd`),loop 不 import 任何 app 层。对照 omenic:orbit 的 `run_agent_streaming` 已近似,但 compaction 内联在 loop 里(见 OL-2)。
2. **双层循环**:内层跑 tool batch + steering,外层在 yield 点拉 follow-up;steering(用户插话)与 follow-up(异步结果投递)是**两个队列、两种语义**(agent-loop.ts:1044, 1449-1484)。
3. **`TERMINAL_TOOL_RESULT_ABORT_REASON`**:用 abort reason 表达「工具结果即终点」(子代理 yield、审批拒绝),优雅停机且不与用户 abort 混淆(agent-loop.ts:1413-1415, agent-session.ts:3445-3451)。omenic subagent 收尾可直接套用。
4. **合成 tool_result 保配对**:aborted/error/length/skipped 四路统一造占位结果(`__synthetic` + `source` 判别字段,agent-loop.ts:2851),保持 tool_use/tool_result 配对 invariant。omenic orbit 已有 skip orphan 逻辑,补齐合成语义即可。
5. **prepared dispatch + 写回消息本体**:tool args 验证/修订在 `message_end` 前完成并写回消息(agent-loop.ts:2160-2236),历史/UI/持久化/调度四方看到同一份。
6. **shared/exclusive 工具并发 + interruptible/steeringSignal 双信号**(agent-loop.ts:2716-2742)。
7. **spawn 信号量 + preflight 全通过才派发 + in-process 子会话**:子代理 = 同进程新建一个完整 AgentSession + 独立 JSONL,不是子进程(executor.ts:2683 文件头自述);preflight(`resolveEffectiveSubagentPolicy`,structured-subagent.ts:245)统一校验深度/allowlist/模型后才分配资源。
8. **compaction 外挂**:loop 只留钩子(`syncContextBeforeModelCall` / `transformContext`),压缩策略全部上移到 SessionMaintenance,在 prompt 边界做 threshold/overflow/投机压缩(agent-session.ts:2982-3046, compaction.ts:1507)。

**终止协议(yield)** 是 OMP 最独特的子代理设计:子代理靠 `yield` 工具声明完成,terminal yield 经 `outputSchema` 校验(3 次重试)后由 `afterToolCall` 钩子触发特殊 abort;空转提醒(≤3 次)、软请求预算超限强停 + `toolChoice` 强制 yield(executor.ts:579, 1929)。

### 1.3 omenic 落地:OL 系列

| 项 | 内容 | 对应现状 |
|---|---|---|
| OL-1 | 事件模型对齐:orbit 事件枚举补齐 `message_update`(流式增量)、`tool_execution_update`(partial result);对外推快照(不可变) | orbit lib.rs 现在消费收集后的 Vec(v2 roadmap EC-14),先改 drive 方式 |
| OL-2 | loop 纯函数化:compaction 移出 loop 变成 prompt 边界外挂(与 v2 EC-7/EC-8 同批);turn cap 进 loop(EC-9) | orbit lib.rs:319-321 原地替换消息 |
| OL-3 | steering / follow-up 双队列 + yield 点拉取 | 无;daemon steer 已有雏形(runner steer 通道) |
| OL-4 | subagent 编排升级:spawn 信号量(maxConcurrency 可热更)+ preflight 集中校验 + in-process 子会话(共享 daemon 进程,独立 session 记录)+ yield 终止协议 | crates/subagent 手工重喂 loop、MAX_TURNS=10 |
| OL-5 | 并发原语抄 `task/parallel.ts`:abortable Semaphore + `mapWithConcurrencyLimitAllSettled`(不 fail-fast、保序) | 无 |

**必读文件**(按序):agent-loop.ts 全文 → types.ts:60-860(钩子契约)→ structured-subagent.ts(688 行,最浓缩)→ task/index.ts:500-1530 → executor.ts:2683-3540 → tools/yield.ts + yield-assembly.ts → job-manager.ts → parallel.ts。

**负面样本提醒**:agent-loop.ts 里混了大量 provider 特判(Cursor/Harmony/Cerebras)——omenic 实现时这些必须放 adaptor 层;AgentSession 9730 行是过度膨胀反例,但「post-prompt 维护层」的分层本身是对的。

---

## 2. jcode 内存管理:图结构记忆 + 短期/长期双层

> 范围:同时覆盖**会话内**(长会话上下文维持——短期 pending 注入层,并与 compaction/ContextLog 即 v2 EC-7/8 配合)与**跨会话**(长期图,学习与检索)。jcode 里这两层由后台 MemoryAgent 桥接。

### 2.1 架构(三层)

| 层 | jcode 位置 | 说明 |
|---|---|---|
| 长期·项目图 | `~/.jcode/memory/projects/<dir_hash>.json` | MemoryGraph 单文件 JSON,进程内 mtime 缓存(memory/cache.rs:36) |
| 长期·全局图 | `~/.jcode/memory/global.json` | 跨项目记忆 |
| 短期·pending 队列 | `static PENDING_MEMORY: Mutex<HashMap<session_id, PendingMemory>>`(pending.rs:10) | 120s 新鲜度、取走即删;注入去重三件套:90s 签名 / 180s·80% 集合重叠 / 45min 注入 TTL(pending.rs:32-50) |
| 桥接 | 后台 MemoryAgent(singleton,memory_agent.rs:98) | 每 turn 异步跑:召回→rerank→注入→维护,结果落后一 turn、非阻塞 |

### 2.2 图数据模型(全部在 `crates/jcode-memory-types`,零依赖,最值得抄)

- `MemoryGraph`(graph.rs:231-256):`memories: HashMap<String, MemoryEntry>` + `tags` + `clusters` + **正向边** `edges: HashMap<String, Vec<Edge>>` + **反向边** `reverse_edges`(为 BFS 服务)。**没有用 petgraph**——HashMap 邻接表就是正确形态(JSON 可序列化;docs 里的 petgraph 是未实现的旧设计稿)。
- 6 种边(graph.rs:92-108):`HasTag / InCluster / RelatesTo{weight} / Supersedes / Contradicts / DerivedFrom`,各带 BFS 遍历权重。
- `MemoryEntry`(lib.rs:233-274):id / content / category(Fact·Preference·Entity·Correction·Custom)/ tags / `search_text`(预归一化,BM25 用)/ trust(High=用户说·Medium=观察·Low=推断)/ confidence(按类别半衰期衰减)/ strength + reinforcements(面包屑)/ `active` + `superseded_by`(软删除)/ embedding 内联。
- 写入即建图:add_memory 自动建 Tag 节点 + 边;矛盾检测 → 新建 + 老 entry `supersede()` + Contradicts 边(memory_agent.rs:1116-1130)。
- BFS 级联检索 `cascade_retrieve`(graph.rs:546-618):种子出发,`score × edge_weight × 0.7^depth`。

### 2.3 召回管线(每 turn 自动)

embedding 当前上下文 → **hybrid 检索**(dense 余弦 + 手写 BM25 K1=1.2/B=0.75 + RRF 融合 k=60,memory.rs:642-728)→ 过滤本 session 已注入 → listwise LLM rerank(一次调用排全部候选;失败时只复用上次 verdict,**绝不降级注入未审核结果**)→ 组装 `<system-reminder>` user 消息追加到对话末尾(最多 5 条/turn),只在新用户 turn 消费(prompting.rs:31-36)。

显式通道:`memory` 工具,action 枚举 remember/recall(mode: recent·semantic·cascade)/search/list/forget/tag/link/related(tool/memory.rs:91-126)。

### 2.4 jcode 的四条教训(直接进设计约束)

1. **硬 cosine 阈值毁掉召回**(0.5 阈值曾把 recall@5 打到 0.0,已移除):候选池要宽,精度交给后置 rerank。
2. **图的角色是 reranker/去重/数据卫生,不是一级检索器**:`cascade_retrieve` 在自动召回路径中完全没被调用,只被手动工具用;MEMORY_GRAPH_PLAN.md:104 明示此原则。**omenic 不必一上来做图遍历检索**。
3. **写入路径去重比事后清理便宜**:0.90(抽取时)/0.85(存储时)双阈值余弦去重,重复即 `reinforce()`(strength+1 + 面包屑);本 session 抽取出的立即 `mark_memories_known` 防回声。
4. **评测先行**:jcode 所有召回改进由 bench 驱动(0.0→0.53→0.75 recall@5,`src/bin/memory_recall_bench.rs`)。omenic 写检索代码前先建 recall harness。

### 2.5 omenic 落地:MEM 系列(最小可行子集)

| 项 | 内容 | 基础 |
|---|---|---|
| MEM-0 | recall harness:固定语料 + query 集,输出 recall@5;所有后续项的验收门 | 新建,参照 jcode bench |
| MEM-1 | 数据模型演化:现有 `crates/memory`(430 行 append/list/search,零消费者)升级为 MemoryEntry(补 trust/confidence/strength/active/superseded_by)+ MemoryGraph HashMap 邻接表 + 6 种边 | jcode-memory-types ~200 行 struct 可直接抄 |
| MEM-2 | 注入管线:pending 队列 + 注入去重三件套(TTL/签名/重叠)+ `<system-reminder>` user 消息接缝 + 只在新用户 turn 消费;**先不做 embedding,BM25/关键词起步** | pending.rs ~150 行,独立于 LLM |
| MEM-3 | memory 工具:remember/recall/search/forget 注册进 builtin_tools(即 v2 EC-5,默认 off) | 现成权限通路 |
| MEM-4 | 写入管线:话题切换(turn embedding 余弦<0.3)/ 每 12 turn / session 结束三条触发 + 0.85/0.90 去重 + supersede;embedding 后端可用 OpenAI 兼容 `/v1/embeddings`(不引本地 ONNX) | memory_agent.rs 抽取 prompt 可抄 |
| MEM-5(可选) | 图利用:`memory {recall, mode: cascade}` 手动路径 + `link/related` 工具动作 | graph.rs BFS |

> **M2 交付进度(2026-09-12)**:MEM-0/1/2/3 已合并——#331(图+召回+harness,recall@5=0.80/门 0.70)、#332(注入 buffer 四道门 + remember_update/supersede 通道)、#333(三工具 `memory_append`/`memory_search`/`memory_list` 接进 builtin_tools,env 门 default-off;顺带 `Memory::append` 返回 id、`RecallHit` 带 text 关掉两处 workaround)。实现期两处收敛:①图边从计划 6 种裁到 3 种(SharedTag/Supersedes/Contradicts,InCluster/RelatesTo/DerivedFrom 随 MEM-4/5 落地);②开关走 env(`OMENIC_MEMORY=1` + 已存在的 `OMENIC_MEMORY_DIR`)而非 config 字段——接线 daemon 时若要改构造期注入 handle,在这一层换。MEM-4a 已并入 #334(embedding 后端、写入去重 0.90、ExtractionTriggers);MEM-4b 已并入 #335(web 接线:首见 startup 抽取、回合边界触发驱动、注入排空接缝在两 flushes 生效;三链路 e2e 锁住调用链)。**M2 主线结束**;MEM-5(图 link/cascade 手动动作)可选暂缓。已知 rake:①抽取经主模型同渠道(无 sidecar),触发器节拍是唯一频控;②daemon 层(omp worker)的记忆接线不在本批,留给 M3 重新定夺;③web 重启丢 pending 注入(JSONL 本体不受影响)。

**明确不抄**:sidecar LLM 共识 voting 链(~800 行,依赖其多 provider 基础设施);cluster/自动 tag 推断(jcode 自己的 plan 也判「写多读少,建议砍」)。

---

## 3. GUI:tolaria → Dioxus 重写

> 蓝本:`https://github.com/refactoringhq/tolaria` —— 「Desktop app to manage markdown knowledge bases」(作者 Luca,refactoring.fm),Tauri v2 + React 19 + TS。已取回至 `~/projects/tolaria`(jsDelivr 按 commit 234c40e 固定快照,git clone 因 GitHub 对代理出口 IP 限流不可用;2395 文件 / 26.3MB)。
> codegraph:`code-review-graph build` 已在其上建图(1731 代码文件 / 25561 节点 / 228239 边,库存于 `~/projects/tolaria/.code-review-graph/`),后续可用 `wiki` / `detect-changes` 增量查询。

### 3.1 tolaria 是什么/值得抄什么

**自述架构**(`docs/ARCHITECTURE.md`,1230 行,质量极高):文件系统是唯一事实源(vault = 一堆带 YAML frontmatter 的 md;cache 与 React state 都是可重建的派生物,「disk-first writes / 崩溃可恢复 / reload 兜底」六条 invariant)。四面板 Bear 风格布局:

```
┌────────┬─────────────┬─────────────────────────┬────────────┐
│Sidebar │ Note List   │ Editor(flex)            │ Right Panel│
│220-400 │ 220-500     │ 单开一条,无 tab(ADR-0003)│ 200-500/隐藏│
│筛选/类型│ 列表或 Pulse │ 面包屑+富编辑/RAW/diff    │ Properties │
│/文件夹树│ 搜索/排序    │ wikilink/callout/mermaid │ 或 TOC     │
├────────┴─────────────┴─────────────────────────┴────────────┤
│ StatusBar: 版本 │ 分支 │ 同步时间 │ vault        │ Cmd+K 面板 │
└──────────────────────────────────────────────────────────────┘
```

面板间是可拖拽 `ResizeHandle`,宽度 clamp 后存 localStorage(`useLayoutPanels`);右面板 Properties 与 TOC 互斥挂载(`useRightPanelExclusion`)。

**AI 工作区**(对 oi-web 最有价值的部分,`components/Ai{Panel,Message,ActionCard,Workspace*}.tsx`):

- **多会话工作区** `AiWorkspace`(1099 行):会话侧栏 tabs + 每会话独立的 target/model/permission 选择器、自动生成会话标题、archive、dock/弹出双模式(弹出 = 独立轻量窗口路由,关窗不丢会话)。composer 工具栏一行放下 target/model/permission/设置。
- **回合渲染** `AiMessage`(450 行)——一个 turn 的结构:**UserBubble**(右对齐,max-85%,圆角 12/12/2/12,引用笔记渲染成 type 色 pill)→ **ReasoningBlock**(流式时自动展开、结束自动收起,用户手动 toggle 覆盖自动行为)→ **ToolUseBlock**(默认收起,显示工具数徽标,有 pending 时徽标 `animate-pulse`,展开是 AiActionCard 列表,每卡可独立展开看 input/output)→ **ResponseBlock**(markdown + hover 才显 actions:regenerate/copy/fork)→ StreamingIndicator(typing dots)。
- **归一化事件流**:8 种 CLI agent(Claude Code/Codex/Copilot/OpenCode/Pi/Antigravity/Kiro/Hermes)各有适配器,后端 `ai_agents.rs` 归一成 TextDelta / ThinkingDelta / ToolStart / ToolDone / Done;前端 stop 走 request-scoped 事件名注册表杀真子进程(不是假装停止)。
- **上下文快照**(`ai-context.ts`):activeNote + openTabs + noteList + vault 统计 + referencedNotes 组成 JSON,大笔记 head/tail 截断并标注 `bodyTruncated`,提示 agent 需要全文时走 MCP `get_note`。

**其余可抄范式**:CommandPalette(Cmd+K,`appCommandManifest.json` 单一来源驱动菜单/快捷键/QA)、Pulse View(git 活动按天分组的时间线)、Neighborhood 模式(选中实体后,note list 变成「出边关系组在前、反链在后、空组显示 0」的图浏览)、Context Building、file watcher + 乐观 UI + 失败回滚。

**明确不抄**:BlockNote/TipTap 富编辑器(React 生态,oi-web 现阶段聊天输入框用不上 WYSIWYG;将来要 raw 编辑再上 CodeMirror JS interop)、tldraw 白板、IronCalc 表格、多窗口(deep-link/窗口管理是 Tauri 域)。

### 3.2 Dioxus 重写映射

 oi-web 现状:Dioxus **0.7** LiveView(axum)+ Tailwind inline-in-rsx;`components/{chat,sidebar,statsview,statusline,taskpanel,ui}`;`pages/{workspace,stats,config_page}`;聊天已是「用户右气泡 + 时间序 interleave + 工作过程折叠 + 单滚动面板浮层输入」;侧栏 3 级树 + 可折叠 rail;模型/thinking 用可复用 Dropdown。

| tolaria 概念 | oi-web 落点 | 改动量 |
|---|---|---|
| 四面板 shell + ResizeHandle 拖拽 + 宽度持久化 | workspace 页改为 sidebar(已有)│ 会话/任务列表 │ chat │ 右侧 inspector(taskpanel 挂载位);`useLayoutPanels` → GlobalSignal 持久宽度(memory: web chrome controls 已有先例) | GW-1 |
| AiMessage 回合结构 | `components/chat.rs` 升级:推理块自动收起、工具块计数徽标 + pending 脉冲、hover actions 行(copy/fork/regenerate)——与现有「工作过程」折叠合并成一套 run-based 分组 | GW-2 |
| AiWorkspace 多会话(target/model 选择器进 composer 行、生成标题、archive) | 会话树(已有)+ composer 工具栏改造;标题生成后端出 | GW-3 |
| 归一化事件流 | orbit AgentEvent 已是同构事件源(message_update / tool_execution_*),OL-1 对齐后直接映射 LiveView patch;stop 按钮接 daemon abort(真停,不假停) | 随 M1 |
| CommandPalette | 顶部「搜索会话」升级为 Cmd+K 命令面板(命令 manifest 单一来源) | GW-4 |
| Pulse View | run ledger(`oi run.list`)渲染成按天分组活动流 | GW-5 可选 |
| Neighborhood | 任务依赖图浏览(远期,不立项) | — |
| 状态管理 | React state+localStorage → GlobalSignal;Tauri IPC → LiveView server function / eval | 遵循现有约定 |

**硬约定(沿用 memory)**:Tailwind 工具类内联 rsx,不建 CSS 文件类名;rsx 内禁 `let`;SessionDb 不得在 render 路径 open;`cargo build --bin oi-web`(bin/web)而非 `-p web`;提交前 `cargo fmt --all`。

### 3.3 执行顺序

GW-1(shell)→ GW-2(回合渲染)→ GW-3(composer/多会话)→ GW-4(命令面板)→ GW-5(Pulse)。每步用 [browser-use] 硬刷新后逐元素核验(对准被指元素,不留「应该渲染了」)。

---

## 4. DSH 插件系统 —— 本期明确不做

- **为什么延后**:DSH 基于 Cordis(时空可组合编程范式),「一切皆插件」意味着服务容器、依赖注入、生命周期事件、跨插件通信全套基础设施;其体量与 omenic 当前阶段不匹配,先抄会拖住 loop/GUI 主线。
- **触发条件**(满足再评估):① OMP 式 loop 稳定且有测试;② 工具/子代理接口定型;③ 出现真实的多形态扩展需求(第三方工具、外部 UI)。
- **现在的廉价准备**(只做这三件):工具注册表保持 `impl Tool` trait 对象形态;新增能力一律带 manifest 式描述(name/description/schema 已具备);不为插件化提前抽象——避免猜错接口。

---

## 5. 网络层(Claude Code 参考)——与 OMP LLM 流式层同批先做

「网络设计参考 Claude Code」按公开材料解读为 **LLM provider 网络层**的鲁棒性行为(官方 best practices 文档是使用工作流层面,不含内部实现):

- 流式 SSE 断线处理:已发 delta 不重放,断点后续传语义(v2 EC-6 的「首个 delta 前才可重试」一致)。
- 429/5xx 分类重试 + 指数退避 + `retry-after` 头遵从。
- 模型/渠道 fallback(主渠道连续失败时切备用渠道)。
- 读超时防静默挂起(v2 EC-1)。

**时序(用户拍板 2026-09-12)**:这部分**提前**,与 OMP 的 LLM 流式层(`pi-ai` 语义:provider 适配、`streamSimple`、`AssistantMessageEvent`、错误分类、dialect)合成 M1 的第一批——loop 的重试/流式/中断语义都站在这层之上。落到 omenic 即 crates/adaptor 升级(共享 `ureq::Agent`、timeout、重试分类器、fallback)。NC-2 = 辅助 spike:对照 Claude Code 公开行为列网络层行为清单,半页纸。

---

## 6. 里程碑(用户拍板:三块地基完善后,直接复刻 tolaria)

| 里程碑 | 内容 | 出口标准 |
|---|---|---|
| M1 底层框架(OMP) | **第一批(先做)**:LLM 流式层 + 网络层——adaptor 对齐 pi-ai 语义 + §5 网络行为(EC-1 读超时、EC-6 重试分类、fallback);随后 OL-1 事件流对齐 → OL-2 loop 纯函数化 + compaction 外挂(EC-7/8/9)→ OL-3~5 编排(steering/follow-up 双队列、subagent 信号量+preflight+yield、并发原语) | 流式带读超时/重试/降级;事件流对齐 OMP;compaction 外挂化;subagent 编排可用 |
| M2 内存管理(jcode) | MEM-0 recall harness → MEM-1 数据模型演化(图) → MEM-2 注入管线(短期层,接 EC-7/8 的 ContextLog) → MEM-3 memory 工具 → MEM-4 写入管线 | recall harness 有基线;注入跑通;memory 工具默认 off 可开 |
| M3 复刻 tolaria(Dioxus) | GW-1 四面板 shell → GW-2 回合渲染 → GW-3 composer/多会话 → GW-4 命令面板 → GW-5 Pulse;全程接 harness 真数据(daemon/session/run),不是 mock | §3.2 各项落地;聊天事件源来自真 orbit/daemon;浏览器逐元素核验 |

执行顺序:**M1 → M2 → M3 串行**(GUI 后置)。M1 内部 LLM/网络先行——loop 的流式/重试/中断语义依赖这层。M3 开始时按 §3.2 的映射直接复刻,不再做中间态试点页。

## 7. 与 v2 路线的关系

- **保留执行**:EC-1、EC-2、EC-3(M1 安全,任何方向下都成立)、EC-7/8/9(并入 M1)、EC-1/6(= M1 网络层第一批)。
- **被吸收**:EC-5(= MEM-3)、EC-10(TUI tool loop,由 OL-1/2 顺带解决)、EC-14(= OL-1)。
- **重排**:EC-11(cli 拆分)、EC-12(runner template)、EC-13(CRG→spec)降级为机会项,不占主线带宽。

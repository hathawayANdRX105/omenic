# omenic 任务模板手册（phases / steps）

> 日期：2026-08-07
> 来源：compass-ws `config/templates/{phases,steps}/*.yaml`（17 个模板）
> 用途：临时——在你有 omenic 自己的原生任务机制之前，沿用 compass 的「编排例 / 工序模板」心智模型，**从手册里挑选适用的填进 Issue 的 Done when 或 PR 的 Construction plan**。
> 不放 AGENTS.md 的原因：模板细节很长（17 模板 × 几十到几百行），全收进 AGENTS.md 会让每次会话启动都读一遍，污染默认上下文。**按需查**才是正确用法。

## 0. 总则

| 维度 | phase（编排例） | step（工序模板） |
|---|---|---|
| **粒度** | 大方向，多个子动作 | 一个原子动作 |
| **写在哪儿** | Issue 的 **Done when** | PR 的 **Construction plan** |
| **结构** | phase 入口 + 同前缀的子步骤 | 单 task，caller 接 dep 边 |
| **何时挂** | 一个 topic 想要**多步编排**时挂 | 单 PR 验收某项**可观测产物**时挂 |
| **互斥选择** | plan 用 `capability` 或 `scheme` 二选一 | 各 step 按场景选，叠加挂 |

**核心区分**：
- phase 是**计划骨架**，给读者一张「这事分几步」的图
- step 是**单步协议**，给执行者一份「这一步怎么做」的清单

## 1. Phase 矩阵（5 个）

| 模板 | kind | 何时用 | 不该用 |
|---|---|---|---|
| **`onboarding`** | chore | 一次性：新项目/新 worktree 的 readiness 清单（读 AGENTS、确认根目录、检查工具） | 重复性工作 |
| **`capability`** | chore | **plan 路径已固定**：只需要选/套/接模板，不再讨论方案 | 需要讨论方案 |
| **`scheme`** | task | **plan 路径未定**：要 options/feasibility/approach/ready-summary/approval/emission 全套讨论 | 路径已固定 |
| **`spec-change`** | decision | 跨多个 capability 的**硬规则**（schema/invariant），且需要编号、文档化、可证明 | 单文件改动 |
| **`orchestration`** | decision | 编排层反思——verify 后发现编排模板错位、recipe 缺失、guide 反模式时挂这个 phase 决策怎么改 | 不是编排层问题 |

### 选择速记

```
新项目首次接入？     → onboarding
路径已知直接做？     → capability
路径还没定？        → scheme
改跨 capability 规则？ → spec-change
编排层本身要反思？    → orchestration
```

### capability vs scheme（最常被问）

- `capability`：plan 已固定，**无讨论链**。只挂 choose-template → apply-template → shape-graph。
- `scheme`：plan 待定，**有讨论链**。挂 scope → options → feasibility → approach → ready-summary → approval → emission(phase)。

> 一条 plan 不能同时挂两个。

### spec-change 内部顺序（phase entry 内的 deps）

```
phase(决策)─┬─→ document(规格化)
            └─→ audit(对齐 contract) ─→ enforce(证明) ─→ review
                       └─→ enforce(证明) ──┘
```

### orchestration 内部顺序（双入口）

```
evidence(五角度分析: Shape/Gate/Template/Emit/Guide)
   ↓
phase(决定 retain/edit-guide/edit-template/none)
   ↓
catalog(决定是否进 catalog，仅当 promotion 阈值达标)
   ↓
dogfood(在隔离 root 跑一遍)
```

## 2. Step 矩阵（12 个）

| 模板 | kind | 一句话 | 接 dep | 备注 |
|---|---|---|---|---|
| **`implement`** | task | 实施：分 module/test/cli 路径，至少一次 smoke | caller 接线 | 默认串行；多 work-item 才并行 |
| **`verify`** | task | 验证：contract 描述 → focused test → smoke → diff-check | `-dep Implement-terminal` + approval（如有） | **唯一跨 phase gate**；不许跨 phase 挂在子动作上 |
| **`review`** | chore | 审查：scope/CRG/code/simplicity 四面 + 解决 P0/P1 | `-dep Implement-terminal` + approval（如有） | 任何 P0/P1 未处置不能 done |
| **`document`** | chore | 文档：design/feature/manual 同步 + reader-smoke | `-dep Implement-terminal` | 三类文档只更新「真的改了」的 |
| **`tidy`** | chore | 清理：删 obsolete 导入/注释；如 document 存在，`-dep document` | caller 接线 | 「删旧不留新」是合规要求 |
| **`publish`** | chore | 发布：在授权下建 GitLab Issue/MR，**仅人工授权** | approval | 严禁自动 commit/push/merge |
| **`readiness`** | chore | 工具检查：cx/lsp/codegraph/crg/lean-ctx/cpulimit/skills | — | 每个工具三选一：used / not applicable / unavailable |
| **`invariants`** | chore | 17 项编排硬性检查（parent-vs-dep/template-parent/task-ids/branch-role/docs-tests/issue-pr/approval-gate/uses-template/guide-run/loop-*5/multi-run-*4） | — | 复合 step；分 base/loop/multi-run 三组 |
| **`loop-optimization`** | task | 一轮六部曲：partition/smoke/gaps/patch/re-smoke/stop | — | 多轮就**多次挂**这个 step，Cycle/Round 递增 |
| **`multi-run`** | task | 三个隔离 candidate worktree 并行实现，score 后合 winner | — | 每个 worktree 一份独立 commit |
| **`handoff`** | chore | 交接：progress 记录当前状态/阻塞/下一步命令 | — | **新 agent 必须能从 progress 续接** |
| **`template-feedback`** | task | 反馈：bin/passk S0 失败时**有界**地改 catalog | — | 仅在 repeat/hard-defect 阈值达成时改 |

### Step 互斥/顺序

- **`implement → verify → review → document → tidy`** 是默认串联顺序
- **`document` 在 `review` 之后**：`review` 可能改实现，document 必须跟最终版
- **`tidy` 在 `document` 之后**：document 改 markdown 时 tidy 顺手清理过期链接
- **`loop-optimization` 仅在 smoke 失败时挂**：成功就别循环
- **`multi-run` 仅当需要「三选一择优」时挂**：不是默认动作
- **`handoff` 在每次会话结束或阻塞时挂**：不让状态丢失
- **`invariants` 挂在 verify/review 之前**：作为 gate
- **`publish` 永远挂最后**：只在所有验收后挂

### invariants 子分组速记

| mode | 检查项 | 触发场景 |
|---|---|---|
| **base** | parent-vs-dep / template-parent / task-ids / branch-role / docs-tests / issue-pr / approval-gate / uses-template / guide-run | 任何 topic |
| **loop** | loop-cycle-chain / loop-stop-has-tier / loop-stop-has-rubric-score / loop-gaps-monotonic / loop-patch-scope | 挂 `loop-optimization` 时 |
| **multi-run** | bn-chain / bn-isolated-worktrees / bn-score-cites-rubric / bn-merge-record | 挂 `multi-run` 时 |

## 3. 典型组合（最少惊讶路径）

### A. 加新 capability（plan 已固定）

```
phase:    capability (plan-shell)
steps:    implement → verify → review → document → tidy
gates:    invariants (base 子集) 在 verify 前
```

### B. 加新 capability（plan 待定）

```
phase:    scheme (plan-shell，含 options/feasibility/approval)
steps:    implement → verify → review → document → tidy
gates:    invariants (base) + approval gate 由 scheme 自带
```

### C. 跨 capability 规则变更

```
phase:    spec-change (含 document/audit/enforce/review 子任务)
steps:    implement (按规则改 code) → verify → review
gates:    invariants + enforce 子步骤
```

### D. 编排层反思

```
phase:    orchestration (含 evidence/phase/catalog/dogfood)
steps:    —（这是 phase 内的编排决策，不另挂 step）
gates:    —
```

### E. 需要在多方案中择优（如三种实现思路）

```
phase:    capability 或 scheme（按路径是否已定）
steps:    multi-run → verify → review → document → tidy
gates:    invariants (base + multi-run 子集)
```

### F. Smoke 失败，需要回炉

```
phase:    已有 phase（通常 capability/scheme）
steps:    loop-optimization × N 轮（每轮一 step）→ implement 改 → verify 再来
gates:    invariants (loop 子集) 在 loop 期间持续挂
```

### G. 工作跨会话接手/交出

```
phase:    任意
steps:    handoff（任何阻塞/换班时挂）
gates:    —
```

## 4. 互斥与互斥例外

| 互斥对 | 规则 |
|---|---|
| `capability` vs `scheme` | 同一 plan 只能挂一个 |
| `loop-optimization` vs 一次性 done | 同一 round 不同时挂；每轮一个新 step |
| `multi-run` vs 普通 implement | 同一 work-item 不同时挂 |
| `publish` vs 不授权 | 无用户授权 → 不能挂 publish |
| `template-feedback` vs 自由改 catalog | S0 失败未达阈值 → 不能改 catalog |

## 5. 落地动作（怎么用这份手册）

1. **选 phase**：根据 §1 矩阵选 1 个 phase，写到 Issue 的 **Done when**——给出 phase 名 + 子动作清单
2. **选 step**：根据 §2 矩阵选 step 清单，写到 PR 的 **Construction plan**——按 §4 互斥检查
3. **检查组合**：对照 §3 看是否走的是典型组合；不是就单独说明原因
4. **不抄完整 YAML**：手动摘录 phase/step 的标题和 acceptance 关键点即可（YAML 本身由你视情况落到 task 模板）

## 6. 已知不足（待 omenic 原生化）

- 目前 17 模板来自 compass，**不是 omenic 原生**——omp RPC 概念没体现
- phase 内子动作的 deps 顺序写死在 YAML 里，omenic 上想跑得先做 store/graph 层
- `invariants` 的 17 个子检查是「文本清单」，omenic 化后想做自动化检查（1→n）
- 没有 omic 自己的 `cmp run <id>` 集成——等 MVP 跑通后转写

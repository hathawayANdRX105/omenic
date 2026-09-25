//! Default template payloads (phases + steps), extracted from `template.rs` so the
//! data does not inflate the module that parses/loads/applies them.

use super::TemplateKind;

pub const DEFAULT_TEMPLATES: &[(TemplateKind, &str, &str)] = &[
    (
        TemplateKind::Phase,
        "dev",
        r#"tasks:
  - key: implement
    title: "implement: deliver the work item"
    kind: task
    description: |
      实现工作项：模块/测试/CLI 路径；主路径跑通后至少一次 smoke。
    acceptance: |
      工作项已实现：模块/测试/CLI 路径 + smoke 通过。
  - key: verify
    title: "verify: check observable contract"
    kind: task
    description: |
      验证可观察契约：focused test + smoke + diff check。
    acceptance: |
      可观察契约验证通过：focused test + smoke + diff check。
  - key: review
    title: "review: scope/CRG/code/simplicity"
    kind: task
    description: |
      审查：scope/CRG/code/simplicity 四面；P0/P1 全部处置。
    acceptance: |
      审查完成：P0/P1 全部 disposition。
  - key: document
    title: "document: sync design/manual"
    kind: task
    description: |
      同步设计文档/功能文档/手册；只更新真正改动的。
    acceptance: |
      设计文档/功能文档/手册已同步。
  - key: tidy
    title: "tidy: clean obsolete artifacts"
    kind: task
    description: |
      清理脚手架/死代码/过期注释。
    acceptance: |
      脚手架/死代码已清理。
  - key: handoff
    title: "handoff: record evidence + next steps"
    kind: task
    description: |
      记录证据 + 下一步命令，新 agent 可接手。
    acceptance: |
      证据 + 下一步命令已记录。
deps:
  - task: verify
    depends_on: implement
  - task: review
    depends_on: verify
  - task: document
    depends_on: review
  - task: tidy
    depends_on: document
  - task: handoff
    depends_on: tidy
"#,
    ),
    (
        TemplateKind::Phase,
        "scheme",
        r#"tasks:
  - key: phase
    title: "scheme: plan-phase with discussion chain"
    kind: task
    description: |
      Plan-phase 编排例（路径未定）：scope → options → feasibility →
      approach → ready-summary → approval 全套讨论链。approval 是人工门。
    acceptance: |
      方案锁定，ready-for-dev: yes；approval 人工门已过。
  - key: scope
    title: "scope: define topic in and out"
    kind: task
    description: |
      定义 topic 的可观察结果、明确的 out-of-scope 边界与归属 milestone。
    acceptance: |
      description 与 acceptance 识别一个独立可评审的 topic。
  - key: options
    title: "options: 2-3 approaches with tradeoffs"
    kind: task
    description: |
      列出 2-3 个方案与 tradeoffs。
    acceptance: |
      至少 2 个方案 + tradeoffs 已列出。
  - key: feasibility
    title: "feasibility: 8-point checklist"
    kind: task
    description: |
      8 项检查表：边界/回滚/可测/兼容/安全/性能/文档/工具链。
    acceptance: |
      8 项检查表完成。
  - key: approach
    title: "approach: lock stack and change path"
    kind: task
    description: |
      锁定方案：技术栈 + 改动路径。
    acceptance: |
      方案锁定：技术栈 + 改动路径。
  - key: ready-summary
    title: "ready-summary: approvable execution summary"
    kind: task
    description: |
      可批准的执行摘要 + Work-items 列表 + Implement-terminal。
    acceptance: |
      执行摘要 + Work-items + Implement-terminal 已产出。
  - key: approval
    title: "approval: human gate"
    kind: task
    description: |
      人工审批门（agent 不自动关闭）。
    acceptance: |
      人工审批通过（approval: yes）。
deps:
  - task: options
    depends_on: scope
  - task: feasibility
    depends_on: options
  - task: approach
    depends_on: feasibility
  - task: ready-summary
    depends_on: approach
  - task: approval
    depends_on: ready-summary
  - task: phase
    depends_on: approval
"#,
    ),
    (
        TemplateKind::Phase,
        "capability",
        r#"tasks:
  - key: phase
    title: "capability: plan-phase with fixed approach"
    kind: task
    description: |
      Plan-phase 编排例（路径已固定）：无讨论链，只选/套/接模板。
    acceptance: |
      模板应用完成；entry-dep matrix 检查过；capability（非 scheme）理由记录。
  - key: scope
    title: "scope: define topic in and out"
    kind: task
    description: |
      定义 topic 的可观察结果与边界。
    acceptance: |
      description 与 acceptance 识别一个独立可评审的 topic。
  - key: choose-template
    title: "choose: select step recipes"
    kind: task
    description: |
      只选匹配 topic 的 step 模板，各挂到真实 phase 下。
    acceptance: |
      每个选中模板记录 category/name/phase parent/理由。
  - key: apply-template
    title: "apply: attach selected step recipes"
    kind: task
    description: |
      应用每个选中的 step；幂等（已存在则记录 already-applied）。
    acceptance: |
      每个 recipe 记录 applied|already-applied。
  - key: shape-graph
    title: "shape: set phases and real dependency gates"
    kind: task
    description: |
      加真实 dep 边：implement entry -dep approval（如有）；verify/review -dep
      Implement-terminal；entry-dep matrix 检查。
    acceptance: |
      entry-dep matrix 检查通过；plan/ready/blocked 与意图一致。
deps:
  - task: choose-template
    depends_on: scope
  - task: apply-template
    depends_on: choose-template
  - task: shape-graph
    depends_on: apply-template
  - task: phase
    depends_on: shape-graph
"#,
    ),
    (
        TemplateKind::Phase,
        "pdca",
        r#"tasks:
  - key: phase
    title: "pdca: plan → implement → audit → smoke → tidy"
    kind: task
    description: |
      PDCA 编排 phase：plan → implement → audit → smoke → tidy 顺序链。
    acceptance: |
      PDCA 五步全部完成；phase 在 tidy 完成后关闭。
  - key: plan
    title: "plan: define approach and steps"
    kind: task
    description: |
      定义方案与实施步骤（同层第一步）。
    acceptance: |
      方案与步骤已定义。
  - key: implement
    title: "implement: deliver the work item"
    kind: task
    description: |
      按方案实现：模块/测试/CLI 路径。
    acceptance: |
      工作项已实现。
  - key: audit
    title: "audit: check against plan and contract"
    kind: task
    description: |
      对照方案与可观察契约审查实现。
    acceptance: |
      审计完成：实现与方案/契约一致。
  - key: smoke
    title: "smoke: run and observe"
    kind: task
    description: |
      跑通主路径并记录观察结果。
    acceptance: |
      smoke 通过：命令 + 结果已记录。
  - key: tidy
    title: "tidy: clean obsolete artifacts"
    kind: task
    description: |
      清理脚手架/死代码/过期注释。
    acceptance: |
      清理完成。
deps:
  - task: implement
    depends_on: plan
  - task: audit
    depends_on: implement
  - task: smoke
    depends_on: audit
  - task: tidy
    depends_on: smoke
  - task: phase
    depends_on: tidy
"#,
    ),
    (
        TemplateKind::Phase,
        "plan",
        r#"tasks:
  - key: scope
    title: "scope: define topic in and out"
    kind: task
    description: |
      定义 In/Out/可观察结果/Non-goals。
    acceptance: |
      In/Out/可观察结果/Non-goals 已定义。
  - key: options
    title: "options: 2-3 approaches with tradeoffs"
    kind: task
    description: |
      2-3 个方案 + tradeoffs。
    acceptance: |
      2-3 个方案 + tradeoffs 已列出。
  - key: feasibility
    title: "feasibility: 8-point checklist"
    kind: task
    description: |
      8 项检查表（边界/回滚/可测/兼容/安全/性能/文档/工具链）。
    acceptance: |
      8 项检查表完成。
  - key: approach
    title: "approach: lock stack and change path"
    kind: task
    description: |
      锁定方案：技术栈 + 改动路径。
    acceptance: |
      方案锁定：技术栈 + 改动路径。
  - key: ready-summary
    title: "ready-summary: approvable execution summary"
    kind: task
    description: |
      可批准的执行摘要 + Work-items + Implement-terminal。
    acceptance: |
      执行摘要 + Work-items + Implement-terminal 已产出。
  - key: approval
    title: "approval: human gate"
    kind: task
    description: |
      人工审批门。
    acceptance: |
      人工审批通过（approval: yes）。
deps:
  - task: options
    depends_on: scope
  - task: feasibility
    depends_on: options
  - task: approach
    depends_on: feasibility
  - task: ready-summary
    depends_on: approach
  - task: approval
    depends_on: ready-summary
"#,
    ),
    (
        TemplateKind::Step,
        "implement",
        r#"tasks:
  - key: implement
    title: "implement: deliver the work item"
    kind: task
    description: |
      实现工作项：模块/测试/CLI 路径；至少一次 smoke。
    acceptance: |
      工作项已实现：模块/测试/CLI 路径 + smoke 通过。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "verify",
        r#"tasks:
  - key: verify
    title: "verify: check observable contract"
    kind: task
    description: |
      验证可观察契约：focused test + smoke + diff check。
    acceptance: |
      可观察契约验证通过。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "review",
        r#"tasks:
  - key: review
    title: "review: scope/CRG/code/simplicity"
    kind: task
    description: |
      审查：scope/CRG/code/simplicity；P0/P1 全部处置。
    acceptance: |
      审查完成：P0/P1 全部 disposition。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "document",
        r#"tasks:
  - key: document
    title: "document: sync design/manual"
    kind: task
    description: |
      同步设计文档/功能文档/手册。
    acceptance: |
      文档已同步。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "tidy",
        r#"tasks:
  - key: tidy
    title: "tidy: clean obsolete artifacts"
    kind: task
    description: |
      清理脚手架/死代码/过期注释。
    acceptance: |
      脚手架/死代码已清理。
deps: []
"#,
    ),
    (
        TemplateKind::Phase,
        "lifecycle",
        r#"mandatory:
  - plan
  - implement
  - audit
  - smoke

tasks:
  - key: phase
    title: "lifecycle: full delivery lifecycle"
    kind: task
    description: |
      全生命周期编排：plan → issue → implement → audit → smoke → tidy → pr → review → close。
      mandatory phase（plan/implement/audit/smoke）必须完成；optional phase 按场景选用。
      review 末尾有"等待用户确认"step，agent 不得自动跳过。
    acceptance: |
      所有 mandatory phase 完成；optional phase 按需完成。

  - key: plan
    title: "plan: define approach and steps"
    kind: task
    description: |
      定义方案与实施步骤。包括：要做什么文件、什么接口、什么行为、验收标准。
      产出：方案文档 + step recipe 清单。
    acceptance: |
      方案与步骤已定义；每个 step 有具体 description + acceptance。

  - key: issue
    title: "issue: generate spec + create GitHub issue"
    kind: task
    description: |
      生成 issue spec 正文（oi spec new issue）；通过 gh-gate 创建 GitHub issue；
      校验 issue 层级与正文符合 spec 规则。
    acceptance: |
      GitHub issue 已创建；spec check 通过。

  - key: implement
    title: "implement: deliver the work item"
    kind: task
    description: |
      按方案实现：创建隔离 worktree、编写代码、编写测试。
      具体文件/函数/行为由 plan phase 的 step recipe 定义。
    acceptance: |
      工作项已实现；主路径可运行。

  - key: audit
    title: "audit: check against plan and contract"
    kind: task
    description: |
      对照方案与可观察契约审查实现：语法检查、一致性核对、依赖边界。
    acceptance: |
      审计完成：实现与方案/契约一致。

  - key: smoke
    title: "smoke: run and observe"
    kind: task
    description: |
      跑通主路径并记录观察结果：合法输入 + 非法输入 + 边界。
    acceptance: |
      smoke 通过：命令 + 结果已记录。

  - key: tidy
    title: "tidy: clean obsolete artifacts"
    kind: task
    description: |
      清理脚手架/死代码/过期注释/调试输出。
    acceptance: |
      清理完成。

  - key: pr
    title: "pr: generate spec + create PR"
    kind: task
    description: |
      生成 PR spec 正文（oi pr render + oi spec new pr）；通过 gh-gate 创建 PR；
      校验 PR 关联与规范。
    acceptance: |
      PR 已创建；spec check 通过。

  - key: review
    title: "review: CRG + ocr + user confirmation"
    kind: task
    description: |
      运行 CRG 与 ocr 审查；回写审查结果到 PR；等待用户确认审查结果。
      agent 不得自动跳过用户确认门。
    acceptance: |
      CRG + ocr 审查完成；用户已确认。

  - key: close
    title: "close: gate merge + close issues"
    kind: task
    description: |
      通过 gate 合并 PR；勾选 sub issue Done when；关闭 sub issue 与 epic。
    acceptance: |
      PR 已合并；sub issue 与 epic 已关闭。

deps:
  - task: issue
    depends_on: plan
  - task: implement
    depends_on: issue
  - task: audit
    depends_on: implement
  - task: smoke
    depends_on: audit
  - task: tidy
    depends_on: smoke
  - task: pr
    depends_on: tidy
  - task: review
    depends_on: pr
  - task: close
    depends_on: review
  - task: phase
    depends_on: close
"#,
    ),
];

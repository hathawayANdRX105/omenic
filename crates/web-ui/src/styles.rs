//! 全仓统一**语义样式常量**：把 rsx 里反复手抄的 Tailwind/dsh class 串收敛到单一来源。
//!
//! 这些常量是视觉 token，不是逻辑（对照 ferrite `ui-components/styles.rs`）。
//! 页面/组件只引用常量，不在 `rsx!` 里写重复长串；改一处即可全仓生效。
//!
//! 颜色/字号已对齐 ui-kit shadcn 语义 token：`role-*` 文字角色（`assets/theme.css`
//! 的 `@utility`）+ `bg-card`/`bg-secondary`/`text-muted-foreground`/`border-border`…
//! 语义 token。**kit 无对应槽的 dsh 项**（`bg-dim`/`bg-codeblock`/`border-b3`/
//! `border-binv`/`shadow-lv*`/`bg-chip-danger`/brand 家族）保留 dsh 类名并入残余清单。

// ===========================================================================
// ① 表面（背景）——按语义引用，不按色板名
// ===========================================================================

/// 页面/根底：`bg-background`（kit `--background`，全黑统一）。
pub const S_PAGE: &str = "bg-background";

/// 面板/卡底：`bg-card`（kit `--card`，最常用的内容卡面）。
pub const S_PANEL: &str = "bg-card";

/// 凸起面：`bg-secondary`（kit `--secondary`，卡内嵌套块、任务看板面）。
pub const S_RAISED: &str = "bg-secondary";

/// 最深面：`bg-secondary-hover`（kit `--secondary-hover`，进度条轨道、选中底）。
pub const S_DEEPEST: &str = "bg-secondary-hover";

/// 侧栏底：`bg-background`（dsh 桥把 `--color-sidebar` 指到 kit `--background`，
/// 全黑统一——批注轮 #5；此处直接用语义类名）。
pub const S_SIDEBAR: &str = "bg-background";

/// 输入卡底：`bg-background`（composer 胶囊；kit 无 input 面槽，composer 浮在
/// 黑底上）。
pub const S_INPUT: &str = "bg-background";

/// 弹层/菜单底：`bg-popover`（kit `--popover`）。
pub const S_MENU: &str = "bg-popover";

/// 代码块底：`bg-codeblock`（dsh `#1b1b1c`，kit 无代码块面槽——残余，待补槽）。
pub const S_CODEBLOCK: &str = "bg-codeblock";

/// 选中条：`bg-secondary-hover`（dsh `--color-selector` = layer-3 值；kit 侧近似）。
pub const S_SELECTOR: &str = "bg-secondary-hover";

/// 用户气泡底：`bg-secondary`（dsh `--color-bubble` = layer-2 值；kit 侧近似）。
pub const S_BUBBLE: &str = "bg-secondary";

/// 交互 hover 底：`bg-accent`（kit `--accent` 白 5%；dsh `--color-ihover` 白 8%
/// 的 kit 近似；`hover:` 语境用 `hover:bg-muted`）。
pub const S_HOVER: &str = "bg-accent";

/// 交互 active 底：`bg-secondary-hover`（kit 最深实面；dsh `--color-iactive`
/// 白 14% 的 kit 近似）。
pub const S_ACTIVE: &str = "bg-secondary-hover";

/// 弱化圆点：`bg-dim`（dsh `#434546`，kit 无 dim 槽——残余，Idle 状态点）。
pub const S_DIM: &str = "bg-dim";

/// 强调面：`bg-brand`（kymido 品牌蓝，同 ferrite tavern 处理——残余保留）。
pub const S_BRAND: &str = "bg-brand";

// ===========================================================================
// ② 描边
// ===========================================================================

/// 一级描边 `border-border`（kit `--border`；最常用分隔线/卡边）。
pub const B_1: &str = "border-border";

/// 二级描边 `border-border`（kit `--border`；悬停强调的边）。
pub const B_2: &str = "border-border";

/// 三级描边 `border-b3`（focus-within 提亮的边）。kit 无 strong 描边槽
/// （白 16%），保留 dsh 槽——残余。
pub const B_3: &str = "border-b3";

/// 反相描边 `border-binv`（弹层/面板的更弱边，dsh 白 8% legacy 别名，值同
/// kit `--border`）——残余，kit 无对应命名。
pub const B_INV: &str = "border-binv";

/// 强调描边 `border-brand`（kymido 品牌蓝——残余保留）。
pub const B_BRAND: &str = "border-brand";

// ===========================================================================
// ③ 文字色（按语义，不按色号）
// ===========================================================================

/// 主文字 `text-foreground`（最亮，标题/正文主色）。
pub const C_TEXT: &str = "text-foreground";

/// 次级文字 `text-muted-foreground`。
pub const C_TEXT2: &str = "text-muted-foreground";

/// 弱化文字 `text-muted-foreground`（说明/辅助）。
pub const C_MUTED: &str = "text-muted-foreground";

/// 最弱 `text-muted-foreground`（脚注/小注；kit 把 dsh label-2/-3/caption
/// 三档并档到一档 muted）。
pub const C_CAPTION: &str = "text-muted-foreground";

/// 强调文字 `text-brand`（品牌蓝——残余保留）。
pub const C_BRAND: &str = "text-brand";

/// 亮强调 `text-brand-300`（工具 kind / 计划评审标；品牌蓝——残余保留）。
pub const C_BRAND_L: &str = "text-brand-300";

/// 危险/错误 `text-destructive`（kit `--destructive`）。
pub const C_DANGER: &str = "text-destructive";

/// 成功 `text-success-foreground`（kit `--success-foreground`）。
pub const C_SUCCESS: &str = "text-success-foreground";

/// 警告 `text-warning-foreground`（kit `--warning-foreground`）。
pub const C_WARN: &str = "text-warning-foreground";

/// 成功 chip 底 `bg-success`（kit `--success` = rgb(30 55 40) ≈ dsh
/// `--color-chip-success` #233c2c）。
pub const S_CHIP_SUCCESS: &str = "bg-success";

/// 危险 chip 底 `bg-chip-danger`（dsh 暗红 #570c0c vs kit 明红 `--destructive`
/// #ff4d5b，语义不合——残余保留）。
pub const S_CHIP_DANGER: &str = "bg-chip-danger";

/// 警告 chip 底 `bg-warning`（kit `--warning`）。
pub const S_CHIP_WARN: &str = "bg-warning";

/// 强调 chip 底 `bg-chip-brand`（品牌蓝底——残余保留）。
pub const S_CHIP_BRAND: &str = "bg-chip-brand";

// ===========================================================================
// ④ 阴影
// ===========================================================================

/// 中卡投影 `shadow-lv2`（composer/问题卡/看板）。kit 无 dsh 阴影阶梯
/// （lv1/lv2/lv3），保留 dsh 槽——残余。
pub const SHADOW_CARD: &str = "shadow-lv2";

/// 大浮层投影 `shadow-lv3`（悬停信息面板）——残余，同上。
pub const SHADOW_POP: &str = "shadow-lv3";

// ===========================================================================
// ⑤ 字号行高（text scale，常用组合）
// ===========================================================================

/// 14px 正文行（组件内最常用的尺寸）：`role-hint`。
pub const TYPE_BODY: &str = "role-hint";

/// 14px medium 标题：`role-hint` + `font-medium`。
pub const TYPE_TITLE: &str = "role-hint font-medium";

/// 13px 次级行：`role-caption`。
pub const TYPE_DESC2: &str = "role-caption";

/// 12px 弱化行（时间戳/说明）：`role-caption`。
pub const TYPE_SMALL: &str = "role-caption";

/// 12px 小注行（caption 高密度）：`role-caption`。
pub const TYPE_CAPTION: &str = "role-caption";

/// 11px 微注（mono 计数/编号）：`role-label`（kit 最小常用档）。
pub const TYPE_TINY: &str = "role-label";

/// 10px 极微（chip/大写标签）：`role-label`；带 `uppercase` 的场景用
/// `role-overline`。
pub const TYPE_MICRO: &str = "role-label";

// ===========================================================================
// ⑥ 组合件外壳（>=2 处同构的语义壳，B2.5 收敛点）
// ===========================================================================

/// 内容卡壳：`rounded border + 卡底`（设置卡/任务卡/统计卡共用）。
pub const CARD: &str = "rounded-2xl border border-border bg-card";

/// hover 可点卡壳（任务卡/列表行）：未选中态。
pub const CARD_ROW: &str =
    "rounded-[10px] border border-border bg-card px-3 py-2.5 cursor-pointer transition-colors";

/// hover 可点卡壳：选中态（brand 边）。
pub const CARD_ROW_ON: &str =
    "rounded-[10px] border border-brand bg-card px-3 py-2.5 cursor-pointer transition-colors";

/// 侧栏激活导航行。
pub const NAV_ON: &str = "h-10 px-3 rounded-xl flex items-center gap-2.5 role-hint text-foreground bg-accent cursor-pointer transition-colors border-none w-full";

/// 侧栏未激活导航行。
pub const NAV_OFF: &str = "h-10 px-3 rounded-xl flex items-center gap-2.5 role-hint hover:bg-muted hover:text-foreground cursor-pointer transition-colors border-none w-full bg-transparent";

/// 表单输入框（设置页 text input 外壳）。
pub const INPUT_FIELD: &str = "w-full h-9 rounded-[10px] bg-secondary border border-border px-3 role-hint text-foreground outline-none transition-colors focus:border-brand placeholder:text-muted-foreground";

/// 提示条成功壳（连接成功 / 保存成功）。
pub const BAR_OK: &str =
    "px-4 py-2.5 rounded-[10px] role-caption bg-success text-success-foreground";

/// 提示条危险壳（连接失败 / 保存失败）。
pub const BAR_ERR: &str = "px-4 py-2.5 rounded-[10px] role-caption bg-chip-danger text-destructive";

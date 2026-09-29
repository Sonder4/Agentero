# 通用决策层（Decision Layer）设计草案

> 状态：设计草案 / 待实现  
> 相关功能：jEV 智能高亮、PDF 选区动作、文件树点击、论文自动标签等

## 1. 背景与问题

jEV（TypeSafe System One）本质上不是一个“PDF 高亮工具”，而是一个**可编程的语义决策 API**：给定一段 `state` 和一组 `questions`，返回类型化的 `answers`。

当前实现把 jEV 硬编码在 `features/jev/` 里，只服务于“PDF 智能高亮”这一个功能。随着产品演进，越来越多的场景需要语义判断：

- PDF 中选中文本后，最该显示/执行哪个操作？
- 点击一个按钮时，根据当前上下文应该进入哪个 workflow？
- 论文入库时自动推荐什么标签？
- Agent 下一步该调用哪个 tool？

与此同时，软件里已经存在大量**确定性规则**（if-else、类型检查、路径匹配、配置开关）。这些规则不需要 AI，也不适合用 jEV：

- 文件树点击：根据路径/类型决定打开 Library、Paper 还是 Note。
- 关闭标签页：根据焦点和标签状态决定行为。
- 打开 paper 时是否自动开 NOTES：由 `autoOpenPaperNotes` 配置决定。

因此，我们需要一个**统一的决策层**，让业务按需选择：

- 纯规则（Rule）
- 纯 jEV（System One）
- jEV + 规则兜底（jEV 置信度低时 fallback）

而不是让 jEV 与现有 hook 架构割裂，或者把所有决策都交给 AI。

## 2. 设计目标

1. **统一抽象**：所有“需要二选一/多选一/打分”的地方都走决策层。
2. **provider 可插拔**：规则引擎、jEV、未来可能加入的 LLM 决策都是平等的 provider。
3. **业务显式配置**：每个决策场景在注册时就明确使用哪种 routing，不搞全局自动路由。
4. **规则没有置信度**：规则是确定性分支，命中即执行；jEV 是概率性判断，有 confidence。
5. **渐进式迁移**：不需要一次性改造所有 hook，从 1–2 个语义决策点开始接入。
6. **可解释**：每个决策结果标注由哪个 provider 做出，便于调试和 fallback。

## 3. 核心概念

| 概念 | 说明 |
|---|---|
| **Decision** | 一次判断任务，例如“PDF 选中文本的意图是什么”。 |
| **State** | 决策所需的上下文，例如 `{ selectedText, page, source: "pdf" }`。 |
| **Question** | 向 jEV 提出的问题，类型可以是 `score` / `choice` / `noul`。 |
| **Rule** | 确定性规则函数：`state -> Option<action>`。 |
| **Provider** | 具体执行决策的模块：RuleProvider、JevProvider 等。 |
| **Routing** | 决策场景的路由配置：主 provider、fallback provider、fallback 阈值。 |
| **ProviderConfig** | provider 专属配置，例如 jEV 的 questions、LLM 的 prompt。 |
| **Outcome** | 决策结果，包含 action、provider 来源、置信度（如有）。 |

## 4. 架构 overview

```
业务 hook / 组件
       │
       ▼
┌─────────────────────────────────────┐
│        DecisionRegistry             │  每个业务注册自己的 decision schema
│   (id → schema + providerConfigs)   │
└───────────────┬─────────────────────┘
                │
                ▼
┌─────────────────────────────────────┐
│         DecisionEngine              │  根据 schema.routing 调用对应 provider
│      (route → execute → outcome)    │
└───────────────┬─────────────────────┘
                │
    ┌───────────┼───────────┐
    ▼           ▼           ▼
┌────────┐  ┌────────┐  ┌────────┐
│  Rule  │  │  jEV   │  │  LLM   │  （未来可扩展）
│Provider│  │Provider│  │Provider│
└────────┘  └────────┘  └────────┘
```

## 5. 后端设计（Rust / agentero-core）

### 5.1 DecisionProvider trait

```rust
#[async_trait]
pub trait DecisionProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn decide(
        &self,
        request: DecisionRequest,
    ) -> Result<DecisionOutcome, AppError>;
}
```

### 5.2 DecisionRequest / DecisionOutcome

```rust
pub struct DecisionRequest {
    pub decision_id: String,
    pub state: serde_json::Value,
}

pub struct DecisionOutcome {
    pub action: serde_json::Value,
    pub provider: String,
    pub confidence: Option<f64>, // 仅 jEV/LLM 有值
}
```

### 5.3 决策路由（DecisionRouting）

`DecisionRouting` 描述一次决策应该由哪个 provider 执行、如何 fallback，而不是把 provider 名字硬编码进 schema 字段。

```rust
pub struct DecisionRouting {
    /// 主 provider 名称，例如 "rule"、"jev"、"llm"
    pub primary: String,

    /// fallback provider 名称（可选）
    pub fallback: Option<String>,

    /// 触发 fallback 的置信度阈值（仅当主 provider 返回 confidence 时生效）
    pub fallback_threshold: Option<f64>,
}
```

常见组合：

| primary | fallback | fallback_threshold | 语义 |
|---|---|---|---|
| `"rule"` | `None` | `None` | 纯规则 |
| `"jev"` | `None` | `None` | 纯 jEV |
| `"jev"` | `Some("rule")` | `Some(0.75)` | jEV 置信度低时回退规则 |
| `"rule"` | `Some("jev")` | `None` | 规则未命中时让 jEV 兜底 |

### 5.4 DecisionSchema

```rust
use std::collections::HashMap;

pub struct DecisionSchema {
    pub id: String,
    pub description: String,

    /// 通用路由：指定主 provider、fallback provider 与阈值
    pub routing: DecisionRouting,

    /// 规则链，保留在顶层供 RuleProvider 使用
    pub rules: Vec<Box<dyn DecisionRule>>,

    /// provider 专属配置，key 为 provider 名称
    /// 例如 `"jev" -> JevProviderConfig`，未来可扩展 `"llm" -> LlmProviderConfig`
    pub provider_configs: HashMap<String, Box<dyn ProviderConfig>>,

    pub default_action: Option<serde_json::Value>,
}

pub trait DecisionRule: Send + Sync {
    fn apply(&self, state: &serde_json::Value) -> Option<serde_json::Value>;
}

/// provider 专属配置trait；不同 provider 自己决定如何组织请求
pub trait ProviderConfig: Send + Sync {
    fn provider_name(&self) -> &'static str;

    /// 把当前 state 构造成该 provider 能理解的请求
    fn build_request(&self, state: &serde_json::Value) -> ProviderRequest;
}

/// provider 请求的通用信封；具体 body 由 provider 自己定义
pub struct ProviderRequest {
    pub provider: String,
    pub body: serde_json::Value,
}
```

### 5.5 为什么用 `provider_configs` 而不是 `jev_questions`

旧设计把 `jev_questions` 直接挂在 `DecisionSchema` 上，导致 schema 与 jEV 强耦合：

- 每新增一种 provider（如本地 LLM、云端 LLM）就要在 schema 里加一个新字段。
- 业务注册时必须 import jEV 相关的类型，即使当前 decision 只使用规则。
- 无法表达“同一个 decision 在不同 provider 下的不同请求模板”。

`provider_configs: HashMap<String, Box<dyn ProviderConfig>>` 解决这些问题：

1. **provider 与 schema 解耦**：schema 只描述“用什么 provider 做决策”，provider 自己描述“我需要什么配置”。
2. **按需配置**：纯规则 decision 的 `provider_configs` 可以为空；jEV decision 放 `"jev"` 配置；未来 LLM decision 放 `"llm"` 配置。
3. **统一扩展**：新增 provider 只需实现 `ProviderConfig` + `DecisionProvider`，不需要改 `DecisionSchema` 定义。
4. **多 provider 请求模板共存**：同一个 decision 可以同时对 jEV 和 LLM 准备不同请求，便于 A/B 测试或渐进迁移。

### 5.6 JevProvider 实现要点

- 复用现有 `reqwest::Client` 连接池。
- 支持批量请求（batch + 并发），供高亮这类大量 questions 的场景使用。
- 解析 jEV answer 时提取 `confidence`。
- `score` 类型回答可通过 probability distribution 计算置信度。

### 5.7 RuleProvider 实现要点

- 规则按注册顺序执行，返回 `Some` 即停止。
- 不计算置信度。
- 规则函数保持轻量，只做确定性判断。

## 6. 前端设计（React）

### 6.1 useDecision hook

```ts
// src/hooks/use-decision.ts

export function useDecision() {
  const decide = useCallback(
    async <T extends Record<string, unknown>>(
      decisionId: string,
      state: T,
    ): Promise<DecisionOutcome> => {
      return callApiResult(() =>
        commands.decide({ decisionId, state }),
      );
    },
    [],
  );

  return { decide };
}
```

### 6.2 业务 hook 使用示例

```ts
// PDF 选中文本后智能判断意图
function usePdfSelectionActions(...) {
  const { decide } = useDecision();

  const handleSmartMenu = async () => {
    const menu = selectionMenuRef.current;
    if (!menu) return;

    const outcome = await decide("pdf.selection.intent", {
      selectedText: menu.anchor.quote,
      source: "pdf",
      page: menu.anchor.page,
    });

    switch (outcome.action) {
      case "ask": return handleMenuAsk();
      case "translate": return handleMenuTranslate();
      case "highlight": return handleHighlight("yellow");
      case "addToChat": return handleMenuAddToChat();
      case "ignore": return closeSelectionMenu();
    }
  };
}
```

### 6.3 纯规则场景

```ts
// 文件树点击完全走规则，不需要异步
function selectFileNode(node: FileNode) {
  const outcome = decideSync("file-tree.click", { node });
  executeFileTreeAction(outcome.action);
}
```

规则决策可以同步执行，不需要发请求到后端。

## 7. 注册表示例

### 7.1 文件树点击（纯规则）

```ts
registerDecision({
  id: "file-tree.click",
  description: "Decide what to do when a file-tree node is clicked",
  routing: { primary: "rule" },
  rules: [
    (state) => state.node.isLibrary ? { action: "select-library" } : null,
    (state) => state.node.isTrash ? { action: "select-trash" } : null,
    (state) => state.node.isPlaza ? { action: "open-plaza" } : null,
    (state) => state.node.isPaperDir ? { action: "open-paper" } : null,
    (state) => state.node.kind === "directory" ? { action: "scope-library" } : null,
    (state) => state.node.kind === "file" ? { action: "open-file" } : null,
  ],
  providerConfigs: {},
  defaultAction: { action: "noop" },
});
```

### 7.2 PDF 选中文本意图（jEV + 规则兜底）

```ts
registerDecision({
  id: "pdf.selection.intent",
  description: "Decide the most likely intent when user selects text in PDF",
  routing: {
    primary: "jev",
    fallback: "rule",
    fallbackThreshold: 0.75,
  },
  rules: [
    (state) => !state.selectedText?.trim() ? { action: "ignore" } : null,
    (state) => /\[\d+\]/.test(state.selectedText) ? { action: "open-citation" } : null,
  ],
  providerConfigs: {
    jev: {
      buildRequest: (state) => ({
        questions: [
          {
            type: "choice",
            id: "intent",
            instructions: `User selected "${state.selectedText}" in a PDF. What is the most likely intent?`,
            options: {
              ask: "They want to ask a question about this content",
              translate: "They want to translate it",
              highlight: "They want to highlight it",
              addToChat: "They want to use it as context in chat",
              ignore: "No clear intent",
            },
          },
        ],
      }),
    },
  },
  defaultAction: { action: "ignore" },
});
```

### 7.3 论文自动标签（纯 jEV）

```ts
registerDecision({
  id: "paper.auto-tag",
  description: "Suggest tags for a paper based on title and abstract",
  routing: { primary: "jev" },
  rules: [],
  providerConfigs: {
    jev: {
      buildRequest: (state) => ({
        questions: [
          {
            type: "choice",
            id: "primary-tag",
            instructions: "What is the primary research area of this paper?",
            options: {
              nlp: "Natural language processing",
              cv: "Computer vision",
              rl: "Reinforcement learning",
              systems: "Systems and infrastructure",
              theory: "Theory",
            },
          },
        ],
      }),
    },
  },
});
```

## 8. 现有 jEV 高亮的迁移

当前 `features/jev/service.rs` 里的 PDF 智能高亮逻辑，应被改造成第一个基于 DecisionEngine 的 decision：

```ts
registerDecision({
  id: "pdf.smart-highlight",
  description: "Score sentences in a PDF to decide which to highlight",
  routing: { primary: "jev" },
  rules: [],
  providerConfigs: {
    jev: {
      buildRequest: (state) => ({
        questions: buildHighlightQuestions(state.sentences),
      }),
    },
  },
});
```

后端 `jev_suggest_highlights_with_progress` 不再直接调 HTTP，而是调用 `DecisionEngine::decide_batch()`。

## 9. 实施路径

建议分阶段推进，避免一次性大改：

| 阶段 | 内容 | 验证标准 |
|---|---|---|
| 1 | 在 `agentero-core` 建立 `DecisionEngine` + `JevProvider`，把现有 jEV 高亮接入 | jEV 高亮功能不变，代码结构更干净 |
| 2 | 实现 `RuleProvider` 和 `DecisionRegistry` | 文件树点击等纯规则场景可注册 |
| 3 | 前端加 `useDecision` hook | React 组件能一行代码发起决策 |
| 4 | 接入第一个语义决策用例（如 PDF 选中文本意图） | 选中引用直接打开引用，选中普通文本经 jEV 判断 |
| 5 | 按需扩展更多 decision schema | 论文标签推荐、按钮 workflow 路由等 |
| 6 | 评估是否需要 LLM Provider | 仅当 jEV 处理不了的复杂推理场景 |

## 10. 边界与注意事项

1. **不是所有决策都需要决策层**
   - 文件保存、标签页关闭、简单 UI 状态切换仍直接由 hook/store 处理。
   - 只有“需要基于内容语义做判断”的场景才注册 decision。

2. **规则优先于 jEV**
   - 能用规则确定的，不要用 AI。
   - jEV 有 token 成本和延迟。

3. **用户可覆盖**
   - AI 决策给出的是“默认建议”，UI 上应允许用户选择其他操作。
   - 例如 PDF 智能菜单可以显示 jEV 推荐的 top action，但保留其他选项。

4. **决策结果要可解释**
   - Outcome 里必须带 `provider` 和 `confidence`。
   - 调试面板可以展示“为什么这个决策是这样”。

5. **避免过度抽象**
   - 不要为了统一而统一。如果一个判断永远只有一条 if 分支，不需要注册成 decision。

## 11. 开放问题

- `RuleProvider` 在前端还是后端执行？文件树点击这种纯前端状态的场景，同步规则在前端执行更轻量。
- jEV 的批量请求是否需要保留独立路径，还是统一走 `decide_batch`？
- 是否需要决策结果缓存？例如同一篇论文的高亮结果可以缓存。
- 是否允许用户为每个 decision 单独开关或选择 provider？

---

## 12. 与现有架构的关系

- **不替换 hook 架构**：hook 仍然存在，只是在语义判断点插入 `decide()` 调用。
- **不替换 zustand**：决策结果可以写入 store，但决策层本身是无状态的。
- **复用 JobCenter**：批量 jEV 决策（如高亮）可走后台 job；单次轻量决策直接走 async command。

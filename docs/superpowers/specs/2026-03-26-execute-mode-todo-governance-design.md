# Execute Mode Todo Governance Design

## Summary

Strengthen the plan-to-execution workflow so that:

- `Plan Mode` must produce a canonical `tasks.csv` todo list before plan acceptance.
- Plan review runs only in `Plan Mode`.
- Once a plan has been accepted for the current thread, `tasks.csv` becomes the absolute source of truth for governed execution.
- `Execute` mode does not reinterpret or review the plan. Instead, the server selects the current executable plan row and tells the model exactly which row it is allowed to execute.
- During governed execution, `update_plan` may only advance the status of the server-selected row. It may not replace the plan, add rows, or mutate plan metadata.
- Execution progress must be updated in real time as work starts and completes.

This design keeps planning flexible while making execution deterministic once a plan exists.

## Goals

- Make `tasks.csv` the canonical todo-list artifact produced by `Plan Mode`.
- Ensure plan review remains a planning-only concern.
- Treat an accepted thread plan as immutable truth during governed execution.
- Move "what should execute now" from model judgment to server judgment.
- Require real-time progress updates during governed execution.
- Preserve current free-form execution behavior when no active plan exists for the thread.

## Non-Goals

- Renaming `tasks.csv` to `plan.csv`.
- Allowing `Execute` mode to revise, append to, or repair a plan.
- Re-running plan review in `Execute` mode.
- Injecting the full plan body into `Execute`-mode developer instructions.
- Blocking plan-external work entirely. Plan-external work remains possible, but it must not mutate the plan.

## Current State

The repository already has the core building blocks for plan-backed execution:

- `Plan Mode` and `Auto Plan` use a file-first workspace with:
  - `requirements.md`
  - `design.md`
  - `tasks.csv`
  - `tasks.md`
- `tasks.csv` is already validated as canonical structured plan data.
- Accepted plans are persisted as active thread plans in state.
- `update_plan` already syncs active plan status into the runtime workspace.
- `Plan Mode` already has a hidden reviewer flow.

However, the execution path is still too permissive:

- `Execute` mode is not wired in as a first-class collaboration preset.
- `Execute` mode does not receive a server-chosen executable row.
- `update_plan` still allows shapes that are too broad for governed execution.
- The model can still implicitly decide what to execute next instead of being constrained to a server-selected row.

## Product Rules

### Planning Rules

- `Plan Mode` must continue to produce `tasks.csv` as the canonical todo-list artifact.
- Plan review runs only during plan production and acceptance.
- Once the plan is accepted for the thread, the accepted active plan is the immutable execution truth for governed execution.

### Execution Rules

- If the current thread has no active plan, `Execute` mode remains free-form.
- If the current thread has an active plan, `Execute` mode becomes governed by that plan.
- Governed `Execute` mode must not review the plan, reinterpret the plan, or modify the plan.
- The server must choose the current executable row and pass that row to the model.
- The model must not decide which row to execute by reading the plan and making its own scheduling decision.

### Progress Rules

- Governed execution must update progress in real time.
- Starting work on the current row requires a status update to `in_progress`.
- Finishing work on the current row requires a status update to `completed`.
- Plan-external work may be described in `explanation` and in the final response, but must not appear as plan mutation.

## High-Level Architecture

The strengthened flow has three stages:

1. `Plan Mode` produces and reviews the canonical plan.
2. The accepted plan is persisted as the active thread plan and synced to the plan workspace.
3. Governed `Execute` mode asks the server for the current executable row, then only advances that row through a tightly validated `update_plan` path.

This moves the critical execution decision from prompt-following behavior into runtime enforcement:

- the prompt tells the model where the plan workspace lives and what rules it must obey;
- the server tells the model which row is currently executable;
- the `update_plan` handler enforces that only that row may move forward.

## Execute-Mode Governance Model

### Ungoverned Execute Mode

If there is no active thread plan for the current thread:

- `Execute` mode behaves like normal free-form execution;
- no todo-list gate is applied;
- existing `update_plan` behavior remains available.

### Governed Execute Mode

If there is an active thread plan for the current thread:

- the active plan is treated as absolute truth;
- no review step runs in `Execute` mode;
- the model receives:
  - the plan workspace path;
  - the `tasks.csv` path;
  - the `tasks.md` path;
  - the server-selected current executable row;
  - explicit compliance requirements;
- the model does not receive the full active plan as injected prompt content;
- `update_plan` becomes a restricted status-transition tool instead of a general plan-editing tool.

## Server-Selected Executable Target

The server computes a single `CurrentExecutableTarget` from the active plan rows.

### Selection Algorithm

Given canonical active-plan rows in row order:

1. If there is an `in_progress` row, select that row.
2. Otherwise, scan `pending` rows in ascending `row_index` order.
3. Select the first `pending` row whose `depends_on` rows are all `completed`.
4. If all rows are `completed`, report that no executable target remains because the plan is complete.
5. If pending rows remain but none are dependency-ready, report that no executable target exists because execution is blocked by the plan state.

### Why a Single Row

A single-row target keeps governed execution deterministic:

- the model does not schedule work;
- the server does not need to support parallel governed execution semantics in this change;
- `update_plan` validation can stay simple and strict.

## Prompt Injection Contract for Governed Execute Mode

When governed execution is active, the server injects a compact instruction block that contains:

- the plan workspace root path;
- the canonical `tasks.csv` path;
- the derived `tasks.md` path;
- the current executable row fields:
  - `id`
  - `step`
  - `path`
  - `details`
  - `acceptance`
- the execute-mode compliance rules:
  - the active plan is absolute truth;
  - do not review the plan;
  - do not modify the plan;
  - only execute the row selected by the server;
  - update progress in real time;
  - record plan-external work only in explanations/final reporting.

The injected block must not include the full plan body or a full dump of all rows.

## Execute-Mode `update_plan` Validation

When the current turn is in `ModeKind::Execute` and an active plan exists, `update_plan` becomes a gated state-transition endpoint.

### Allowed Request Shape

The request must:

- include exactly one plan item;
- include a non-empty `id`;
- target the server-selected current executable row;
- avoid changing plan structure or metadata.

### Allowed Transitions

For the current executable row only:

- `pending -> in_progress`
- `in_progress -> completed`
- `in_progress -> in_progress`
- `completed -> completed`

The idempotent transitions support retries and duplicated status syncs without widening the mutation surface.

### Rejected Mutations

The handler must reject all of the following in governed execute mode:

- replacing the whole plan;
- omitting `id`;
- updating multiple rows in one request;
- targeting any row other than the current executable row;
- adding rows;
- deleting rows;
- changing `step`;
- changing `path`;
- changing `details`;
- changing `inputs`;
- changing `outputs`;
- changing `depends_on`;
- changing `acceptance`;
- `pending -> completed`;
- `completed -> in_progress`;
- any transition that conflicts with the server-selected executable row.

### Explanation Handling

`explanation` remains allowed during governed execution and is the correct place to describe:

- current progress;
- blockers;
- plan-external work;
- assumptions made while completing the current row.

`explanation` must not be treated as plan mutation.

## Plan-External Work

Plan-external work is not forbidden, but it is not allowed to mutate the accepted plan in `Execute` mode.

If the model performs work that was not represented in the accepted plan:

- the work may proceed;
- the active plan must remain unchanged except for legal status updates on the current row;
- the work must be surfaced in `explanation`;
- the final response must note that the work was outside the accepted plan.

This preserves the "plan is truth" contract while still letting execution complete responsibly.

## Module and File Changes

### 1. Wire Execute Mode as a Real Preset

Modify [`codex-rs/core/src/models_manager/collaboration_mode_presets.rs`](/home/ubuntu/realmx/codex-rs/core/src/models_manager/collaboration_mode_presets.rs):

- add an `execute` collaboration template constant;
- add an `execute_preset(...)`;
- include the execute preset where hidden/internal mode presets are assembled.

The preset does not need to become TUI-visible in this change. It does need to become a first-class preset so it can supply developer instructions consistently.

### 2. Tighten the Execute Template

Modify [`codex-rs/core/templates/collaboration_mode/execute.md`](/home/ubuntu/realmx/codex-rs/core/templates/collaboration_mode/execute.md):

- remove any implication that execute-time plan review occurs;
- explicitly state that accepted plans are absolute truth during governed execution;
- instruct the model to read the provided plan file path before acting;
- instruct the model that the server-selected row is the only allowed governed target;
- instruct the model to use `update_plan` for real-time progress only;
- instruct the model that plan-external work belongs in explanations and final reporting only.

### 3. Add a Focused Execute-Plan Guard Module

Add [`codex-rs/core/src/execute_plan_guard.rs`](/home/ubuntu/realmx/codex-rs/core/src/execute_plan_guard.rs).

This module owns governed execution logic so that it does not expand the already large `codex.rs` and `plan.rs` files further.

Suggested responsibilities:

- load active plan context for execute-time gating;
- compute the current executable row;
- build compact governed-execute developer instructions;
- validate governed execute-time `update_plan` requests.

Suggested API shape:

- `resolve_execute_plan_context(...)`
- `resolve_current_executable_target(...)`
- `build_execute_plan_guard_instructions(...)`
- `validate_execute_mode_plan_update(...)`

The exact names can vary, but the ownership should stay isolated in a dedicated module.

### 4. Inject Governed-Execute Instructions

Modify [`codex-rs/core/src/codex.rs`](/home/ubuntu/realmx/codex-rs/core/src/codex.rs):

- when assembling developer instructions for a turn in `ModeKind::Execute`;
- look up the current active thread plan;
- if no active plan exists, inject nothing extra;
- if an active plan exists, call the new guard module to:
  - compute the current executable target;
  - build the compact governed-execute instruction block;
- append that block as developer instructions.

This injection should contain only paths, compliance rules, and the server-selected row, not the entire plan.

### 5. Enforce Governed Status Transitions

Modify [`codex-rs/core/src/tools/handlers/plan.rs`](/home/ubuntu/realmx/codex-rs/core/src/tools/handlers/plan.rs):

- before executing the existing CSV-backed update flow;
- detect whether the turn is in `ModeKind::Execute`;
- detect whether an active plan exists for the thread;
- if both are true, delegate to the execute guard validator;
- reject invalid governed updates with explicit model-visible errors;
- accept valid single-row transitions and continue to reuse existing persistence/sync code.

This file should remain the integration layer. It should not inline the full executable-row selection algorithm.

### 6. State Layer

Prefer keeping transition-policy logic in core for this change.

[`codex-rs/state/src/runtime/thread_plans.rs`](/home/ubuntu/realmx/codex-rs/state/src/runtime/thread_plans.rs) should only be changed if a small helper materially improves correctness or reduces duplication. The initial implementation should avoid pushing collaboration-mode semantics into state unless testing reveals a strong need.

## Testing Strategy

### Execute Preset Tests

Update [`codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs`](/home/ubuntu/realmx/codex-rs/core/src/models_manager/collaboration_mode_presets_tests.rs):

- verify `Execute` has a real preset;
- verify the preset includes the execute template text;
- verify the execute template reflects the governed execution rules.

### Collaboration Instruction Tests

Update [`codex-rs/core/tests/suite/collaboration_instructions.rs`](/home/ubuntu/realmx/codex-rs/core/tests/suite/collaboration_instructions.rs):

- verify governed `Execute` mode injects:
  - plan workspace path;
  - `tasks.csv` path;
  - `tasks.md` path;
  - current executable row content;
- verify the injected content does not include the full plan body.

### Execute Guard Unit Tests

Add a focused unit-test module for [`codex-rs/core/src/execute_plan_guard.rs`](/home/ubuntu/realmx/codex-rs/core/src/execute_plan_guard.rs):

- select the existing `in_progress` row when present;
- otherwise select the first dependency-ready `pending` row;
- report completion when all rows are complete;
- report blocked state when pending rows remain but none are dependency-ready;
- validate allowed transitions;
- reject disallowed transitions.

### `update_plan` Handler Tests

Add or extend tests for [`codex-rs/core/src/tools/handlers/plan.rs`](/home/ubuntu/realmx/codex-rs/core/src/tools/handlers/plan.rs):

- allow `pending -> in_progress` on the current executable row;
- allow `in_progress -> completed` on the current executable row;
- reject `pending -> completed`;
- reject multi-row execute-mode updates;
- reject updates for non-current rows;
- reject updates without `id`;
- reject replacement-style updates during governed execution;
- verify free-form behavior is preserved when no active plan exists.

### Existing Rendering / Exec Tests

Retain and, if necessary, extend the existing plan-progress coverage:

- [`codex-rs/exec/tests/event_processor_with_json_output.rs`](/home/ubuntu/realmx/codex-rs/exec/tests/event_processor_with_json_output.rs)

The plan-progress event stream should continue to render correctly after governed execute-mode updates.

## Acceptance Criteria

This change is complete when all of the following are true:

1. `Plan Mode` still produces and accepts canonical `tasks.csv`.
2. Plan review still runs in `Plan Mode` only.
3. `Execute` mode becomes a real preset with dedicated instructions.
4. If no active plan exists, `Execute` mode remains free-form.
5. If an active plan exists, governed execute-mode instructions include:
   - plan workspace path;
   - `tasks.csv` path;
   - `tasks.md` path;
   - the server-selected current executable row.
6. Governed execute-mode instructions do not inject the full active plan body.
7. Governed execute-mode `update_plan` accepts only legal single-row status transitions for the current executable row.
8. Governed execute-mode `update_plan` rejects plan replacement, metadata mutation, row creation, row selection by the model, and multi-row updates.
9. Legal status changes continue to persist to the active thread plan and sync back to the runtime workspace.
10. Plan-external work can be reported, but cannot mutate `tasks.csv` during `Execute` mode.

## Risks and Mitigations

### Risk: Execute preset wiring changes hidden mode behavior

Mitigation:

- keep the preset internal if needed;
- add preset-level tests to verify the effective instructions.

### Risk: Prompt injection becomes too large

Mitigation:

- inject only paths, rules, and the single executable row;
- do not inject the full plan.

### Risk: Over-constraining `update_plan` breaks non-execute flows

Mitigation:

- gate all strict validation on both conditions:
  - `ModeKind::Execute`
  - active plan exists

### Risk: Plan state with invalid dependency topology yields no executable target

Mitigation:

- return a clear blocked-state instruction instead of silently picking a row;
- preserve plan immutability in `Execute` mode.

## Rollout Notes

This design intentionally tightens execution without changing the planning artifact shape. It should ship behind the existing collaboration-mode/runtime plan infrastructure and reuse the current CSV-backed active-plan persistence rather than introducing a second source of execution truth.

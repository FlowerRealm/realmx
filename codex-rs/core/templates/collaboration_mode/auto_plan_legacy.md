# Deprecated Historical Template

This file is kept only as historical reference. Runtime code no longer loads it.
The current public collaboration modes are `Default`, `Plan`, and `Ultra Work`.

# Historical Auto Plan (Non-interactive)

This file preserves the deprecated **Auto Plan** mode wording. Its job was to produce a high-quality, decision-complete implementation plan **without asking the user any questions**.

## Core rules (strict)

- Do **not** ask the user questions.
- Do **not** call `request_user_input` (it is unavailable in this mode).
- If something is unclear, **first** discover facts by reading/searching the local repo (code, configs, docs, tests).
- If it still cannot be determined from local context, make a **reasonable default assumption** and record it explicitly in the plan.

## Execution vs. mutation

You may do **non-mutating exploration** to refine the plan:
- Read/search files and docs
- Run builds/tests/linters that do not edit repo-tracked files (build artifacts are OK)

You must **not** do work that changes repo-tracked state:
- Editing/writing files
- Running formatters or codegen that rewrite tracked files
- Applying patches or migrations

## Output requirements

Emit exactly one `<proposed_plan>` block (and no additional plan blocks). The plan must be **decision complete** so another engineer/agent can implement it immediately without making choices.

The plan should be concise by default and include:
- A clear title
- Summary
- Key changes (APIs/interfaces/types) if applicable
- Test plan
- Assumptions (explicit defaults chosen)

Use the tags exactly as:

<proposed_plan>
...markdown...
</proposed_plan>

# Deprecated Historical Template

This file is kept only as historical reference. Runtime code no longer loads it.
The current public collaboration modes are `Default`, `Plan`, and `Ultra Work`.

# Historical Collaboration Style: Execute

This template captures the deprecated pre-`Ultra Work` execute-only behavior kept for reference.

## Plan governance

- Do not review the plan in the execution phase.
- If the server provides plan workspace paths, read provided tasks.csv path before acting.
- If the server provides a current executable row, only execute the server-selected row.
- Do not replace, append to, or repair the plan during execution.
- Record plan-external work only in `update_plan.explanation` and the final response.

## Progress updates

- Follow any server-provided execute-specific instructions for how progress is recorded.
- If the server provides an automatic plan-dispatch tool, use that tool instead of manually updating plan rows.
- Otherwise, only update the server-selected row and do not update any other row.

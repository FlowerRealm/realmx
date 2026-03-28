# Collaboration Style: Ultra Work Execution

You are in the execution phase of Ultra Work. If the current thread has an accepted active plan, that accepted active plan is absolute truth during Ultra Work execution.

## Plan governance

- Do not review the plan in the execution phase.
- If the server provides plan workspace paths, read the provided `tasks.csv` path before acting.
- If the server provides a current executable row, only execute the server-selected row.
- Do not replace, append to, or repair the plan during execution.
- Record plan-external work only in `update_plan.explanation` and the final response.

## Progress updates

- Follow any server-provided execute-specific instructions for how progress is recorded.
- If the server provides an automatic plan-dispatch tool, use that tool instead of manually updating plan rows.
- Otherwise, only update the server-selected row and do not update any other row.

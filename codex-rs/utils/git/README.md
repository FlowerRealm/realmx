# codex-git

Helpers for interacting with git, including patch application and worktree
snapshot utilities.

Ghost snapshots work in both real Git repositories and non-Git workspaces. For
non-Git projects, `codex-git` can snapshot the workspace through a private
shadow Git directory while keeping the same create/restore API.

```rust,no_run
use std::path::Path;

use codex_git::{
    apply_git_patch, create_ghost_commit, restore_ghost_commit, ApplyGitRequest,
    CreateGhostCommitOptions,
};

let repo = Path::new("/path/to/repo");

// Apply a patch (omitted here) to the repository.
let request = ApplyGitRequest {
    cwd: repo.to_path_buf(),
    diff: String::from("...diff contents..."),
    revert: false,
    preflight: false,
};
let result = apply_git_patch(&request)?;

// Capture the current working tree as an unreferenced commit.
let ghost = create_ghost_commit(&CreateGhostCommitOptions::new(repo))?;

// Later, undo back to that state.
restore_ghost_commit(repo, &ghost)?;
```

Pass a custom message with `.message("…")` or force-include ignored files with
`.force_include(["ignored.log".into()])`.

For non-Git workspaces, provide a workspace root and shadow Git directory:

```rust,no_run
use codex_git::{CreateGhostCommitOptions, ShadowGitWorkspace};

let workspace_root = Path::new("/path/to/project");
let shadow_git_dir =
    ShadowGitWorkspace::shadow_git_dir_for(Path::new("/path/to/codex-home"), workspace_root);

let ghost = create_ghost_commit(
    &CreateGhostCommitOptions::new(Path::new("/path/to/project/subdir"))
        .workspace_root(workspace_root.to_path_buf())
        .shadow_git_dir(shadow_git_dir),
)?;
```

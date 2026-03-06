# Realmx CLI (Rust Implementation)

We provide Realmx CLI as a standalone, native executable to ensure a zero-dependency install.

## Installing Realmx

Today, the easiest way to install Realmx is via `npm`:

```shell
npm i -g @flowerrealm/realmx
realmx
```

The legacy `codex` command remains available for compatibility. You can also install via Homebrew (`brew install --cask codex`), which currently continues to expose the `codex` command name, or download a platform-specific release directly from our [GitHub Releases](https://github.com/openai/codex/releases).

## Documentation quickstart

- First run with Realmx? Start with [`docs/getting-started.md`](../docs/getting-started.md) (links to the walkthrough for prompts, keyboard shortcuts, and session management).
- Want deeper control? See [`docs/config.md`](../docs/config.md) and [`docs/install.md`](../docs/install.md).

## What's new in the Rust CLI

The Rust implementation is now the maintained Realmx CLI and serves as the default experience. It includes a number of features that the legacy TypeScript CLI never supported.

### Config

Realmx supports a rich set of configuration options. Note that the Rust CLI uses `config.toml` instead of `config.json`. See [`docs/config.md`](../docs/config.md) for details.

### Model Context Protocol Support

#### MCP client

Realmx CLI functions as an MCP client that allows the Realmx CLI and IDE extension to connect to MCP servers on startup. See the [`configuration documentation`](../docs/config.md#connecting-to-mcp-servers) for details.

#### MCP server (experimental)

Realmx can be launched as an MCP _server_ by running `realmx mcp-server`. The legacy `codex mcp-server` command also works. This allows _other_ MCP clients to use Realmx as a tool for another agent.

Use the [`@modelcontextprotocol/inspector`](https://github.com/modelcontextprotocol/inspector) to try it out:

```shell
npx @modelcontextprotocol/inspector realmx mcp-server
```

Use `realmx mcp` to add/list/get/remove MCP server launchers defined in `config.toml`, and `realmx mcp-server` to run the MCP server directly.

### Notifications

You can enable notifications by configuring a script that is run whenever the agent finishes a turn. The [notify documentation](../docs/config.md#notify) includes a detailed example that explains how to get desktop notifications via [terminal-notifier](https://github.com/julienXX/terminal-notifier) on macOS. When Realmx detects that it is running under WSL 2 inside Windows Terminal (`WT_SESSION` is set), the TUI automatically falls back to native Windows toast notifications so approval prompts and completed turns surface even though Windows Terminal does not implement OSC 9.

### `realmx exec` to run Realmx programmatically/non-interactively

To run Realmx non-interactively, run `realmx exec PROMPT` (you can also pass the prompt via `stdin`). The legacy `codex exec` command also works. Output is printed to the terminal directly. You can set the `RUST_LOG` environment variable to see more about what's going on.
Use `realmx exec --ephemeral ...` to run without persisting session rollout files to disk.

### Experimenting with the Realmx Sandbox

To test what happens when a command is run under the sandbox provided by Realmx, we provide the following subcommands in Realmx CLI:

```
# macOS
realmx sandbox macos [--full-auto] [--log-denials] [COMMAND]...

# Linux
realmx sandbox linux [--full-auto] [COMMAND]...

# Windows
realmx sandbox windows [--full-auto] [COMMAND]...

# Legacy aliases
realmx debug seatbelt [--full-auto] [--log-denials] [COMMAND]...
realmx debug landlock [--full-auto] [COMMAND]...
```

### Selecting a sandbox policy via `--sandbox`

The Rust CLI exposes a dedicated `--sandbox` (`-s`) flag that lets you pick the sandbox policy **without** having to reach for the generic `-c/--config` option:

```shell
# Run Realmx with the default, read-only sandbox
realmx --sandbox read-only

# Allow the agent to write within the current workspace while still blocking network access
realmx --sandbox workspace-write

# Danger! Disable sandboxing entirely (only do this if you are already running in a container or other isolated env)
realmx --sandbox danger-full-access
```

The same setting can be persisted in `~/.codex/config.toml` via the top-level `sandbox_mode = "MODE"` key, e.g. `sandbox_mode = "workspace-write"`.
In `workspace-write`, Realmx also includes `~/.codex/memories` in its writable roots so memory maintenance does not require an extra approval.

## Code Organization

This folder is the root of a Cargo workspace. It contains quite a bit of experimental code, but here are the key crates:

- [`core/`](./core) contains the business logic for Realmx. Ultimately, we hope this to be a library crate that is generally useful for building other Rust/native applications that use Realmx.
- [`exec/`](./exec) "headless" CLI for use in automation.
- [`tui/`](./tui) CLI that launches a fullscreen TUI built with [Ratatui](https://ratatui.rs/).
- [`cli/`](./cli) CLI multitool that provides the aforementioned CLIs via subcommands.

If you want to contribute or inspect behavior in detail, start by reading the module-level `README.md` files under each crate and run the project workspace from the top-level `codex-rs` directory so shared config, features, and build scripts stay aligned.

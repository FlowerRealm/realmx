<p align="center"><code>npm i -g @flowerrealm/realmx</code><br />or <code>brew install --cask codex</code></p>
<p align="center"><strong>Realmx CLI</strong> is a coding agent from OpenAI that runs locally on your computer.
<p align="center">
  <img src="https://github.com/openai/codex/blob/main/.github/codex-cli-splash.png" alt="Realmx CLI splash" width="80%" />
</p>
</br>
If you want Realmx in your code editor (VS Code, Cursor, Windsurf), <a href="https://developers.openai.com/codex/ide">install in your IDE.</a>
</br>If you want the desktop app experience, run <code>realmx app</code> (or <code>codex app</code> for compatibility) or visit <a href="https://chatgpt.com/codex?app-landing-page=true">the Codex App page</a>.
</br>If you are looking for the <em>cloud-based agent</em> from OpenAI, <strong>Codex Web</strong>, go to <a href="https://chatgpt.com/codex">chatgpt.com/codex</a>.</p>

---

## Quickstart

### Installing and running Realmx CLI

Install globally with your preferred package manager:

```shell
# Install using npm
npm install -g @flowerrealm/realmx
```

```shell
# Install using Homebrew
brew install --cask codex
```

Then run the command that matches how you installed it:

- `npm install -g @flowerrealm/realmx`: run `realmx` (the legacy `codex` command also works)
- `brew install --cask codex`: run `codex`

<details>
<summary>You can also go to the <a href="https://github.com/openai/codex/releases/latest">latest GitHub Release</a> and download the appropriate binary for your platform.</summary>

Each GitHub Release contains many executables, but in practice, you likely want one of these:

- macOS
  - Apple Silicon/arm64: `codex-aarch64-apple-darwin.tar.gz`
  - x86_64 (older Mac hardware): `codex-x86_64-apple-darwin.tar.gz`
- Linux
  - x86_64: `codex-x86_64-unknown-linux-musl.tar.gz`
  - arm64: `codex-aarch64-unknown-linux-musl.tar.gz`

Each archive currently contains a single entry with the platform baked into the name (e.g., `codex-x86_64-unknown-linux-musl`), so you will likely want to rename it to `realmx` after extracting it. If you have existing automation, keeping the legacy `codex` name also works.

</details>

### Using Realmx with your ChatGPT plan

Run `realmx` and select **Sign in with ChatGPT**. The legacy `codex` command also works. We recommend signing into your ChatGPT account to use Realmx as part of your Plus, Pro, Team, Edu, or Enterprise plan. [Learn more about what's included in your ChatGPT plan](https://help.openai.com/en/articles/11369540-codex-in-chatgpt).

You can also use Realmx with an API key, but this requires [additional setup](https://developers.openai.com/codex/auth#sign-in-with-an-api-key).

## Docs

- [**Realmx Documentation**](https://developers.openai.com/codex)
- [**Contributing**](./docs/contributing.md)
- [**Installing & building**](./docs/install.md)
- [**Open source fund**](./docs/open-source-fund.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).

# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## Connecting to MCP servers

Realmx can connect to MCP servers configured in `~/.realmx/config.toml`. See the configuration reference for the latest MCP server options:

- https://developers.openai.com/codex/config-reference

## Apps (Connectors)

Use `$` in the composer to insert a ChatGPT connector; the popover lists accessible
apps. The `/apps` command lists available and installed apps. Connected apps appear first
and are labeled as connected; others are marked as can be installed.

## Notify

Realmx can run a notification hook when the agent finishes a turn. See the configuration reference for the latest notification settings:

- https://developers.openai.com/codex/config-reference

When Realmx knows which client started the turn, the legacy notify JSON payload also includes a top-level `client` field. The TUI reports `codex-tui`, and the app server reports the `clientInfo.name` value from `initialize`.

## JSON Schema

The generated JSON Schema for `config.toml` lives at `codex-rs/core/config.schema.json`.

## Provider Usage Scripts

Realmx can poll provider-specific usage data with a project-local script instead of storing the
request in `config.toml`.

- Script path: `.codex/providers/<provider-id>/usage.js`
- Scope: trusted projects only
- Editor: run `/provider`, select a provider, then press `u`
- Default content: empty file
- Runtime: Node.js (`CODEX_JS_REPL_NODE_PATH`, `js_repl_node_path`, or `node` from `PATH`)

The script should evaluate to an object expression in the `ccswitch` style, for example:

```js
({
  request: {
    url: "{{baseUrl}}/usage",
    method: "GET",
    headers: {
      Authorization: "Bearer {{apiKey}}",
    },
  },

  extractor: function (response) {
    return [
      {
        remaining: Number(response.remaining),
        used: Number(response.used),
        unit: "USD",
      },
    ];
  },
})
```

Supported placeholders inside `request` string fields:

- `{{baseUrl}}`
- `{{apiKey}}`
- `{{providerId}}`
- `{{providerName}}`
- `{{bearerToken}}`
- `{{accessToken}}`
- `{{accountId}}`
- `{{userId}}`

`{{baseUrl}}` expands to the provider `base_url` exactly as configured. If your provider already
uses a base URL like `https://example.com/codex/v1`, append only the endpoint suffix such as
`{{baseUrl}}/usage`; do not repeat `/codex/v1` in `request.url`.

Request rules:

- `request.url` is required
- `request.method` is optional and defaults to `GET`
- `request.headers`, `request.body`, `request.bodyText`, and `request.bodyJson` are optional
- `request.bodyText` and `request.bodyJson` are mutually exclusive

`extractor(response)` receives the parsed JSON response body when the response is valid JSON.
Otherwise it receives the raw response text string.

Supported `extractor()` return values:

- A `ccswitch`-style rows array with fields such as `planName`, `remaining`, `used`, `total`, `unit`, `isValid`, and `extra`
- `null` to skip this poll cycle
- An error object such as `{ isValid: false, invalidMessage, invalidCode }`

Status line setup:

- The only provider-usage status item is `remote-usage`
- Older ids such as `provider-usage-remaining` or `su8-remaining` are normalized to `remote-usage` when read
- When remote usage refresh fails, Realmx records the error in the transcript, shows it in `/status`, and stops polling until configuration changes restart it

## SQLite State DB

Realmx stores the SQLite-backed state DB under `sqlite_home` (config key) or the
`CODEX_SQLITE_HOME` environment variable. When unset, WorkspaceWrite sandbox
sessions default to a temp directory; other modes default to `CODEX_HOME`.

## Custom CA Certificates

Codex can trust a custom root CA bundle for outbound HTTPS and secure websocket
connections when enterprise proxies or gateways intercept TLS. This applies to
login flows and to Codex's other external connections, including Codex
components that build reqwest clients or secure websocket clients through the
shared `codex-client` CA-loading path and remote MCP connections that use it.

Set `CODEX_CA_CERTIFICATE` to the path of a PEM file containing one or more
certificate blocks to use a Codex-specific CA bundle. If
`CODEX_CA_CERTIFICATE` is unset, Codex falls back to `SSL_CERT_FILE`. If
neither variable is set, Codex uses the system root certificates.

`CODEX_CA_CERTIFICATE` takes precedence over `SSL_CERT_FILE`. Empty values are
treated as unset.

The PEM file may contain multiple certificates. Codex also tolerates OpenSSL
`TRUSTED CERTIFICATE` labels and ignores well-formed `X509 CRL` sections in the
same bundle. If the file is empty, unreadable, or malformed, the affected Codex
HTTP or secure websocket connection reports a user-facing error that points
back to these environment variables.

## Notices

Realmx stores "do not show again" flags for some UI prompts under the `[notice]` table.

## Plan mode defaults

`plan_mode_reasoning_effort` lets you set a Plan-mode-specific default reasoning
effort override. When unset, Plan mode uses the built-in Plan preset default
(currently `medium`). When explicitly set (including `none`), it overrides the
Plan preset. The string value `none` means "no reasoning" (an explicit Plan
override), not "inherit the global default". There is currently no separate
config value for "follow the global default in Plan mode".

## Realtime start instructions

`experimental_realtime_start_instructions` lets you replace the built-in
developer message Codex inserts when realtime becomes active. It only affects
the realtime start message in prompt history and does not change websocket
backend prompt settings or the realtime end/inactive message.

Ctrl+C/Ctrl+D quitting uses a ~1 second double-press hint (`ctrl + c again to quit`).

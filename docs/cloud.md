# Kerf Cloud

Kerf Cloud is the hosted model behind Kerf's paid tier. The editor is free and
open source; a subscription buys requests to the hosted model. The service is
not deployed yet. This document is the contract the editor already follows,
so the service can be built to it.

## Where the service is

The editor reads the service URL from `KERF_CLOUD_URL`, else from `url` in
`~/.kerf/cloud.json` (on Windows, `%USERPROFILE%\.kerf\cloud.json`). Without
either, Kerf Cloud reports itself unavailable and the editor points people to
their own API key (Settings → Advanced).

The account token is kept in the system's keychain, as API keys are: the macOS
Keychain, Windows Credential Manager, or the Secret Service on Linux, under the
service `Industrial Kerf` and the name `cloud/token`. Where no keychain can be
reached (a Linux without a Secret Service, or `KERF_SECRETS=off`, which tests
set), the same file keeps it as `token`, written atomically with mode 0600. A
token in the file while a keychain can be reached, as older versions kept every
token, moves into the keychain the first time the editor reads it, and the file
keeps the rest. It is never stored in a project
(`kerf-qt-bridge/src/cloud_session.rs`).

```json
{ "url": "https://cloud.example.com" }
```

## Signing in

For now the editor signs in with a token the service issues, pasted into
Settings. A browser sign-in (OAuth device flow) can replace the paste without
changing anything below: the result is the same token.

## Endpoints

All requests carry the token. The editor sends it as `Authorization: Bearer
<token>` to the account endpoint and as `x-api-key: <token>` to the model,
which speaks the Anthropic Messages protocol.

| Request | Purpose |
|---|---|
| `GET /v1/account` | The signed-in account: `{"email", "plan", "ai", "credits"}`. `ai` says whether the plan includes hosted requests; `credits` is what is left this period, or `null` when unmetered. `401`/`403` signs the editor out. |
| `POST /v1/messages` | The hosted model, Anthropic Messages (streamed or not). The service picks the model; the editor sends its default and the service may replace it. |

The service must enforce every entitlement itself — plan, credits, rate
limits. The editor only reflects what `GET /v1/account` reports: it disables
Ask Kerf and says why when the account has no access, but a modified client
gains nothing, because the service refuses its requests.

Errors the editor shows as they are: `402` when credits are used up, `429`
when rate limited, with a `message` in the JSON body.

## In the editor

Settings → AI chooses Kerf Cloud or My own API key. Kerf Cloud is the default
unless a key of one's own is already saved, so existing setups keep working;
an explicit choice is remembered. Kerf Cloud
shows the account, its plan, and the requests left, with Sign in and Sign out;
Plans is a placeholder until pricing exists. Advanced holds the two
own-key slots (Anthropic Messages and OpenAI-compatible) and the MCP
settings. The status bar shows which connection the agent uses.

The bridge commands are `cloud_sign_in {token}`, `cloud_sign_out`, and
`cloud_refresh`; each answers with a `cloud` event: `access` (`ready`,
`unavailable`, `signedOut`, `unreachable`, `notIncluded`, `outOfCredits`) and
the `account` when signed in. An agent request with `"cloud": true` uses the
hosted model with the stored token.

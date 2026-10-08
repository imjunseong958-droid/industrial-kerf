# Cut Cloud

Cut Cloud is the hosted model behind Cut's paid tier. The editor is free and
open source; a subscription buys requests to the hosted model. The service is
not deployed yet. This document is the contract the editor already follows,
so the service can be built to it.

## Where the service is

The editor reads the service URL from `CUT_CLOUD_URL`, else from `url` in
`~/.cut/cloud.json`. Without either, Cut Cloud reports itself unavailable and
the editor points people to their own API key (Settings → Advanced). The same
file keeps the account token (`token`); it is written atomically with mode
0600 and never stored in a project.

```json
{ "url": "https://cloud.example.com", "token": "…" }
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
Ask Cut and says why when the account has no access, but a modified client
gains nothing, because the service refuses its requests.

Errors the editor shows as they are: `402` when credits are used up, `429`
when rate limited, with a `message` in the JSON body.

## In the editor

Settings → AI chooses Cut Cloud (the default) or My own API key. Cut Cloud
shows the account, its plan, and the requests left, with Sign in and Sign out;
Plans is a placeholder until pricing exists. Advanced holds the two
own-key slots (Anthropic Messages and OpenAI-compatible) and the MCP
settings. The status bar shows which connection the agent uses.

The bridge commands are `cloud_sign_in {token}`, `cloud_sign_out`, and
`cloud_refresh`; each answers with a `cloud` event: `access` (`ready`,
`unavailable`, `signedOut`, `unreachable`, `notIncluded`, `outOfCredits`) and
the `account` when signed in. An agent request with `"cloud": true` uses the
hosted model with the stored token.

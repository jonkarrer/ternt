# From ternt: the sidecar is wired up, two things left

Written 2026-11-21 from the ternt repo (`~/devjon/projects/ternt`), for whoever works on
tokidoki. It answers `docs/TERNT-HANDOFF.md`. The contract is ternt's `docs/AGENT-SIDECAR.md`;
everything the handoff proposed for it is in there now.

## Where things stand

Ternt runs `tokidoki-serve` as its agent sidecar and has been verified against it end to end,
with real keys, on both providers:

| provider | model | result |
| --- | --- | --- |
| Anthropic | tokidoki's default | a question streamed back as `delta` events, landed as the `checkpoint`, `turn_ended`; after `agent_restart` the thread was listed and its history back from disk |
| Fireworks | `TOKIDOKI_MODEL=accounts/fireworks/models/kimi-k3p1-pro` | the same, no warnings |
| Fireworks | tokidoki's default, `deepseek-v4-pro` | **404 from Fireworks on every call** (below) |

The ternt side is on `main` (commits `9400472` through `8695cb8`). Nothing in tokidoki was
changed by ternt; this file is the only thing ternt wrote here, and it is uncommitted.

## What ternt does, so you know what you're talking to

- **Builds your binary.** `scripts/sidecar.sh tokidoki` builds `tokidoki-serve` from the
  checkout beside the ternt repo (`TOKIDOKI_SRC` to point elsewhere) and puts it where Tauri
  bundles it; `--release` builds both Apple targets with Apple's toolchain and checks `otool -L`.
  The stub stays the default for ternt's own tests. Releases still ship the stub (see "Left to
  do").
- **Starts it** as `tokidoki-serve --data <dir>` with `<dir>/<space>` per space, `space-<id>`.
- **Bring your own key.** Ternt's Settings → Agent picks the provider and takes the user's key
  into the macOS Keychain. The process gets exactly this environment and nothing else about keys:

  ```
  TOKIDOKI_PROVIDER=fireworks   FIREWORKS_API_KEY=…    [BASETEN_API_KEY=…]
  TOKIDOKI_PROVIDER=anthropic   ANTHROPIC_API_KEY=…    [BASETEN_API_KEY=…]
  ```

  Fireworks is ternt's default. Changing the provider or the key restarts the process (SIGTERM,
  SIGKILL at 300 ms; yours exits in milliseconds). In development ternt's own environment wins,
  so `doppler run -- just dev tokidoki` uses the team keys. Ternt passes `TOKIDOKI_MODEL`
  through if it happens to be set, but **a ternt user has no way to set it**: your defaults are
  what they get.
- **Calls** `send`, `threads`, `history`, `abort`, `close`, `shutdown`, and `delete` when a space
  is deleted. Not yet: `memories`, `remember`, `revise`, `forget`, `feedback` (no UI for them).
- **Restarts you** when you die: after 0, 1, 3, 10 s; a fifth death in two minutes leaves you
  down with a Restart button. A restart ternt asks for isn't counted. Pending requests fail with
  "The agent process exited." and the panel re-reads `threads` and `history`, which is why your
  resume-from-disk matters and works.
- **Forwards your stderr** to its own log, prefixed `ternt-agent:`.
- **Tests you**: `TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki` in the ternt repo, with the keys in
  the environment (`doppler run -- …`). Without a key for the chosen provider it checks the
  "add your key" path; with one it asks one question, then restarts you and checks the thread is
  back. Each run gets its own `--data` folder, so it never meets the lock of a running `just dev`.

## Left to do, on your side

1. **A Fireworks default model that exists.** `Config::models(Vendor::Fireworks).0` is
   `accounts/fireworks/models/deepseek-v4-pro`; Fireworks answers
   `404 Model not found, inaccessible, and/or not deployed`, and the account's `/v1/models` has
   no such id. What it has, of the kind you want:

   ```
   accounts/fireworks/models/kimi-k3p1-pro          (verified through ternt)
   accounts/fireworks/models/deepseek-v4p1-flash    (your router default; present)
   accounts/fireworks/models/glm-5p3
   accounts/fireworks/models/qwen3p8-max
   accounts/fireworks/models/minimax-m3
   accounts/fireworks/models/gpt-oss-120b
   ```

   One line. Until then every ternt user on Fireworks (the default) gets no answers.
2. **Commit.** Ternt builds from your working tree, and a ternt release should name the tokidoki
   commit it was built from. Until there is one, ternt keeps shipping the stub.

## Worth doing, not blocking

- **Request bodies in warnings.** `inference::providers` logs `model call failed … request="{…}"`
  with the whole body: the system prompt and the user's message. Ternt puts your stderr in its
  log, so a user's messages land there on every failed call. The error alone would do.
- **`threads` and `history` without a key.** Fixed since the first handoff (they no longer open
  the agent). Ternt's suite accepts both behaviours; nothing to do, just noting it landed.
- **`label`** is still read and unused. Ternt keeps sending the space's name.
- **Size.** Release `tokidoki-serve` is 16 MB per architecture, 31 MB universal, beside ternt's
  1.3 MB stub: most of the download. Fine; known.

## Decided and deferred

- **The agent is a chat with memory for now.** Requests from the sidecar back to ternt (tabs,
  pages, terminals) are deferred by decision; don't build them. The frame shape sketched in
  `AGENT-SIDECAR.md` (`ask`/`answer` lines) is there for when that changes.
- **A key per space** is not designed. One environment, one provider, one key.
- `when_busy: steer` is sent by ternt and accepted by you; not yet exercised mid-answer with a
  real model.

## What happens next on ternt's side

Once 1 and 2 above land: `TERNT_SIDECAR=tokidoki just upload minor` ships the real agent to
installed copies, with Settings → Agent asking each user for their own key.

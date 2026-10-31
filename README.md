# Fix it with a photo

A knob is missing from your oven. Take one phone photo of the empty shaft, a knob that is still there and a coin. Meta SAM 3.1 measures, Muse Spark designs the replacement, and Any Part Made checks, prices and guarantees it.

Live demo: https://anypartmade-demo.jobboard.workers.dev

## Built during the hackathon

Everything in this repo was built from Oct 31 to Nov 4, 2026, during the Meta Global AI Developer Hackathon (Indie track). The commit history shows it.

Pre-existing and **not** in this repo: the Any Part Made verification API at `https://mcp.anypartmade.com` ([docs](https://anypartmade.com/docs)). This demo calls it over its public MCP endpoint, like any other agent.

## Status

| Step | State |
|---|---|
| 1. Phone page, photo capture, storage | live |
| 2. SAM 3.1 masks and measurements | next |
| 3. KnobSpec to mesh to 3MF | |
| 4. Muse Spark design and fix loop | |
| 5. Quote and order link | |

## How it works (so far)

- `GET /` is one page with a small inline script. It shrinks the photo to 2048 px in the browser and uploads it.
- `POST /api/session` checks the photo in Rust (JPEG or PNG, at most 8 MB, 480 to 2048 px), stores it in R2 and creates a session in D1. 20 sessions per network per day.
- `GET /api/session/{id}` returns the state for the page.
- Photos expire after 7 days (R2 lifecycle rule), sessions by a daily cron.

## Run it

Rust with the `wasm32-unknown-unknown` target, Node 20 or newer.

```sh
cargo test
npx wrangler@4 d1 migrations apply anypartmade-demo --remote
npx wrangler@4 secret put META_API_KEY   # local runs: put it in .dev.vars, which git ignores
npx wrangler@4 deploy
```

No keys are in this repo. `META_API_KEY` is a Worker secret.

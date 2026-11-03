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
| 2. SAM 3.1 masks and measurements | live: ruler scale from the ticks; shaft by calipers when SAM misses it |
| 3. KnobSpec to mesh to 3MF, checked by Any Part Made | live |
| 4. Muse Spark design and fix loop | |
| 5. Quote and order link | |

## How it works (so far)

- `GET /` is one page with a small inline script. It shrinks the photo to 2048 px in the browser and uploads it.
- `POST /api/session` checks the photo in Rust (JPEG or PNG, at most 8 MB, 480 to 2048 px), stores it in R2 and creates a session in D1. 20 sessions per network per day.
- `GET /api/session/{id}` returns the state for the page.
- Photos expire after 7 days (R2 lifecycle rule), sessions by a daily cron.

## Measuring with SAM 3.1

- The Worker calls `POST https://api.meta.ai/v1/responses` with model `sam-3.1`, one concept prompt per request (`coin` or `ruler`, `shaft`, `knob`), the photo inline as a data URL, `stream: false`, `store: false` and `metadata.include_confidence: "true"`. Source: https://dev.meta.ai/docs/sam/segmenting
- SAM answers with special token text: a box and a base85 mask per object. `src/sam/codec.rs` decodes the masks. It is a Rust port of the decoder in Meta's `@meta-sam/parser` 0.1.0, used under the SAM License (`LICENSE-SAM`), and it is tested against rasters from that package.
- The numbers come from geometry in Rust (`src/measure.rs`), not from a model: sub pixel edges from the mask coverage, ellipse moments for the coin, a circle fit that sets the D flat aside plus a line fit on the flat for the shaft, and a radius profile for the knob diameter and grip ribs.
- Scale, found automatically: a coin (its known diameter), or a ruler. For a ruler, SAM finds it and Rust (`src/ruler.rs`) reads the millimetre ticks as a periodic pattern: it tells the metric edge from an inch edge by the 5 and 10 mm marks, measures the local tick spacing in windows along the ruler, fits it against position (a tilted ruler changes scale along its length) and takes the scale next to the shaft. Tapping two ruler marks is the fallback.
- Results are cached per photo hash in R2. Every paid call is logged in D1 with its estimated cost, and the Worker stops calling models once a day's spend reaches `BUDGET_USD_PER_DAY` (5 dollars).
- When the shaft is unclear, the user types the calipers reading instead.

## From spec to a checked part

- `src/knob.rs`: the `KnobSpec`, a small JSON feature language (body, top, skirt, grip, pointer, D bore, shell). Rust checks its bounds and sweeps a profile around the axis into a watertight mesh. The shaft numbers come from the measuring step and a spec cannot change them. Tests: every shape is watertight, the D bore lands at the measured size, clearance grows it.
- `src/threemf.rs`: the mesh as a 3MF (a zip with deflate, no Zip64). The default knob is about 37 KB.
- `src/mcp.rs`: `check_part` on `https://mcp.anypartmade.com/mcp` over JSON-RPC, with `process: fdm`, `material: asa`, the use and energy source from the user's line, the library D shaft as an interface and the bore as a critical dimension. Every issue comes back with a fix, which the next round uses.
- `POST /api/session/{id}/check` runs one round; `GET /api/session/{id}/model/{n}` downloads that round's 3MF.

## Run it

Rust with the `wasm32-unknown-unknown` target, Node 20 or newer.

```sh
cargo test
npx wrangler@4 d1 migrations apply anypartmade-demo --remote
npx wrangler@4 secret put META_API_KEY   # local runs: put it in .dev.vars, which git ignores
npx wrangler@4 deploy
```

MIT licensed (`LICENSE`); the SAM mask decoder port is under the SAM License (`LICENSE-SAM`). No keys are in this repo. `META_API_KEY` is a Worker secret.

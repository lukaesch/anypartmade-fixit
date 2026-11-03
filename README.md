# Fix it with a photo

A knob is missing from your oven. Take one phone photo of the empty shaft with a coin or a ruler next to it. Meta SAM 3.1 measures, Muse Spark designs the replacement, and Any Part Made checks, prices and guarantees it.

A design agent you can trust because a deterministic verifier checks every step: Muse never writes geometry or code, only a small JSON spec. Rust builds the part, Any Part Made says exactly what is wrong and where, Muse fixes it, and the loop runs until the part passes.

Live demo: https://anypartmade-demo.jobboard.workers.dev

## Built during the hackathon

Everything in this repo was built from Oct 31 to Nov 4, 2026, during the Meta Global AI Developer Hackathon (Indie track). The commit history shows it.

Pre-existing and **not** in this repo: the Any Part Made verification API at `https://mcp.anypartmade.com` ([docs](https://anypartmade.com/docs)). This demo calls it over its public MCP endpoint, like any other agent.

## What happens

| Step | Who | What |
|---|---|---|
| 1. Photo | Phone page, Rust | Photo of the empty shaft with a coin or ruler; the user names the part. Stored in R2 for 7 days. |
| 2. Measure | SAM 3.1, Rust | SAM finds the coin or ruler, the shaft and a sibling knob. Rust turns masks and ruler ticks into millimetres. Calipers can override the shaft. |
| 3. Design | Muse Spark | Looks at the photo and the numbers and writes a `KnobSpec`. |
| 4. Build | Rust | KnobSpec to a watertight mesh to a 3MF, and a shaded picture of it (PNG, rendered in Rust). |
| 5. Check | Any Part Made | `check_part`: verdict, and every issue with a location and a fix. |
| 6. Fix | Muse Spark, Rust, Any Part Made | Muse reads the issues and changes the spec; build and check again, at most four times. |
| 7. Quote | Any Part Made | `quote_part` and `create_order_link`: price, dates, guaranteed sizes, approval on anypartmade.com. |

All seven steps are live. On Lukas's oven photo the knob passes in three rounds.

## Routes

| Route | Does |
|---|---|
| `GET /` | The phone page: one HTML document with a small inline script, pinned by its hash in the CSP. No tracking, no external assets. |
| `POST /api/session` | Photo (JPEG or PNG, at most 8 MB, 480 to 2048 px, shrunk in the browser), the part line and coin or ruler. 20 sessions per network per day. |
| `GET /api/session/{id}` | State for the page. |
| `POST /api/session/{id}/see` | SAM 3.1 and the measurements. |
| `POST /api/session/{id}/scale`, `/shaft` | Fallbacks: two tapped ruler marks, or calipers. |
| `POST /api/session/{id}/design` | Muse designs, round 1 is checked. |
| `POST /api/session/{id}/fix` | One fix round. |
| `GET /api/session/{id}/model/{n}`, `/model/{n}.png` | The 3MF of round n, and its picture. |
| `POST /api/session/{id}/quote`, `/order` | Price for the passed part, and the approval link for the picked option. |

## Measuring with SAM 3.1

- The Worker calls `POST https://api.meta.ai/v1/responses` with model `sam-3.1`, one concept prompt per request (`coin` or `ruler`, `shaft`, `knob`), the photo inline as a data URL, `stream: false`, `store: false` and `metadata.include_confidence: "true"`. Source: https://dev.meta.ai/docs/sam/segmenting
- SAM answers with special token text: a box and a base85 mask per object. `src/sam/codec.rs` decodes the masks. It is a Rust port of the decoder in Meta's `@meta-sam/parser` 0.1.0, used under the SAM License (`LICENSE-SAM`), and it is tested against rasters from that package.
- The numbers come from geometry in Rust (`src/measure.rs`), not from a model: sub pixel edges from the mask coverage, ellipse moments for the coin, a circle fit that sets the D flat aside plus a line fit on the flat for the shaft, and a radius profile for the knob diameter and grip ribs.
- Scale, found automatically: a coin (its known diameter), or a ruler. For a ruler, SAM finds it and Rust (`src/ruler.rs`) reads the millimetre ticks as a periodic pattern: it tells the metric edge from an inch edge by the 5 and 10 mm marks, measures the local tick spacing in windows along the ruler, fits it against position (a tilted ruler changes scale along its length) and takes the scale next to the shaft. Tapping two ruler marks is the fallback.
- Results are cached per photo hash in R2. Every paid call is logged in D1 with its estimated cost, and the Worker stops calling models once a day's spend reaches `BUDGET_USD_PER_DAY` (5 dollars).
- When the shaft is unclear, the user types the calipers reading instead.

## From spec to a checked part

- `src/knob.rs`: the `KnobSpec`, a small JSON feature language (body, top, skirt, grip, pointer, D bore, shell). Rust checks its bounds and sweeps a profile around the axis into a watertight mesh. The shaft numbers come from the measuring step and a spec cannot change them. Tests: every shape is watertight, the D bore lands at the measured size, clearance grows it.
- `src/render.rs`: an orthographic, flat shaded picture of the mesh with a z buffer, written as a PNG, so the page shows each round's knob without WebGL or external assets.
- `src/threemf.rs`: the mesh as a 3MF (a zip with deflate, no Zip64). The default knob is about 37 KB.
- `src/mcp.rs`: `check_part` on `https://mcp.anypartmade.com/mcp` over JSON-RPC, with `process: fdm`, `material: asa` (heat), the use and energy source from the user's line, and the bore as critical dimensions. Every issue comes back with a fix, which the next round uses.
- The D bore is declared as two straight distances (round diameter, and round side to flat), which the core measures exactly. The library D shaft interface would send every knob to a person, because a D cannot be measured as a round hole.
- `POST /api/session/{id}/check` runs one round; `GET /api/session/{id}/model/{n}` downloads that round's 3MF.

## Muse Spark designs and repairs

- `src/muse.rs` calls `POST https://api.meta.ai/v1/chat/completions` with model `muse-spark-1.3`, `response_format` of type `json_schema` (the KnobSpec schema plus a list of changes and a short summary) and `max_completion_tokens`. The design call also sends the photo as an `image_url` data URL. Sources: https://dev.meta.ai/docs/structured-output, https://dev.meta.ai/docs/image-understanding, https://dev.meta.ai/docs/pricing-rate-limits
- Design (`POST /api/session/{id}/design`): Muse sees the photo, the measurements and the user's line and writes the KnobSpec. Rust overwrites the shaft numbers with the measured ones, checks the bounds, builds the 3MF, and Any Part Made checks it (round 1).
- Fix (`POST /api/session/{id}/fix`): Muse gets the current spec and every issue with the checker's fix text and location, and returns a changed spec with each change and its reason. Rust rebuilds, the core checks again. The page runs this until the verdict is `ok`, at most four fixes, and shows every round: verdict, issues, fixes, what Muse changed, and the 3MF.
- If Rust refuses a spec, Muse is told why once. A safety refusal from the core is final: it is shown as the core wrote it and never sent again in other words.
- Every Muse call is cached by the hash of its request and logged with its cost from `usage` ($1.25 in, $4.25 out per million tokens).
- On Lukas's oven photo: round 1 had a 0.22 mm thin wall where the pointer groove meets the ribs and an overhanging ceiling inside; Muse made the knob solid (round 2), then turned the groove into a ridge (round 3, verdict ok, guaranteed bore 6.2 and 4.8 mm).

## Quote and approval

- `quote_part` with the passed round's `check_id` and the ship to country (picker, default Germany) gives three options with price, dates and the guaranteed dimensions.
- Picking an option calls `create_order_link`. The page shows a button to the approval page on anypartmade.com. Nothing is ordered or charged until the user approves there. Quotes are test quotes from a simulated partner for now, and the page says so.

## Spend

Every paid Meta call is logged in D1 with its estimated cost, results are cached in R2, and the Worker stops calling models once a day's spend reaches 5 dollars. `RUN_LOG.md` has the totals per build session: the whole build so far cost about 10 cents.

## Run it

Rust with the `wasm32-unknown-unknown` target, Node 20 or newer.

```sh
cargo test
npx wrangler@4 d1 migrations apply anypartmade-demo --remote
npx wrangler@4 secret put META_API_KEY   # local runs: put it in .dev.vars, which git ignores
npx wrangler@4 deploy
```

MIT licensed (`LICENSE`); the SAM mask decoder port is under the SAM License (`LICENSE-SAM`). No keys are in this repo. `META_API_KEY` is a Worker secret.

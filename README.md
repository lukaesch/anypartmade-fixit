# Fix it with a photo

You need a part that fits something real: a knob that is missing from your oven, a case for a circuit board. Take a phone photo with a ruler or a coin next to it and say what you need. **Meta SAM 3.1** measures, **Muse Spark** designs the part in a CAD language and repairs it, and the [Any Part Made](https://anypartmade.com) verifier checks every design, says exactly what to fix and where, and guarantees the sizes that must fit.

A design agent you can trust because a deterministic verifier checks every step: numbers come from measurements, not from a model's eye; Muse writes the design, Rust builds it, Any Part Made inspects it, and the loop runs until it passes.

Live demo: https://anypartmade-demo.jobboard.workers.dev (a hosted copy; the project runs from this repo, see [Run it](#run-it)).

## Built during the hackathon, and what existed before

Everything in this repo was built from Oct 31 to Nov 4, 2026, during the Meta Global AI Developer Hackathon (Indie track). The commit history shows it.

**Existed before, not in this repo:** the Any Part Made verification service at `https://mcp.anypartmade.com` ([docs](https://anypartmade.com/docs)). It checks a 3MF or STL file for printing and for fit, prices it and creates an approval link. This app calls its public MCP endpoint like any other client. It is a deterministic checker, not a model.

**Built this week:** the phone app, the agent, the measuring, the CAD language and mesher, the in-browser fabricator and hologram, all in this repo.

## Where Meta's models are used

Every model this project calls at runtime is a Meta model, through the Meta Model API (`https://api.meta.ai/v1`):

| Model | Endpoint | What it does here |
|---|---|---|
| **SAM 3.1** (`sam-3.1`) | `POST /v1/responses` | Segments the ruler or coin, the shaft, the board or any object, by a short noun phrase, a tapped point or a box of two taps. Masks are decoded in Rust and turned into millimetres. |
| **Muse Spark** (`muse-spark-1.3`) | `POST /v1/chat/completions` with tool calling | The agent: looks at the photos, decides what to measure, asks only for what it needs, designs the part in the CAD language, reads the checker's issues and repairs the design, gets the quote and the approval link. |

Request shapes, model ids and prices are from https://dev.meta.ai/docs (SAM segmenting and reading segmentation, tool calling, structured output, image understanding, reasoning, pricing). Each step is shown on screen, so the demo video shows every Meta call as it happens.

## How it works

```
phone photo ──► SAM 3.1 masks ──► Rust: ruler ticks, mm, shaft D, board size
                                         │  (numbers recorded with their source)
user words ──► Muse Spark agent ◄────────┘
                   │ CAD part (JSON)
                   ▼
           Rust mesher (WebAssembly, on the phone) ──► closed mesh ──► 3MF
                   ▼
           Any Part Made check_part ──► verdict, issues with location and fix
                   │                          │
                   └──── Muse repairs ◄───────┘   (until ok)
                   ▼
           quote_part ──► price tiles ──► create_order_link ──► approve on anypartmade.com
```

1. **Measure.** SAM 3.1 finds the ruler (or a coin), and Rust reads the ruler's millimetre ticks as a periodic signal (metric edge chosen by its 5 and 10 mm marks, local spacing fitted along the ruler for tilt, scale taken next to the object). For a D shaft seen end on, Rust fits the circle and the flat; seen from the side, its width and length. For a board, the smallest rotated rectangle around the outline. When SAM's word search misses, the user taps the thing (a point prompt) or two corners (a box prompt).
2. **Record.** Muse records each number it relies on with `set_dimension`. Rust accepts a value only if a measurement returned it (cited like `m3.shaft.diameter_mm`) or the user wrote it in their own words.
3. **Design.** Muse writes the part in a small CAD language (`src/cad.rs`): boxes, cylinders, extrusions, revolves, unions, differences, moves, repeats, and **fit features**: `d_hole` for a measured shaft, `pocket` for a measured board, `usb_c_cutout` for a USB C plug, `hole`. The sizes of fit features come from the recorded measurements, never from Muse, and the checks Any Part Made needs are derived from them (the D hole as two straight distances, the pocket's length and width, the `usb_c_receptacle` interface).
4. **Build.** The phone builds the part with the same Rust as WebAssembly (`mesher/`): the CAD part as a signed distance field, marching tetrahedra (always a closed mesh), then quadric edge collapse simplification (a case goes from 300,000 to 8,000 triangles, 66 KB) with a strict check that a checker merging points within 0.01 mm sees no holes.
5. **Check and repair.** Any Part Made returns a verdict and every issue with a location and a fix. Muse changes the design and the loop runs again. A safety refusal is final: it is shown as given and never resubmitted with other wording.
6. **Quote and approve.** Price tiles, the guaranteed sizes, and a button to approve on anypartmade.com. Nothing is ordered until the user approves there. Quotes are test quotes from a simulated partner for now.

The page is a full screen design console: the photo with a scan beam and SAM's outline, a WebGL hologram that builds up from the floor, the inspection ring with issue markers at the checker's coordinates, a 10 mm grid with dimension lines, and the part turning solid when it passes. No libraries, no external assets; everything is inline in one page, pinned by its hash in the Content Security Policy.

## Run it

You need Rust with the `wasm32-unknown-unknown` target, Node 20 or newer, and a Meta Model API key from https://dev.meta.ai (API keys tab; it starts with `LLM_`).

```sh
git clone https://github.com/lukaesch/anypartmade-fixit && cd anypartmade-fixit
rustup target add wasm32-unknown-unknown

# Your Meta key, for local runs only. .dev.vars is in .gitignore; never commit a key.
cp .dev.vars.example .dev.vars
# then edit .dev.vars: META_API_KEY=LLM_...

# Local database (D1) and storage (R2) are simulated by Wrangler.
npx wrangler@4 d1 migrations apply anypartmade-demo --local
npx wrangler@4 dev
```

Open http://localhost:8787 (on a phone: `http://<your-computer-ip>:8787` on the same network), tap **Case for a board** or **Replace a knob**, add a photo with a ruler next to the object and send. The first build compiles the Rust Worker and the browser mesher (`mesher/`), which takes a minute.

Without a key the app starts and stops at the first model call with "The Meta API key is not set."

Tests: `cargo test` (56 tests: mask decoder against Meta's parser, ruler reading, geometry, CAD language, closed meshes, simplification, agent rules, page). Lint: `cargo clippy --all-targets` and `cargo clippy --target wasm32-unknown-unknown`.

Deploy your own copy to Cloudflare Workers: create the D1 database and R2 bucket named in `wrangler.jsonc`, set the key with `npx wrangler@4 secret put META_API_KEY`, then `npx wrangler@4 deploy`.

## Spend and limits

Every paid Meta call is logged with its estimated cost (SAM $2.50 per 1,000 images; Muse Spark $1.25 in and $4.25 out per million tokens, from `usage`), SAM and Muse results are cached per photo and request, and the app stops calling models once a day's spend reaches `BUDGET_USD_PER_DAY` (5 dollars). A full repair costs about 10 to 15 cents. The whole build cost about 2.25 dollars in model calls (`RUN_LOG.md`).

## Code map

| Path | What |
|---|---|
| `src/agent.rs` | Muse Spark agent: instructions, tools, the rule that numbers come from measurements or the user's words |
| `src/cad.rs` | The CAD language, fit features, derived checks, distance field and marching tetrahedra |
| `src/simplify.rs` | Quadric edge collapse simplification and the strict mesh check |
| `src/sam/` | SAM 3.1 requests and a Rust port of the mask decoder from Meta's `@meta-sam/parser` (SAM License, `LICENSE-SAM`) |
| `src/ruler.rs`, `src/measure.rs`, `src/see.rs` | Ruler tick reading, mask geometry, measurements |
| `src/mcp.rs` | Any Part Made over MCP: check, quote, order link, status |
| `src/agent_page.rs` | The phone app: stage, hologram, dialog |
| `mesher/` | The part builder as browser WebAssembly |
| `src/entry.rs` | Worker routes and glue |
| `src/knob.rs`, `src/muse.rs`, `src/page.rs` | The first, step by step knob flow, still at `/guided` |

## License

MIT (`LICENSE`). The SAM mask decoder port is under the SAM License (`LICENSE-SAM`). No keys are in this repo; `META_API_KEY` is a Worker secret, or `.dev.vars` locally.

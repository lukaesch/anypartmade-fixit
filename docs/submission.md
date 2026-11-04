# Submission drafts

Form: https://cerebralvalley.ai/e/meta-ai-developer-hackathon/hackathon/submit (deadline Wed Nov 4, 9:00 PM ET, Thu Nov 5, 03:00 Berlin; edits allowed until then).

## Basics

- **Project name:** Any Part Made Studio (tagline: Fix it with a photo)
- **Track:** Indie
- **Bonus:** Best Agentic Use Case (and Most Viral with the X post link)
- **Repo:** https://github.com/lukaesch/anypartmade-fixit (public, created Oct 31, only work from the event)
- **Video:** (YouTube or Loom link, playable without login, at most 3:00)
- **Live demo:** https://studio.anypartmade.com

## One line

A phone photo in, a verified part out: Meta SAM 3.1 measures what the part must fit, Muse Spark designs and repairs it, and a deterministic verifier guarantees the fit.

## Description (short)

When a knob breaks off an oven or a board needs a case, the part is usually discontinued or never existed. 3D printing could make it, but nobody can measure and model it. Any Part Made Studio lets anyone take a photo with a ruler next to the object and say what they need. SAM 3.1 segments the ruler and the object and Rust turns the masks into millimetres. Muse Spark, as an agent with tools, asks only for what is missing, designs the part in a small CAD language and sends it to Any Part Made, which checks it for printing and fit and says exactly what to fix and where. Muse repairs the design until it passes, then the user gets a price and approves on anypartmade.com. The guarantee covers the sizes that were measured.

## What existed before, what was built during the event

- **Before:** Any Part Made, a verification API at mcp.anypartmade.com that checks, quotes and guarantees parts (deterministic, no model). Used as an external service through its public MCP endpoint.
- **During the event (all in the repo):** the phone app and its full screen design console with a WebGL hologram; the Muse Spark agent with tools; SAM 3.1 integration including a Rust port of Meta's mask decoder; the ruler tick reader; the CAD language with fit features and derived checks; the distance field mesher and mesh simplification compiled to WebAssembly so the phone builds the part; the quote and approval flow.

## How Meta's models are used

- **SAM 3.1** segments the ruler or coin, the shaft, the board or any object by noun phrase, tapped point or a box of two taps. Masks are decoded in Rust (port of `@meta-sam/parser`) and turned into millimetres: ruler ticks give the scale, a D fit gives shaft diameter and flat, a rotated rectangle gives the board size.
- **Muse Spark** (`muse-spark-1.3`, tool calling) is the agent and the designer. It sees the photos, chooses what to measure, records each number with its source, asks the user only for what the part needs, writes the part in the CAD language, reads the verifier's issues and repairs the design, and handles quote and approval. Reasoning effort is low while talking and medium while designing.
- No other models are called at runtime.

## How it is built

Rust on Cloudflare Workers (workers-rs), one inline HTML page, no external assets. The agent's numbers are guarded: a dimension is accepted only from a measurement result or the user's own words, and fit features take their sizes from those dimensions. The CAD part becomes a signed distance field, then a closed mesh by marching tetrahedra, then a small 3MF by quadric edge collapse with a strict check, all in WebAssembly on the phone. The verifier's answer drives the repair loop. Tests cover the mask decoder against Meta's parser, ruler reading, geometry, the CAD language, closed meshes and the agent rules (56 tests).

## Challenges

- SAM's word search misses small parts like a 6 mm shaft. The answer was interaction: the agent asks the user to tap, and a box of two taps for things made of parts (a board, where one tap only picks the label).
- Muse's own pixel guesses are rough, so it never reads numbers off a photo; it cites measurements.
- Meshes from a grid are huge. Quadric simplification shrank them 40 times, and a strict check stopped tiny edges that a checker would merge into holes.
- Cloudflare's free plan allows 10 ms CPU per request, too little to mesh a part, so the phone builds it.

## Impact

Spare parts for appliances, gadgets and repairs are a daily problem and the main reason things get thrown away. This turns a phone photo into a part that fits, with a guarantee, without CAD skills.

## What is next

More inputs than photos (video, LiDAR depth, an existing CAD file), more fit features (screw bosses, snap lids, hinges), and real manufacturing partners behind the approval link.

## X post (for Most Viral)

Lost an oven knob? Snap a photo with a ruler. Meta SAM 3.1 measures it, Muse Spark designs the replacement, and Any Part Made checks it until it fits. Built at the Meta AI Developer Hackathon. (video link) (repo link)

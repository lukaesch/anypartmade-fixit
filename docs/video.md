# Demo video storyboard (3 minutes)

Record on the phone in light mode at https://anypartmade-demo.jobboard.workers.dev. Every step is a separate call, so each one shows on screen. Keep the "test quote" note visible: quotes come from a simulated partner.

| Time | On screen | Voice over |
|---|---|---|
| 0:00 to 0:15 | The oven with one knob missing | "One knob is missing. The spare is discontinued. This is the most common repair problem there is." |
| 0:15 to 0:30 | Title card: built this week (photo page, SAM measuring, Muse design loop) and what existed before (the Any Part Made checker at mcp.anypartmade.com) | "Any Part Made checks parts that AI agents design. This week I built the part that lets anyone start from a photo." |
| 0:30 to 0:45 | Phone: take the photo of the empty shaft with a ruler next to it, type "control knob, electric oven", pick the ruler | "I take one photo and say what is missing." |
| 0:45 to 1:10 | Tap Measure with SAM 3.1: the ruler outline and the 10 mm line from its ticks appear, the scale in px per mm with its confidence | "SAM 3.1 finds the ruler. Rust reads the millimetre ticks, so the scale comes from the photo itself. No model guesses a number." |
| 1:10 to 1:20 | The shaft numbers: from the photo, or typed from calipers when SAM is unsure | "Where the photo is not enough, I type the shaft from calipers. The design never changes these numbers." |
| 1:20 to 2:05 | Tap Design with Muse Spark. Round 1: needs changes, a 0.22 mm thin wall at the pointer groove and an overhang inside. Round 2: Muse makes it solid. Round 3: Muse turns the groove into a ridge, verdict ok | "Muse Spark looks at the photo and writes the knob as a small spec. Rust builds the 3D file. Any Part Made checks it and says exactly what is wrong and where. Muse reads that, changes the spec, and checks again. Three rounds, and it passes." |
| 2:05 to 2:25 | Get the price: three options with dates, the guaranteed bore sizes, Review and approve on anypartmade.com | "A real price and a written guarantee on the two sizes that matter. Nothing is made until I approve it." |
| 2:25 to 2:35 | Only if the core's safety screen is on: a gas hob knob gets refused, shown as the core wrote it | "And it refuses parts that could hurt someone." |
| 2:35 to 2:50 | The printed knob pushed onto the shaft, turns | "It fits." |
| 2:50 to 3:00 | The public repo, anypartmade.com, mcp.anypartmade.com | "A photo in. A verified part out." |

## Shots to get

1. The oven front with the gap (wide), then close on the empty shaft.
2. The photo for the app: straight on at the shaft end, ruler flat next to it, good light, no glare on the ruler ticks. A sibling knob in view helps Muse match the look.
3. Screen recording of the phone page through all steps. Clean session, so start from the page with no `#s=` in the address.
4. The printed knob (from the round 3 3MF, Download link on the page) going onto the shaft.

## Before recording

- Shaft and flat measured with calipers and typed in, if SAM does not find the shaft.
- The Muse and SAM results are cached per photo and request, so a second take of the same photo costs nothing and gives the same rounds.
- The gas refusal clip needs the core's safety screen on. If it is not on, drop that shot and give the time to the fix loop.

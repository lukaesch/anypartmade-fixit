# Demo video (at most 3:00, English)

Judges score Impact 30%, Use of Meta AI Tooling 25%, Technical Depth and Execution 20%, Demo 25%. Most of the video is real footage of the app running on the phone. Every Meta call is visible as a card on the stage.

Record the phone screen (iOS screen recording) plus a few camera shots of the real objects. Start each take from a fresh page (no `#s=` in the address). Cut the waiting time while Muse thinks or speed it up 4x; keep the stage animations at normal speed.

| Time | On screen | Voice over |
|---|---|---|
| 0:00 to 0:12 | Title card, then the oven with one knob missing | "Any Part Made already existed: a verifier that checks, prices and guarantees parts that AI agents design. This week I built Any Part Made Studio on top of it, with Meta SAM 3.1 and Muse Spark." |
| 0:12 to 0:25 | Camera: the empty shaft, a ruler laid next to it, the phone takes the photo | "A knob is missing. I take one photo with a ruler next to the shaft." |
| 0:25 to 0:50 | Phone: send the photo with "My oven is missing a knob". Stage: the scan beam, SAM's outline draws in. Muse asks for a tap; tap the shaft. Readout: shaft 5.96 mm, flat 4.37 mm, scale from the ruler ticks. The facts lock in at the top | "SAM 3.1 finds the ruler and the shaft. Rust reads the millimetre ticks, so the scale comes from the photo. Where SAM is unsure, Muse asks me to tap. Every number has a source." |
| 0:50 to 1:35 | Muse designs. Stage: "Fabricator, Rust on this device", the hologram builds up from the 10 mm grid. Inspection ring slides up; a red marker pulses where Any Part Made found a thin wall. Muse's line: what it changes. Next round: passes, the part turns solid, the certificate appears | "Muse Spark is the designer. It writes the knob in a small CAD language; my phone builds the mesh in Rust. Any Part Made inspects it and says exactly what is wrong and where. Muse fixes it, and checks again, until it passes." |
| 1:35 to 1:55 | The price tiles under the hologram, tap Standard, the approve button | "A real price, and a written guarantee on the sizes that must fit. Nothing is made until I approve it." |
| 1:55 to 2:30 | Second run, sped up: the ESP32 board photo, two taps mark the board (21.6 by 18.7 mm), Muse asks for the height and offers "Type it instead", the case builds, the USB C opening is checked, passes | "It is not a knob generator. The same agent designs a case for this board: SAM measures it, Muse designs pocket, antenna bay and USB C opening, and Any Part Made checks that the plug fits." |
| 2:30 to 2:48 | The printed knob pushed onto the shaft and turned (if printed); else the 3MF in a slicer | "It fits." |
| 2:48 to 3:00 | The repo, the "Where Meta's models are used" table in the README | "SAM 3.1 measures, Muse Spark designs and repairs, Any Part Made verifies. A photo in, a verified part out." |

## Shots to get

1. The oven front with the gap (wide), then close on the empty shaft with the ruler.
2. The photo for the app: straight at the shaft end, the ruler right beside it, bright light, no glare on the ticks. IMG_1571 works.
3. The board flat on the table, the ruler touching its long edge, photo from straight above. IMG_1573 works.
4. Screen recordings of both runs on the phone, from the start screen to the approval button.
5. The printed knob going onto the shaft, if there is one.

## Before recording

- Use the phone in a quiet light setting; the app is dark by design.
- Muse and SAM results are cached per photo and request, so a second take with the same photos and the same words is faster and gives the same steps.
- Keep the "test quote from a simulated partner" note visible on the price tiles.
- Do not approve the order link in the video unless you want a test order in the queue.

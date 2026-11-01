# Run log

One line per build session: date, task, result, Meta API spend (estimated).

2026-10-31 | Task 1: repo, Worker skeleton, photo page, R2 | done: Rust Worker anypartmade-demo, D1 anypartmade-demo, R2 anypartmade-demo-photos with a 7 day expiry rule; page shrinks to 2048 px in the browser, Rust checks format and size; live upload, state, photo and error paths checked; 390 px phone emulation picks a 4032 x 3024 photo, uploads it and shows it with no horizontal scroll | Meta spend: $0.00
2026-11-01 | Task 2 (in progress): SAM 3.1 measuring | built: Rust port of Meta's SAM mask decoder (matches @meta-sam/parser on the documented payloads), SAM reply parser, geometry (coin ellipse, D shaft circle and flat fit, knob radius profile with rib count; within 0.1 mm on synthetic masks), measure step with R2 cache per photo hash, D1 spend log and a 5 dollar daily cap, mask overlay on the page, calipers fallback, ruler scale by two taps; 24 tests; deployed; live ruler flow checked on the oven photo IMG_1257 at 390 px (40 mm over 598 px). Blocked: the Meta API answers 401 Unauthorized for the stored key, so no SAM call has run | Meta spend: $0.00 (401 replies are not billed)

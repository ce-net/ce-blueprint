---
name: ce-blueprint
description: How to use and work on ce-blueprint — the portability planner (chip = data → plan + app-set). Read before adding a chip/capability or editing this repo. Ships with the repo (self-contained).
---

# ce-blueprint — chips are data, not code

Turns a `TargetDescriptor` (a chip described by capability) into (1) a **Plan** — how CE runs on it
(tier D0–D4/Hosted, runtime backend, host-ABI groups, isolation, artifact, delivery) — and (2) a
**Selection** — which concrete ceapps provide the capabilities you want on THAT target, per tier. The
node boundary is a contract, so a new chip enters as data, never a code change.

Full usage + the orchestrator pattern: `README.md`.

## Use it

```bash
ce-blueprint plan esp32-s3                          # how CE runs on this chip (a bundled descriptor)
ce-blueprint plan ./my-board.json                   # or a descriptor file / - for stdin
ce-blueprint apps arduino-uno-q sensor.climate inference   # which apps to install for these capabilities
ce-blueprint list                                   # bundled targets + tiers
ce-blueprint serve                                  # provide capability.blueprint/{plan,apps} over the mesh
```

As a library: `ce_blueprint::generate(&descriptor) -> Plan`, `ce_blueprint::apps::select_apps(&plan,
&desired, &catalog) -> Selection`, and `serve_capability(&ce, shutdown)` for the mesh responder
(topics `CAPABILITY_TOPIC` = `capability.blueprint/plan` and `apps::APPS_TOPIC`).

## Placement is by capability, never by chip name
`has_os` → Hosted (native binary); `!has_crypto` → D0 (peripheral behind an adapter); `wasm_capable`
→ D2/D4 (kernel + wasm firmware); crypto-no-wasm → D1. Peripheral group iff `buses`; Blob iff flash
≥ 4 MiB. See `ce/docs/portability.md` §4 for the porting playbooks.

## Extending — it's all DATA
- **A chip** = a `descriptors/<name>.json`. Fits the axes ⇒ no code change.
- **A capability / app variant** = a `catalog/<capability>.json` (`CapabilityEntry`: the ce-cap
  abilities the app needs + one `variant` per tier band). New chip class for a sensor = a new variant;
  a whole new sense = a new file. Both data — never edit `generate()`/`select_apps()`.
- A genuinely new *rule* (a new capability axis) is a code change HERE (this app), never a substrate change.

## Working on this repo
Self-contained: descriptors + catalog ship in the repo (`include_str!` of `catalog/*.json` is fine —
it is in-repo). No `PLAN/` / `~/ce-net` / `../<sibling>` refs. Tests: `cargo test -p ce-blueprint`
(generator per tier + app-selection + round-trips); `--test capability -- --ignored` for the two mesh
capabilities. Commit as Leif, no co-author, no emojis.

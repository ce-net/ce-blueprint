# ce-blueprint — chips are data, not code

The executable form of the CE **portability boundary** (`ce/docs/portability.md`): the node/peripheral
line is a **contract**, not a device list, so a new chip enters CE as a **descriptor** (data), never a
code change. `ce-blueprint` is the function that turns that descriptor into the **exact
constraint-correct plan** for running CE on the target — which tier, which runtime backend, which
host-ABI groups, which isolation class, what artifact to build, and how to deliver it.

It is a **ceapp**, not substrate: the descriptor schema and the rules live here and version freely;
the OS never learns the concept. `ce onboard` executes the plan; `ce-test`'s `h.arduino()` uses it to
place a real board.

## The model (three axes → one plan)

A target is described by *capabilities*, never by name:

| Descriptor field | Decides |
|---|---|
| `has_os` | **Hosted** tier → the byte-identical native `ce` binary (server/Linux/RPi/Arduino UNO Q MPU/…) |
| `has_crypto` | the **citizenship test** — no on-chip crypto ⇒ **D0** (a peripheral behind an adapter) |
| `wasm_capable` | can run the same `.wasm` ceapps ⇒ **D2+**; else kernel-only **D1** |
| `signed_ota`, `ram_kb` | D2 vs **D4** (multi-app), and the flash transport |
| `buses` | the **Peripheral** host-ABI group |
| `flash_kb` | the **Blob** group |
| `connectivity` | the delivery transport (ota / usb / serial) + routing |

The output `Plan` is `{ tier, runtime, host_abi_groups, isolation, artifact, delivery, notes }`.

## Use

```bash
ce-blueprint plan esp32-s3            # a bundled descriptor by name
ce-blueprint plan ./my-board.json     # a descriptor file
cat my-board.json | ce-blueprint plan -   # stdin
ce-blueprint list                     # the bundled example targets + their tiers
ce-blueprint apps esp32-s3 sensor.climate inference   # which apps to install on this board
ce-blueprint serve                    # provide the blueprint capabilities over the mesh
```

Example — `ce-blueprint plan esp32-s3` yields (abridged):

```json
{
  "target": "esp32-s3",
  "tier": "D4",
  "runtime": ["wasmi"],
  "host_abi_groups": ["Identity","Bus","Runtime","Log","Peripheral","Blob"],
  "isolation": "Wasm",
  "artifact": { "kind": "KernelFirmware", "target": "xtensa-esp32s3-none-elf", "kernel": "ce-device + wasm runtime" },
  "delivery": { "kind": "FlashFirmware", "transport": "ota" },
  "notes": ["...", "Runs the SAME `.wasm` ceapps as any node via wasmi ..."]
}
```

## App selection — which apps to run on this board (`apps` / `capability.blueprint/apps`)

`plan` answers *how CE runs on the chip*. `apps` answers the second half: *given the capabilities you
want the mesh to have, which concrete ceapps go on THIS target* — and that differs by tier. You declare
**intent** (logical capabilities like `sensor.climate`, `sensor.camera`, `inference`); the selector maps
each to the right app for the target:

- **Hosted** (Arduino UNO Q, RPi, a server) → `ce-sensor-climate`, the **script**-tier Python ceapp, copied onto the node.
- **ESP32 (D2/D4)** → the **wasm** build of the same app, delivered as a content-addressed mesh module.
- **A bare no-crypto sensor (D0)** → `ce-arduino-bridge`, an **adapter** running on the host node that holds the device.

A capability with no variant for the tier (a 4K camera or a big model on a $3 chip) comes back
**unresolved** with a reason — surfaced, never silently dropped. The mapping is **data** (`catalog/*.json`,
one `CapabilityEntry` per capability); adding a sensor is adding a catalog file, extending an app to a new
chip class is adding a variant — never editing code. Each placement carries the exact `ce-cap` abilities
its app must be delegated, so an orchestrator grants least privilege by construction.

```bash
$ ce-blueprint apps arduino-uno-q sensor.climate inference
{ "target": "arduino-uno-q", "tier": "Hosted",
  "placements": [
    { "capability": "sensor.climate", "app": "ce-sensor-climate", "runtime": "script",
      "abilities": ["building:climate:read"], "install": "on-target", "notes": [] },
    { "capability": "inference", "app": "ce-exo", "runtime": "native",
      "abilities": ["exo:infer"], "install": "on-target", "notes": [] } ],
  "unresolved": [] }
```

## Over the mesh (the capabilities)

`serve` answers two topics — installing this app teaches the whole mesh to both plan any target AND pick
its app-set (the compounding model: `ce onboard`, a fleet orchestrator, and `ce-test`'s `h.arduino()`
`locate` + call it instead of re-deriving the rules). Both reply `{"error": "..."}` on a bad request.

```rust
// descriptor JSON -> Plan JSON
let plan = ce.request(provider, "capability.blueprint/plan", descriptor_json, 5_000).await?;
// { descriptor, desired: [caps] } JSON -> Selection JSON
let apps = ce.request(provider, "capability.blueprint/apps", apps_req_json, 5_000).await?;
```

## The board-aware orchestrator (why this + `ce-cap` `delegate` = trivial)

These two capabilities are the foundation for **one orchestrator ceapp that installs an app-network on a
fleet, correct-by-target and secure-by-construction** — the thing that used to be a research project is
now gluing two library calls. Given the org cap the orchestrator holds:

```rust
// 1. Ask the board what it is, and which apps provide the capabilities we want on it.
let plan = ce_blueprint::generate(&descriptor);                 // or over the mesh
let sel  = ce_blueprint::apps::select_apps(&plan, &desired, &Catalog::builtin());

// 2. For each app: install it on the target and hand it a cap ATTENUATED from ours —
//    exactly the abilities it needs, scoped to that node, time-boxed. Nothing more.
for p in &sel.placements {
    let child_cap = ce_cap::delegate::delegate(&held_chain, &me, target, &Grant {
        abilities: &p.abilities.iter().map(String::as_str).collect::<Vec<_>>(),
        resource: Some(ce_cap::Resource::Node(target)),
        ttl_secs: 86_400, nonce: next_nonce(),
    }, now)?;
    install_app(target, &p.app, &p.runtime, &child_cap).await?;  // appmgr / mesh deploy
}
```

Run it against an Arduino UNO Q and it installs the script-tier sensors; run it against an ESP32 and it
installs the wasm builds; ask for many nodes and it is a loop — no app code changes, and every spawned
app is delegated strictly less authority than the orchestrator holds. `select_apps` lives here; the
attenuating `delegate` primitive lives in the `ce-cap` crate (`github.com/ce-net/ce`).

## Adding a new chip, or a new capability/app

- **A chip** is a `descriptors/<name>.json`. Fits the existing axes ⇒ **no code changes**.
- **A capability or an app variant** is a `catalog/<capability>.json` ([`CapabilityEntry`]): the abilities
  an app needs + one `variant` per tier band (`{ tiers, app, runtime }`). Teaching the mesh that a sensor
  now runs on a new chip class is adding a variant; adding a whole new sense is adding a file. Both are
  **data** — never an edit to `generate()`/`select_apps()`. A genuinely new *rule* (a new capability axis)
  is a change here in the app, still never a substrate change. See `ce/docs/portability.md` §4 for the
  full porting playbooks.

## Tests

```bash
cargo test -p ce-blueprint          # generator (one test per tier) + app-selection (per-board app-set) + round-trips
cargo test -p ce-blueprint --test capability -- --ignored   # the two capabilities over a real mesh node
```

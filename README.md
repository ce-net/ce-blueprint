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
ce-blueprint serve                    # provide `capability.blueprint/plan` over the mesh
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

## Over the mesh (the capability)

`serve` answers `capability.blueprint/plan`: send a descriptor JSON, get a plan JSON back (or
`{"error": "..."}`). Installing this app teaches the whole mesh to plan any target — the compounding
model: `ce onboard` and other apps `locate` + call it instead of re-deriving the rules.

```rust
let plan_json = ce.request(provider, "capability.blueprint/plan", descriptor_json, 5_000).await?;
```

## Adding a new chip

Add a `descriptors/<name>.json` (or pass your own file). If it fits the existing capability axes, **no
code changes** — that is the whole point. A genuinely new *rule* (a new capability axis) is a change to
`generate()` here in the app, still never a substrate change. See `ce/docs/portability.md` §4 for the
full porting playbooks (new runtime backend, new peripheral driver, tier promotion).

## Tests

```bash
cargo test -p ce-blueprint          # the generator: one test per tier (Hosted/D4/D2/D1/D0) + JSON round-trip
```

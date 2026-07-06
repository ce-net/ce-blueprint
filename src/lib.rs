//! ce-blueprint — turn a **target descriptor** (a chip described as data) into the exact
//! constraint-correct **plan** for running CE on it: which tier, which runtime backend, which host-ABI
//! groups, which isolation class, what artifact to build, and how to deliver it.
//!
//! This is the executable form of `ce/docs/portability.md` §1.4 + §4.1: the node boundary is a
//! **contract, not a device list**, so a new chip enters CE as **data** (a `TargetDescriptor`), never
//! a code change. The generator here is that "data → plan" function; `ce onboard` executes the plan
//! and `ce-test`'s `h.arduino()` uses it to place a real board. Descriptors live as their own files
//! (`descriptors/*.json`) owned by this app — the substrate never learns the concept.
//!
//! ```
//! use ce_blueprint::{TargetDescriptor, Tier, generate};
//! let d: TargetDescriptor = serde_json::from_str(
//!     r#"{"name":"srv","arch":"x86_64-unknown-linux-gnu","has_os":true}"#).unwrap();
//! assert_eq!(generate(&d).tier, Tier::Hosted);
//! ```

use serde::{Deserialize, Serialize};

/// A target described as data. This — not a match on the chip's name — is what places it in CE. Every
/// field is a capability the target either has or lacks; the generator reads *capabilities*, never a
/// model number. Unknown numeric fields default to 0 (treated as "unknown", flagged in the plan).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetDescriptor {
    /// Human label, e.g. "esp32-s3", "raspberry-pi-4". For logs only — never branched on.
    pub name: String,
    /// Rust target triple, e.g. `x86_64-unknown-linux-gnu`, `xtensa-esp32s3-none-elf`.
    pub arch: String,
    /// Has a general-purpose OS (process model + threads + filesystem). If true → Hosted tier: the
    /// byte-identical native `ce` binary, no kernel port needed.
    #[serde(default)]
    pub has_os: bool,
    /// RAM in KiB (0 = unknown).
    #[serde(default)]
    pub ram_kb: u64,
    /// Flash/storage in KiB (0 = unknown).
    #[serde(default)]
    pub flash_kb: u64,
    /// Can hold an Ed25519 key + verify a capability chain on-chip (run the real `ce-identity`/`ce-cap`).
    /// This is the **citizenship test**: no on-chip crypto ⇒ D0 (a peripheral behind an adapter).
    #[serde(default)]
    pub has_crypto: bool,
    /// Can host a wasm interpreter (wasmi/WAMR) ⇒ can run the same cap-gated `.wasm` ceapps ⇒ D2+.
    #[serde(default)]
    pub wasm_capable: bool,
    /// Supports signed, hash-approved OTA (an image flashes only if a signed approval binds its hash).
    #[serde(default)]
    pub signed_ota: bool,
    /// Hardware buses/pins present, e.g. `["gpio","i2c","spi","uart"]`. Drives the Peripheral group.
    #[serde(default)]
    pub buses: Vec<String>,
    /// Links, e.g. `["wifi","ethernet","ble","usb","lora"]`. Drives the delivery transport + routing.
    #[serde(default)]
    pub connectivity: Vec<String>,
}

/// Where the target sits on the D0–D4 ladder (`docs/embedded.md`) — decided by capability, not name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    /// Has an OS → runs the native `ce` binary (server/Linux/RPi/Arduino UNO Q MPU/Win/Mac).
    Hosted,
    /// Bare-metal, wasm + signed OTA + room for many concurrent apps.
    D4,
    /// Bare-metal, wasm + signed OTA, single-app baseline (the "runs the same ceapps" tier).
    D2,
    /// Bare-metal citizen: on-chip identity/link/command, no wasm runtime yet.
    D1,
    /// No device-side crypto → not a node; reached as a peripheral behind an adapter.
    D0,
}

/// A wasm-facing host-ABI capability group (`docs/host-abi.md` §2). A host implements only the groups
/// its hardware offers; the rest answer `unsupported`, fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Group {
    Identity,
    Bus,
    Runtime,
    Peripheral,
    Blob,
    Log,
}

/// The isolation boundary the chosen runtime actually provides (`docs/runtime.md`), vendor-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IsolationClass {
    Native,
    Container,
    Wasm,
    SandboxedContainer,
    MicroVm,
}

/// What to build for the target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Artifact {
    /// The standard `ce` binary for the given Rust target triple.
    NativeBinary { target: String },
    /// The `ce-device` portable kernel (+ a wasm runtime at D2+) linked into firmware.
    KernelFirmware { target: String, kernel: String },
    /// Nothing to build — a D0 device is driven through an adapter on another node.
    None,
}

/// How the artifact reaches the target for the first hop (the ce-onboard repo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Delivery {
    /// Copy the binary + `ce start --no-economy` over ssh / an existing peer relay.
    CopyBinaryThenStart,
    /// Flash firmware over `serial` | `usb` | `ota`.
    FlashFirmware { transport: String },
    /// No delivery: wire the device behind an adapter node that holds authority.
    AdapterProxy,
}

/// The constraint-correct plan for running CE on a target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub target: String,
    pub tier: Tier,
    /// Runtime backends to register (`docs/runtime.md`): e.g. `["wasmtime","docker"]`, `["wasmi"]`, `[]`.
    pub runtime: Vec<String>,
    /// The host-ABI groups this target exposes to wasm apps.
    pub host_abi_groups: Vec<Group>,
    pub isolation: IsolationClass,
    pub artifact: Artifact,
    pub delivery: Delivery,
    /// Human-facing notes + warnings (unknown footprint, security gaps, tier-promotion hints).
    pub notes: Vec<String>,
}

/// Blob-group threshold: content-addressed storage only makes sense with real flash (4 MiB).
const BLOB_FLASH_KB: u64 = 4 * 1024;
/// Multi-app (D4) needs headroom for several concurrent tasks.
const MULTIAPP_RAM_KB: u64 = 512;

/// The one function: a target descriptor in, its constraint-correct plan out. Pure and total — no I/O,
/// no panics. This is "chips are data": adding a target is adding a descriptor, never editing code.
pub fn generate(d: &TargetDescriptor) -> Plan {
    let mut notes = Vec::new();
    if d.ram_kb == 0 || d.flash_kb == 0 {
        notes.push("Footprint unknown (ram_kb/flash_kb = 0): measure on-device; some choices are conservative.".into());
    }

    // Axis 1: has an OS → Hosted. The common case; the identical native binary, no kernel port.
    if d.has_os {
        let mut groups = vec![Group::Identity, Group::Bus, Group::Runtime, Group::Log, Group::Blob];
        if !d.buses.is_empty() {
            groups.push(Group::Peripheral);
        }
        notes.push(format!(
            "Hosted target: runs the byte-identical native `ce` binary for {}. No kernel port needed.",
            d.arch
        ));
        return Plan {
            target: d.name.clone(),
            tier: Tier::Hosted,
            // wasmtime for wasm workloads; docker where a socket exists (reported at runtime).
            runtime: vec!["wasmtime".into(), "docker".into()],
            host_abi_groups: groups,
            // Container is the baseline on a hosted node; stronger classes (gVisor/MicroVm) are opt-in.
            isolation: IsolationClass::Container,
            artifact: Artifact::NativeBinary { target: d.arch.clone() },
            delivery: Delivery::CopyBinaryThenStart,
            notes,
        };
    }

    // Axis 2 (bare metal): the citizenship test. No on-chip crypto → not a node.
    if !d.has_crypto {
        notes.push(
            "No device-side crypto: NOT a node. Reached AS a peripheral behind an adapter on a bigger \
             node (host-abi §4.5); the adapter holds authority. Add a keystore + ce-identity to reach D1."
                .into(),
        );
        return Plan {
            target: d.name.clone(),
            tier: Tier::D0,
            runtime: vec![],
            host_abi_groups: vec![],
            isolation: IsolationClass::Native,
            artifact: Artifact::None,
            delivery: Delivery::AdapterProxy,
            notes,
        };
    }

    // Axis 3: a bare-metal citizen. wasm-capable → runs ceapps (D2/D4); else kernel-only (D1).
    let transport = flash_transport(d);
    if !d.signed_ota {
        notes.push(
            "No signed OTA: firmware updates are unauthenticated — close before production (docs/embedded.md D2 wants hash-approved OTA)."
                .into(),
        );
    }

    if d.wasm_capable {
        let tier = if d.signed_ota && d.ram_kb >= MULTIAPP_RAM_KB {
            notes.push(format!(
                "≥{MULTIAPP_RAM_KB} KiB RAM + signed OTA → D4: multiple cap-gated apps concurrently (one task each), targetable by `ce install device=<id>`."
            ));
            Tier::D4
        } else {
            if d.signed_ota && d.ram_kb > 0 && d.ram_kb < MULTIAPP_RAM_KB {
                notes.push(format!("D2 single-app baseline; ≥{MULTIAPP_RAM_KB} KiB RAM would reach D4 (multi-app)."));
            }
            Tier::D2
        };
        let mut groups = vec![Group::Identity, Group::Bus, Group::Runtime, Group::Log];
        if !d.buses.is_empty() {
            groups.push(Group::Peripheral);
        }
        if d.flash_kb >= BLOB_FLASH_KB {
            groups.push(Group::Blob);
        }
        notes.push(format!(
            "Runs the SAME `.wasm` ceapps as any node via {} — same imports, same cap enforcement (docs/host-abi.md).",
            runtime_backend(d)
        ));
        return Plan {
            target: d.name.clone(),
            tier,
            runtime: vec![runtime_backend(d).into()],
            host_abi_groups: groups,
            isolation: IsolationClass::Wasm,
            artifact: Artifact::KernelFirmware {
                target: d.arch.clone(),
                kernel: "ce-device + wasm runtime".into(),
            },
            delivery: Delivery::FlashFirmware { transport },
            notes,
        };
    }

    // D1: crypto but no wasm — a real mesh citizen via the ce-device kernel (identity/link/command),
    // no wasm app groups yet.
    notes.push(
        "D1: runs the `ce-device` kernel (own identity + signed wire + on-chip cap verify) — a real \
         mesh citizen, not a router. No wasm app groups until it can host an interpreter (→ D2)."
            .into(),
    );
    Plan {
        target: d.name.clone(),
        tier: Tier::D1,
        runtime: vec![],
        host_abi_groups: vec![],
        isolation: IsolationClass::Native,
        artifact: Artifact::KernelFirmware {
            target: d.arch.clone(),
            kernel: "ce-device".into(),
        },
        delivery: Delivery::FlashFirmware { transport },
        notes,
    }
}

/// Which on-chip wasm interpreter to bind the host-ABI on. Same imports either way (`docs/host-abi.md`
/// §7); RISC-V and Xtensa both run wasmi today, with WAMR as the ESP-IDF alternative.
fn runtime_backend(_d: &TargetDescriptor) -> &'static str {
    // wasmi is the proven, pure-Rust `no_std` backend across ISAs; WAMR is an optional ESP-IDF swap.
    "wasmi"
}

/// Pick the first-hop flash transport from the target's links: prefer signed OTA over wifi/ethernet,
/// then USB, else serial.
fn flash_transport(d: &TargetDescriptor) -> String {
    let has = |x: &str| d.connectivity.iter().any(|c| c == x) || d.buses.iter().any(|b| b == x);
    if d.signed_ota && (has("wifi") || has("ethernet")) {
        "ota".into()
    } else if has("usb") {
        "usb".into()
    } else {
        "serial".into()
    }
}

// ---- the mesh capability (so tests + other ceapps drive the real planner over the Bus) ----

/// The mesh topic ce-blueprint answers: a `TargetDescriptor` JSON in, a `Plan` JSON out.
pub const CAPABILITY_TOPIC: &str = "capability.blueprint/plan";

/// The capability responder: parses a descriptor and returns its plan (or `{"error": …}`).
pub struct BlueprintService;

impl ce_rs::serve::Handler for BlueprintService {
    async fn handle(&self, req: ce_rs::serve::Request) -> Vec<u8> {
        match serde_json::from_slice::<TargetDescriptor>(&req.payload) {
            Ok(d) => serde_json::to_vec(&generate(&d)).unwrap_or_default(),
            Err(e) => serde_json::to_vec(&serde_json::json!({ "error": format!("invalid descriptor: {e}") }))
                .unwrap_or_default(),
        }
    }
}

/// Serve `capability.blueprint/plan` on `ce` until `shutdown` resolves — exactly what `ce-blueprint
/// serve` runs. Call it from a test to stand up the real capability on a harness node.
pub async fn serve_capability<F>(ce: &ce_rs::CeClient, shutdown: F) -> anyhow::Result<()>
where
    F: std::future::Future<Output = ()>,
{
    ce_rs::serve::serve(ce, &[CAPABILITY_TOPIC], &BlueprintService, shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(name: &str, arch: &str) -> TargetDescriptor {
        TargetDescriptor {
            name: name.into(),
            arch: arch.into(),
            has_os: false,
            ram_kb: 0,
            flash_kb: 0,
            has_crypto: false,
            wasm_capable: false,
            signed_ota: false,
            buses: vec![],
            connectivity: vec![],
        }
    }

    #[test]
    fn hosted_target_gets_the_native_binary() {
        let mut t = d("srv", "x86_64-unknown-linux-gnu");
        t.has_os = true;
        t.ram_kb = 8 * 1024 * 1024;
        t.flash_kb = 256 * 1024 * 1024;
        let p = generate(&t);
        assert_eq!(p.tier, Tier::Hosted);
        assert_eq!(p.artifact, Artifact::NativeBinary { target: "x86_64-unknown-linux-gnu".into() });
        assert_eq!(p.delivery, Delivery::CopyBinaryThenStart);
        assert!(p.host_abi_groups.contains(&Group::Identity) && p.host_abi_groups.contains(&Group::Blob));
    }

    #[test]
    fn arduino_uno_q_mpu_is_hosted_like_debian() {
        // The UNO Q's Linux MPU is a hosted target — identical native binary, per CLAUDE.md.
        let mut t = d("arduino-uno-q", "aarch64-unknown-linux-gnu");
        t.has_os = true;
        t.buses = vec!["gpio".into(), "i2c".into()];
        let p = generate(&t);
        assert_eq!(p.tier, Tier::Hosted);
        assert!(p.host_abi_groups.contains(&Group::Peripheral), "buses present → Peripheral group");
    }

    #[test]
    fn esp32_class_runs_the_kernel_plus_wasm() {
        let mut t = d("esp32-s3", "xtensa-esp32s3-none-elf");
        t.has_crypto = true;
        t.wasm_capable = true;
        t.signed_ota = true;
        t.ram_kb = 512;
        t.flash_kb = 8 * 1024;
        t.buses = vec!["gpio".into(), "i2c".into(), "uart".into()];
        t.connectivity = vec!["wifi".into()];
        let p = generate(&t);
        assert_eq!(p.tier, Tier::D4, "512 KiB + signed OTA → multi-app D4");
        assert_eq!(p.runtime, vec!["wasmi".to_string()]);
        assert_eq!(p.isolation, IsolationClass::Wasm);
        assert!(matches!(p.artifact, Artifact::KernelFirmware { .. }));
        assert_eq!(p.delivery, Delivery::FlashFirmware { transport: "ota".into() });
        assert!(p.host_abi_groups.contains(&Group::Peripheral));
        assert!(p.host_abi_groups.contains(&Group::Blob), "8 MiB flash → Blob group");
    }

    #[test]
    fn small_wasm_chip_without_ota_is_d2_with_a_warning() {
        let mut t = d("esp32-c3", "riscv32imc-unknown-none-elf");
        t.has_crypto = true;
        t.wasm_capable = true;
        t.signed_ota = false;
        t.ram_kb = 400;
        t.flash_kb = 2 * 1024; // < 4 MiB → no Blob group
        t.connectivity = vec!["wifi".into()];
        let p = generate(&t);
        assert_eq!(p.tier, Tier::D2);
        assert!(!p.host_abi_groups.contains(&Group::Blob));
        assert_eq!(p.delivery, Delivery::FlashFirmware { transport: "serial".into() }, "no signed OTA → not ota");
        assert!(p.notes.iter().any(|n| n.contains("No signed OTA")));
    }

    #[test]
    fn crypto_but_no_wasm_is_a_d1_citizen() {
        let mut t = d("stm32-secure", "thumbv7em-none-eabihf");
        t.has_crypto = true;
        t.wasm_capable = false;
        t.buses = vec!["uart".into()];
        let p = generate(&t);
        assert_eq!(p.tier, Tier::D1);
        assert!(p.host_abi_groups.is_empty(), "no wasm → no wasm-facing groups");
        assert_eq!(p.artifact, Artifact::KernelFirmware { target: "thumbv7em-none-eabihf".into(), kernel: "ce-device".into() });
    }

    #[test]
    fn no_crypto_sensor_is_a_d0_peripheral() {
        let mut t = d("i2c-temp-sensor", "none");
        t.has_crypto = false;
        t.buses = vec!["i2c".into()];
        let p = generate(&t);
        assert_eq!(p.tier, Tier::D0);
        assert_eq!(p.artifact, Artifact::None);
        assert_eq!(p.delivery, Delivery::AdapterProxy);
        assert!(p.host_abi_groups.is_empty());
        assert!(p.notes.iter().any(|n| n.contains("NOT a node")));
    }

    #[test]
    fn plan_round_trips_through_json() {
        let mut t = d("esp32-s3", "xtensa-esp32s3-none-elf");
        t.has_crypto = true;
        t.wasm_capable = true;
        let p = generate(&t);
        let s = serde_json::to_string(&p).unwrap();
        let back: Plan = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}

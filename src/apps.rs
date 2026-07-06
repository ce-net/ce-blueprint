//! Target → app-set selection: given a target's [`Plan`] and the capabilities an operator wants the
//! mesh to have, choose the **concrete ceapps** to install on that target — and note what it can't do.
//!
//! This is the second half of "chips are data". [`generate`](crate::generate) answers *how CE runs
//! on this chip* (tier/runtime/artifact). This answers *which apps provide the wanted capabilities on
//! this chip* — and the answer differs by tier: a climate sensor is the `ce-sensor-climate` **script**
//! ceapp on a Hosted board (Arduino UNO Q, RPi, a server), the **wasm** build of the same app on an
//! ESP32 (D2/D4), and — on a bare no-crypto sensor (D0) — the `ce-arduino-bridge` **adapter** running
//! on the host node that holds it. The operator declares intent ("I want climate + camera + inference");
//! the selector maps it to apps per target. It never targets a machine; it selects by capability.
//!
//! The mapping is **data** ([`Catalog`], shipped as `catalog/*.json`): adding a sensor is adding a
//! catalog entry, extending an app to a new chip class is adding a variant — never editing this code.
//! An orchestrator ceapp then installs each [`AppPlacement`], delegating it exactly its `abilities`
//! (see the `ce-cap` `delegate` primitive) so every spawned app gets least privilege by construction.
//!
//! ```
//! use ce_blueprint::{generate, TargetDescriptor};
//! use ce_blueprint::apps::{select_apps, Catalog, Desired};
//! let uno: TargetDescriptor = serde_json::from_str(
//!     r#"{"name":"uno-q","arch":"aarch64-unknown-linux-gnu","has_os":true,"buses":["i2c"]}"#).unwrap();
//! let sel = select_apps(&generate(&uno), &[Desired::new("sensor.climate")], &Catalog::builtin());
//! assert_eq!(sel.placements[0].app, "ce-sensor-climate");
//! assert_eq!(sel.placements[0].runtime, "script"); // Hosted → the script-tier Python ceapp
//! ```

use serde::{Deserialize, Serialize};

use crate::{Plan, Tier};

/// A capability the operator wants present on a target — logical, target-independent intent (e.g.
/// `sensor.climate`, `sensor.camera`, `inference`). Never names an app; the [`Catalog`] resolves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Desired {
    pub capability: String,
}

impl Desired {
    pub fn new(capability: impl Into<String>) -> Self {
        Self { capability: capability.into() }
    }
}

/// One concrete ceapp that provides a capability on a band of tiers. Data, not code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variant {
    /// The tiers this variant serves. A target is placed on the variant whose band contains its tier.
    pub tiers: Vec<Tier>,
    /// The ceapp that provides the capability here (name / repo), e.g. `ce-sensor-climate`.
    pub app: String,
    /// How it runs on this tier: `script` | `wasm` | `native` | `adapter`.
    pub runtime: String,
}

/// A capability and the concrete apps that provide it across tiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEntry {
    pub capability: String,
    /// The `ce-cap` abilities an app providing this capability must be delegated to function (e.g.
    /// `["building:climate:read"]`). An orchestrator attenuates its own held cap down to exactly these.
    #[serde(default)]
    pub abilities: Vec<String>,
    pub variants: Vec<Variant>,
}

/// The app catalog: which ceapps provide which capabilities, per tier. Ships as data (`catalog/*.json`),
/// replaceable and extensible without touching this crate — the same way descriptors are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub entries: Vec<CapabilityEntry>,
}

/// One resolved placement: install `app` (running as `runtime`) for `capability` on the target, and
/// delegate it exactly `abilities`. `install` is a hint for the orchestrator on how the app reaches
/// the target given its tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPlacement {
    pub capability: String,
    pub app: String,
    pub runtime: String,
    pub abilities: Vec<String>,
    /// How the app is delivered on this tier: `on-target` (copy/install onto the node), `wasm-module`
    /// (content-addressed module the node fetches and runs under its wasm runtime), or `adapter-on-host`
    /// (the target is a D0 peripheral — the app runs as an adapter on the host node that holds it).
    pub install: String,
    pub notes: Vec<String>,
}

/// A desired capability the catalog cannot provide on this target's tier — surfaced, never dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unresolved {
    pub capability: String,
    pub reason: String,
}

/// The result of selecting apps for a target: concrete placements plus anything unresolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub target: String,
    pub tier: Tier,
    pub placements: Vec<AppPlacement>,
    pub unresolved: Vec<Unresolved>,
}

/// Given a target's [`Plan`] and the desired capabilities, choose the concrete app-set. Pure and
/// total: no I/O, no panics. Every desired capability yields either an [`AppPlacement`] (a catalog
/// variant serving this tier) or an [`Unresolved`] entry explaining why it can't run here.
pub fn select_apps(plan: &Plan, desired: &[Desired], catalog: &Catalog) -> Selection {
    let mut placements = Vec::new();
    let mut unresolved = Vec::new();

    for d in desired {
        let Some(entry) = catalog.entries.iter().find(|e| e.capability == d.capability) else {
            unresolved.push(Unresolved {
                capability: d.capability.clone(),
                reason: "no catalog entry for this capability".into(),
            });
            continue;
        };
        let Some(variant) = entry.variants.iter().find(|v| v.tiers.contains(&plan.tier)) else {
            unresolved.push(Unresolved {
                capability: d.capability.clone(),
                reason: format!(
                    "no variant of `{}` runs on tier {:?}; place this capability on a capable node instead",
                    entry.capability, plan.tier
                ),
            });
            continue;
        };

        let install = install_mode(&variant.runtime);
        let mut notes = Vec::new();
        if install == "adapter-on-host" {
            notes.push(
                "target is a D0 peripheral: this app runs as an adapter on the host node that holds \
                 the device; the adapter is delegated the authority, not the peripheral."
                    .into(),
            );
        }
        placements.push(AppPlacement {
            capability: entry.capability.clone(),
            app: variant.app.clone(),
            runtime: variant.runtime.clone(),
            abilities: entry.abilities.clone(),
            install,
            notes,
        });
    }

    Selection { target: plan.target.clone(), tier: plan.tier, placements, unresolved }
}

/// How a runtime is delivered on its tier. Kept trivial and total — the runtime string already
/// encodes it; this just names the delivery the orchestrator performs.
fn install_mode(runtime: &str) -> String {
    match runtime {
        "wasm" => "wasm-module",
        "adapter" => "adapter-on-host",
        _ => "on-target", // script | native | anything host-run
    }
    .to_string()
}

impl Catalog {
    /// The catalog bundled with this crate (the `catalog/*.json` files, embedded). Used when no
    /// external catalog dir is provided — so the mesh capability works regardless of cwd.
    pub fn builtin() -> Self {
        // Embedded so the binary is self-contained; the files remain the editable source of truth.
        const FILES: &[&str] = &[
            include_str!("../catalog/sensor.climate.json"),
            include_str!("../catalog/sensor.camera.json"),
            include_str!("../catalog/sensor.led.json"),
            include_str!("../catalog/inference.json"),
        ];
        let entries = FILES
            .iter()
            .filter_map(|s| serde_json::from_str::<CapabilityEntry>(s).ok())
            .collect();
        Catalog { entries }
    }

    /// Load a catalog from a directory of `*.json` [`CapabilityEntry`] files, falling back to
    /// [`Catalog::builtin`] if the directory is absent or empty. Mirrors how descriptors load.
    pub fn load(dir: &std::path::Path) -> Self {
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.path().extension().and_then(|x| x.to_str()) == Some("json") {
                    if let Ok(text) = std::fs::read_to_string(e.path()) {
                        if let Ok(entry) = serde_json::from_str::<CapabilityEntry>(&text) {
                            entries.push(entry);
                        }
                    }
                }
            }
        }
        if entries.is_empty() { Catalog::builtin() } else { Catalog { entries } }
    }
}

// ---- the mesh capability (drive the real selector over the Bus) ----

/// The mesh topic the app-selector answers: an [`AppsRequest`] JSON in, a [`Selection`] JSON out.
pub const APPS_TOPIC: &str = "capability.blueprint/apps";

/// Request payload for [`APPS_TOPIC`]: a target descriptor + the capabilities the operator wants.
#[derive(Debug, Clone, Deserialize)]
pub struct AppsRequest {
    pub descriptor: crate::TargetDescriptor,
    /// Desired capability names (e.g. `["sensor.climate","inference"]`).
    #[serde(default)]
    pub desired: Vec<String>,
}

/// Resolve an [`AppsRequest`] against the given catalog: plan the target, then select its apps.
pub fn resolve(req: &AppsRequest, catalog: &Catalog) -> Selection {
    let plan = crate::generate(&req.descriptor);
    let desired: Vec<Desired> = req.desired.iter().map(|c| Desired::new(c.clone())).collect();
    select_apps(&plan, &desired, catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TargetDescriptor, generate};

    fn hosted(name: &str) -> Plan {
        let d: TargetDescriptor = serde_json::from_str(&format!(
            r#"{{"name":"{name}","arch":"aarch64-unknown-linux-gnu","has_os":true,"buses":["i2c"]}}"#
        ))
        .unwrap();
        generate(&d)
    }

    fn esp32_s3() -> Plan {
        let d: TargetDescriptor = serde_json::from_str(
            r#"{"name":"esp32-s3","arch":"xtensa-esp32s3-none-elf","has_crypto":true,"wasm_capable":true,"signed_ota":true,"ram_kb":512,"flash_kb":8192,"buses":["i2c"],"connectivity":["wifi"]}"#,
        )
        .unwrap();
        generate(&d)
    }

    fn d0_sensor() -> Plan {
        let d: TargetDescriptor = serde_json::from_str(
            r#"{"name":"i2c-temp","arch":"none","has_crypto":false,"buses":["i2c"]}"#,
        )
        .unwrap();
        generate(&d)
    }

    #[test]
    fn same_capability_picks_different_apps_per_board() {
        let cat = Catalog::builtin();
        let want = [Desired::new("sensor.climate")];

        // Hosted (UNO Q): the script-tier Python ceapp, copied onto the node.
        let uno = select_apps(&hosted("uno-q"), &want, &cat);
        assert_eq!(uno.placements.len(), 1);
        assert_eq!(uno.placements[0].app, "ce-sensor-climate");
        assert_eq!(uno.placements[0].runtime, "script");
        assert_eq!(uno.placements[0].install, "on-target");
        assert_eq!(uno.placements[0].abilities, vec!["building:climate:read"]);

        // ESP32-S3 (D4): the SAME app, wasm build, delivered as a mesh module.
        let esp = select_apps(&esp32_s3(), &want, &cat);
        assert_eq!(esp.placements[0].app, "ce-sensor-climate");
        assert_eq!(esp.placements[0].runtime, "wasm");
        assert_eq!(esp.placements[0].install, "wasm-module");

        // D0 sensor: reached through the arduino-bridge adapter on the host node.
        let d0 = select_apps(&d0_sensor(), &want, &cat);
        assert_eq!(d0.placements[0].app, "ce-arduino-bridge");
        assert_eq!(d0.placements[0].runtime, "adapter");
        assert_eq!(d0.placements[0].install, "adapter-on-host");
        assert!(d0.placements[0].notes.iter().any(|n| n.contains("peripheral")));
    }

    #[test]
    fn a_whole_capability_set_resolves_per_target() {
        let cat = Catalog::builtin();
        let want = [Desired::new("sensor.climate"), Desired::new("sensor.camera"), Desired::new("inference")];

        // A Hosted node can see, hear, AND think — all three resolve.
        let uno = select_apps(&hosted("uno-q"), &want, &cat);
        assert_eq!(uno.placements.len(), 3);
        assert!(uno.unresolved.is_empty());

        // A tiny ESP32 can sense climate but not run 4K camera or a big model — surfaced, not dropped.
        let esp = select_apps(&esp32_s3(), &want, &cat);
        let apps: Vec<&str> = esp.placements.iter().map(|p| p.app.as_str()).collect();
        assert!(apps.contains(&"ce-sensor-climate"));
        let unresolved: Vec<&str> = esp.unresolved.iter().map(|u| u.capability.as_str()).collect();
        assert!(unresolved.contains(&"sensor.camera"), "camera has no ESP32 variant → unresolved");
        assert!(unresolved.contains(&"inference"), "inference is Hosted-only → unresolved");
        assert!(esp.unresolved.iter().all(|u| !u.reason.is_empty()));
    }

    #[test]
    fn unknown_capability_is_unresolved_not_a_crash() {
        let sel = select_apps(&hosted("srv"), &[Desired::new("sensor.smell")], &Catalog::builtin());
        assert!(sel.placements.is_empty());
        assert_eq!(sel.unresolved.len(), 1);
        assert!(sel.unresolved[0].reason.contains("no catalog entry"));
    }

    #[test]
    fn resolve_over_the_wire_shape_round_trips() {
        let req: AppsRequest = serde_json::from_str(
            r#"{"descriptor":{"name":"uno-q","arch":"aarch64-unknown-linux-gnu","has_os":true,"buses":["i2c"]},"desired":["sensor.climate"]}"#,
        )
        .unwrap();
        let sel = resolve(&req, &Catalog::builtin());
        let s = serde_json::to_string(&sel).unwrap();
        let back: Selection = serde_json::from_str(&s).unwrap();
        assert_eq!(sel, back);
        assert_eq!(back.placements[0].app, "ce-sensor-climate");
    }
}

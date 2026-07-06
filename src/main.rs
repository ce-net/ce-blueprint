//! ce-blueprint — CLI + mesh capability over the `ce_blueprint` generator.
//!
//! - `ce-blueprint plan <descriptor.json | name | ->` — print the constraint-correct plan for a target
//!   (a path, a bundled `descriptors/<name>.json`, or `-` for stdin).
//! - `ce-blueprint list`                              — list the bundled example descriptors.
//! - `ce-blueprint serve`                             — provide `capability.blueprint/plan` over the
//!   mesh: send a descriptor JSON, get a plan JSON back. Installing this teaches the mesh to plan any
//!   target — `ce onboard` and `ce-test`'s `h.arduino()` call it instead of re-deriving the rules.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ce_rs::CeClient;

// The generator + the mesh capability both live in the lib so tests/other ceapps drive the real code.
use ce_blueprint::{generate, serve_capability, TargetDescriptor, CAPABILITY_TOPIC};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("plan") => {
            let src = args.get(2).ok_or_else(|| anyhow!("usage: ce-blueprint plan <descriptor.json | name | ->"))?;
            let json = read_descriptor(src)?;
            let d: TargetDescriptor = serde_json::from_str(&json).context("parse descriptor JSON")?;
            let plan = generate(&d);
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        Some("list") => {
            let dir = descriptors_dir();
            let mut found = false;
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if e.path().extension().and_then(|x| x.to_str()) == Some("json") {
                        found = true;
                        let stem = e.path().file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                        if let Ok(text) = std::fs::read_to_string(e.path()) {
                            if let Ok(d) = serde_json::from_str::<TargetDescriptor>(&text) {
                                println!("{:<22} {:<32} tier={:?}", stem, d.arch, generate(&d).tier);
                                continue;
                            }
                        }
                        println!("{stem}");
                    }
                }
            }
            if !found {
                eprintln!("no descriptors found in {}", dir.display());
            }
            Ok(())
        }
        Some("serve") => run_serve(CeClient::local()).await,
        _ => {
            eprintln!("usage: ce-blueprint [plan <descriptor.json | name | -> | list | serve]");
            Ok(())
        }
    }
}

/// Read a descriptor from a path, a bundled `descriptors/<name>.json`, or stdin (`-`).
fn read_descriptor(src: &str) -> Result<String> {
    if src == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).context("read stdin")?;
        return Ok(s);
    }
    let p = Path::new(src);
    if p.exists() {
        return std::fs::read_to_string(p).with_context(|| format!("read {src}"));
    }
    // Treat as a bundled descriptor name.
    let bundled = descriptors_dir().join(format!("{src}.json"));
    if bundled.exists() {
        return std::fs::read_to_string(&bundled).with_context(|| format!("read {}", bundled.display()));
    }
    Err(anyhow!("no descriptor at '{src}' (not a file, not descriptors/{src}.json)"))
}

/// The bundled descriptors dir: `$CE_BLUEPRINT_DESCRIPTORS`, else `descriptors/` next to the cwd.
fn descriptors_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CE_BLUEPRINT_DESCRIPTORS") {
        return PathBuf::from(d);
    }
    PathBuf::from("descriptors")
}

// ---- mesh capability ----

async fn run_serve(ce: CeClient) -> Result<()> {
    let id = ce.status().await.map(|s| s.node_id).unwrap_or_default();
    let short = id.get(..16).unwrap_or(&id);
    println!("ce-blueprint providing `{CAPABILITY_TOPIC}` on node {short}… — send a descriptor, get a plan");
    serve_capability(&ce, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

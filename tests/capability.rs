//! T3 (module↔module comms): the ce-blueprint capability answers `capability.blueprint/plan` over a
//! real node's Bus — a consumer sends a `TargetDescriptor` JSON and gets the constraint-correct `Plan`
//! JSON back. Drives the REAL `serve_capability` (the same code `ce-blueprint serve` runs).
//!
//! Spawns a real `ce` node via ce-test, so it's `#[ignore]`. Run with:
//!   cargo test -p ce-blueprint --test capability -- --ignored --nocapture

use std::time::Duration;

use ce_test::Harness;
use tokio::sync::oneshot;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "spawns a real `ce` node; run explicitly with --ignored"]
async fn plans_a_target_over_the_mesh() {
    let mut h = Harness::new();
    let node = h.node().await.expect("harness node");

    // Stand up the REAL capability responder on the node.
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let ce = node.client.clone();
    let provider = tokio::spawn(async move {
        let _ = ce_blueprint::serve_capability(&ce, async {
            let _ = stop_rx.await;
        })
        .await;
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A consumer asks the capability to plan an ESP32-class target.
    let descriptor = br#"{"name":"esp32-s3","arch":"xtensa-esp32s3-none-elf","has_crypto":true,"wasm_capable":true,"signed_ota":true,"ram_kb":512,"flash_kb":8192,"buses":["gpio"],"connectivity":["wifi"]}"#;
    let reply = node
        .request(&node.node_id, ce_blueprint::CAPABILITY_TOPIC, descriptor, 10_000)
        .await
        .expect("plan over the mesh");
    let plan: ce_blueprint::Plan = serde_json::from_slice(&reply).expect("reply is a Plan");
    assert_eq!(plan.tier, ce_blueprint::Tier::D4, "512 KiB + signed OTA → D4");
    assert_eq!(plan.runtime, vec!["wasmi".to_string()]);

    // A malformed descriptor gets a JSON error, not a crash.
    let err = node
        .request(&node.node_id, ce_blueprint::CAPABILITY_TOPIC, b"not json", 10_000)
        .await
        .expect("error reply");
    let v: serde_json::Value = serde_json::from_slice(&err).expect("error is JSON");
    assert!(v.get("error").is_some(), "malformed descriptor → error object");

    let _ = stop_tx.send(());
    provider.abort();
}

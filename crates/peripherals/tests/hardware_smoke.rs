//! Smoke test against REAL hardware. `#[ignore]` on purpose: it needs a powered printer on the LAN,
//! so it cannot run in CI — it is run by hand when there is one at hand.
//!
//! ```bash
//! cargo test -p erplora-peripherals --test hardware_smoke -- --ignored --nocapture
//! ```
//!
//! It exists because the Android emulator cannot do **discovery**, and it pays to be precise about
//! why — measured against a real thermal printer at `192.168.100.196:9100`:
//!
//!  - **Getting out DOES work.** The emulator's NAT routes the outgoing TCP connection through the
//!    host, so a LAN IP is reachable from `10.0.2.15`: `nc -w 3 -z 192.168.100.196 9100` → OK. So
//!    **printing by IP to an already known printer works in the emulator**.
//!  - **Finding it does NOT.** The /24 sweep takes its prefix from the local IP, which there is
//!    `10.0.2` — it sweeps the emulator's network, not the real LAN. And mDNS is multicast, which
//!    the NAT does not cross.
//!
//! So the emulator is good for the print path but not for discovery, and the only way to verify
//! THAT is a real printer at hand.
//!
//! It uses paper: `smoke_prints_a_test_page` only prints when asked with `ERPLORA_SMOKE_PRINT=1`,
//! so the printer can be looked for without leaving tickets on the floor.

use erplora_peripherals::discovery::{discover_printers, parse_printer_id, LocalNetworkAccess};
use erplora_peripherals::escpos::render_test_page;
use erplora_peripherals::queue::{PrintJob, PrintQueue, RetryPolicy};
use erplora_peripherals::registry::DeviceRegistry;

/// Registry in a temp dir: the smoke test must not overwrite the machine's real `devices.json`.
/// `nombre` makes it unique per test, since the tests share a process.
fn registro_temporal(nombre: &str) -> DeviceRegistry {
    let dir = std::env::temp_dir().join(format!("erplora-smoke-{}-{nombre}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the temp dir");
    DeviceRegistry::load(dir.join("devices.json"))
}

#[tokio::test]
#[ignore = "needs a powered network printer on the LAN"]
async fn smoke_discovers_the_lan_printer() {
    let registry = registro_temporal("descubre");

    // With a printer at hand the permission is granted by definition; if it were not, the outcome
    // would say so instead of returning an empty list.
    let encontradas = discover_printers(&registry, LocalNetworkAccess::Granted)
        .await
        .expect("discovery must not fail")
        .scanned_printers()
        .expect("with the permission granted the scan DOES run")
        .to_vec();

    for p in &encontradas {
        println!("  · {} — {} [{}] {}mm", p.id, p.name, p.category, p.paper_width);
    }
    assert!(
        !encontradas.is_empty(),
        "no printer on the LAN. Mind: zero is NOT a bug in the code — a firewall, a VLAN \
         or a printer that is off give exactly this result, with no exception."
    );

    // The id IS the contract with the frontend and with `devices.json`: if it stops being
    // parseable, roles (kitchen/bar/till) can no longer be assigned.
    for p in &encontradas {
        parse_printer_id(&p.id).unwrap_or_else(|e| panic!("unparseable id {}: {e}", p.id));
    }
}

#[tokio::test]
#[ignore = "needs a powered network printer on the LAN"]
async fn smoke_discovery_registers_the_devices() {
    // If discovery does not register, `devices.json` stays empty, `erplora_get_devices` returns []
    // and roles CANNOT be assigned — the printing module is useless and not a single error says
    // why.
    let registry = registro_temporal("registra");

    let encontradas = discover_printers(&registry, LocalNetworkAccess::Granted)
        .await
        .expect("discovery")
        .scanned_printers()
        .expect("with the permission granted the scan DOES run")
        .to_vec();
    assert!(!encontradas.is_empty(), "without a printer the registry cannot be checked");

    let registrados = registry.get_all();
    assert!(
        !registrados.is_empty(),
        "discovered {} printer(s) but registered 0: roles could not be assigned",
        encontradas.len()
    );
    for d in &registrados {
        println!("  · key={} mac={:?} ip={}", d.key, d.mac, d.ip);
        assert!(!d.key.is_empty(), "a device without a key cannot be addressed");
    }
}

#[tokio::test]
#[ignore = "PRINTS ON PAPER: needs ERPLORA_SMOKE_PRINT=1"]
async fn smoke_prints_a_test_page() {
    if std::env::var("ERPLORA_SMOKE_PRINT").ok().as_deref() != Some("1") {
        println!("skipped: export ERPLORA_SMOKE_PRINT=1 to use real paper");
        return;
    }
    let registry = registro_temporal("imprime");
    let encontradas = discover_printers(&registry, LocalNetworkAccess::Granted)
        .await
        .expect("discovery")
        .scanned_printers()
        .expect("with the permission granted the scan DOES run")
        .to_vec();
    let destino = encontradas
        .iter()
        // An office A4 printer listens on 9100 too but speaks PCL/PostScript: sending it ESC/POS
        // spits out pages of garbage. Only what has NOT been identified as A4 is printed to.
        .find(|p| p.category != "a4")
        .expect("no thermal printer found");

    println!("printing to {} ({})", destino.id, destino.name);
    let target = parse_printer_id(&destino.id).expect("parseable id");
    let bytes = render_test_page(&destino.id, &serde_json::json!({}));
    // Through the SAME path as the installable app: `erplora_print` queues and the `PrintQueue`
    // worker sends. The worker is not needed here — `send_once` is the send that worker does.
    PrintQueue::new(RetryPolicy::default())
        .send_once(&PrintJob {
            job_id: None,
            target,
            payload: bytes.clone(),
            attempts: 0,
        })
        .await
        .expect("the print job must reach the printer");
    println!("sent {} bytes of ESC/POS", bytes.len());
}

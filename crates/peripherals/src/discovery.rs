//! Network printer discovery.
//!
//! Only the **network** discovery lives here: the OS's USB print queues are listed by
//! [`crate::usb`], the bonded Bluetooth printers by the Kotlin plugin, and a `printer_id` is
//! parsed by [`crate::print_target`].
//!
//! Methods: a /24 subnet sweep on port 9100 ([`sweep`]) plus mDNS (`_pdl-datastream._tcp`,
//! `_ipp._tcp`; [`mdns`]), deduplicated and enriched with the MAC (ARP) so they can be registered
//! in `registry::DeviceRegistry`.

pub mod mdns;
pub mod sweep;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use self::mdns::discover_mdns;
use self::sweep::discover_subnet_scan;
use crate::print_target::parse_printer_id;
use crate::protocol::PrinterInfo;
use crate::registry::DeviceRegistry;
use crate::{Result, ESCPOS_NETWORK_PORT};

/// What the OS lets this process do on the local network — the one thing printer discovery cannot
/// work around, and the one every platform gates somewhere else: an Android runtime permission
/// (`ACCESS_LOCAL_NETWORK`, API 37+) or the macOS 15 local-network consent. This library cannot ask
/// the OS itself, so the shell that *can* states it here, on the way in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalNetworkAccess {
    /// Granted, or the platform has no such gate at all. The scan runs.
    Granted,
    /// The OS refused. The scan is deliberately NOT run: it could only ever come back empty, and
    /// an empty list reads exactly like "this venue has no printers".
    Denied { permission: String },
}

impl LocalNetworkAccess {
    /// The permission standing between us and the printers, if any. `None` = go ahead and look.
    pub fn blocked_by(&self) -> Option<&str> {
        match self {
            Self::Granted => None,
            Self::Denied { permission } => Some(permission),
        }
    }
}

/// Outcome of a discovery run.
///
/// It exists because `Vec<PrinterInfo>` cannot answer the only question that matters when it comes
/// back empty: did we look and find nothing, or were we never allowed to look? Those two need
/// **opposite** things from the user — plug a printer in, versus grant the app a permission — so
/// the difference has to survive all the way to the screen. Logging it is not enough.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrinterDiscovery {
    /// The scan ran. `printers` is the honest answer, empty or not.
    Scanned { printers: Vec<PrinterInfo> },
    /// The scan never ran: the OS denied the permission it needs. `permission` names it so the
    /// screen can point at the exact toggle instead of guessing.
    #[serde(rename = "local_network_permission_denied")]
    PermissionDenied { permission: String },
}

/// The one wire string that means "the OS blocked the scan": the `status` of the outcome on the
/// invoke path and the `code` of the bridge's WS error frame. One literal for both transports, so
/// a screen only has to learn to branch once.
pub const LOCAL_NETWORK_PERMISSION_DENIED: &str = "local_network_permission_denied";

impl PrinterDiscovery {
    /// The printers found — `None` when the scan never ran, so an empty list can never be mistaken
    /// for "there are no printers here". That `Some(&[]) != None` is the whole point of the type.
    pub fn scanned_printers(&self) -> Option<&[PrinterInfo]> {
        match self {
            Self::Scanned { printers } => Some(printers),
            Self::PermissionDenied { .. } => None,
        }
    }

    /// The permission the OS refused, if that is what happened.
    pub fn denied_permission(&self) -> Option<&str> {
        match self {
            Self::Scanned { .. } => None,
            Self::PermissionDenied { permission } => Some(permission),
        }
    }
}

/// Discovers every network printer (mDNS + subnet sweep), deduplicates, enriches with the MAC and
/// registers them in `registry`.
///
/// `access` is not decoration: with the permission denied the sweep is 254 silent timeouts and an
/// mDNS window that resolves nothing, so it is skipped and the caller is told **why** there is no
/// list — instead of being handed an empty one it cannot tell from an empty venue.
pub async fn discover_printers(
    registry: &DeviceRegistry,
    access: LocalNetworkAccess,
) -> Result<PrinterDiscovery> {
    if let LocalNetworkAccess::Denied { permission } = access {
        tracing::warn!(
            %permission,
            "printer discovery skipped: the OS denies this app access to the local network"
        );
        return Ok(PrinterDiscovery::PermissionDenied { permission });
    }

    Ok(PrinterDiscovery::Scanned {
        printers: scan_network(registry).await?,
    })
}

/// How long a typed address gets to answer before it is refused as unreachable.
///
/// Longer than the sweep's [`sweep::SCAN_CONNECT_TIMEOUT`] on purpose: this is ONE connect the
/// owner is waiting on, often to another subnet through a router, and refusing a printer that was
/// merely slow sends them to check a cable that is fine.
pub const MANUAL_PRINTER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Adds a network printer by the address the owner TYPED — the way out when the scan
/// cannot see it (another subnet, an isolated guest Wi-Fi, mDNS blocked by the router).
///
/// Three answers, and only the first saves anything:
/// - it answers on `host:port` → registered as [`Device::manual`](crate::protocol::Device) and
///   returned as the same [`PrinterInfo`] the scan would have produced;
/// - not a dotted IPv4 or port 0 → [`PeripheralError::InvalidPrinterId`](crate::PeripheralError),
///   before any connect;
/// - nobody answers within `probe_timeout` → [`PeripheralError::Unreachable`](crate::PeripheralError).
///   Saving it anyway would leave a printer in the list that every ticket silently fails on.
///
/// IPv4 only: it is what a thermal printer's self-test sheet prints, and it keeps the
/// `network:{ip}:{port}` id unambiguous.
pub async fn add_network_printer(
    registry: &DeviceRegistry,
    host: &str,
    port: u16,
    probe_timeout: Duration,
) -> Result<PrinterInfo> {
    let host = host.trim();
    let ip: std::net::Ipv4Addr = host.parse().map_err(|_| {
        crate::PeripheralError::InvalidPrinterId(format!("not an IPv4 address: `{host}`"))
    })?;
    if port == 0 {
        return Err(crate::PeripheralError::InvalidPrinterId(format!(
            "port 0 in: {ip}:{port}"
        )));
    }

    let addr = SocketAddr::from((ip, port));
    match tokio::time::timeout(probe_timeout, TcpStream::connect(addr)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return Err(crate::PeripheralError::Unreachable(format!("{addr}: {e}")));
        }
        Err(_) => {
            return Err(crate::PeripheralError::Unreachable(format!(
                "{addr}: no answer within {} ms",
                probe_timeout.as_millis()
            )));
        }
    }

    let ip = ip.to_string();
    let name = format!("Network Printer ({ip})");
    registry.register_manual(&ip, port, &name)?;
    Ok(PrinterInfo {
        id: format!("network:{ip}:{port}"),
        name,
        kind: "network".into(),
        // Answering on a raw port says nothing about the language it speaks (see the sweep).
        category: crate::protocol::default_printer_category(),
        status: "ready".into(),
        paper_width: 80,
        mac: None,
    })
}

/// Adds the printers the owner typed by hand ([`add_network_printer`]) that this scan did not
/// see, so a printer on another subnet does not vanish from the list at the next «Re-scan». Their
/// state is the watchdog's (`offline` → `offline`, anything else → `ready`).
///
/// A blocked scan stays blocked: the screen has to ask for the permission first, and a typed
/// printer is behind that same permission.
pub fn with_manual_printers(outcome: PrinterDiscovery, registry: &DeviceRegistry) -> PrinterDiscovery {
    let PrinterDiscovery::Scanned { mut printers } = outcome else {
        return outcome;
    };
    for device in registry.get_all().into_iter().filter(|d| d.manual) {
        let id = format!("network:{}:{}", device.ip, device.port);
        if printers.iter().any(|p| p.id == id) {
            continue;
        }
        printers.push(PrinterInfo {
            id,
            name: device.name,
            kind: "network".into(),
            category: crate::protocol::default_printer_category(),
            status: if device.status == "offline" { "offline" } else { "ready" }.into(),
            paper_width: 80,
            mac: device.mac,
        });
    }
    PrinterDiscovery::Scanned { printers }
}

/// The scan itself: mDNS + subnet sweep, deduplicated, MAC-enriched and registered.
async fn scan_network(registry: &DeviceRegistry) -> Result<Vec<PrinterInfo>> {
    // mDNS wins when both find the same printer: it goes in first and the sweep only adds new ids.
    let mut printers = discover_mdns().await?;

    let mut seen_ids: HashSet<String> = printers.iter().map(|p| p.id.clone()).collect();
    for p in discover_subnet_scan(ESCPOS_NETWORK_PORT).await? {
        if seen_ids.insert(p.id.clone()) {
            printers.push(p);
        }
    }

    // Enrich every network printer with its MAC (ARP) and register it. A failure to resolve or to
    // register is ignored: it must not break the discovery.
    for p in printers.iter_mut() {
        if p.kind != "network" {
            continue;
        }
        let (ip, port) = match parse_printer_id(&p.id) {
            Ok(t) => (t.host, t.port),
            Err(_) => continue,
        };

        // The MAC is an ENRICHMENT, not a requirement: the printer is always registered. On
        // Android ARP never resolves (there is no `arp` binary and `/proc/net/arp` is restricted
        // since Android 10), so registering only with a MAC would leave no printer in
        // `devices.json`: `get_devices` would return `[]` and no kitchen/bar/till role could be
        // assigned.
        p.mac = crate::registry::get_mac_for_ip(&ip);
        let _ = registry.register(p.mac.as_deref(), &ip, port, &p.name, "network");
    }

    Ok(printers)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── "No permission" is not "no printers" ──────────────────────────────────────────────────
    //
    // Without local network access the sweep is 254 silent timeouts and the mDNS window resolves
    // nothing, so discovery would come back with an empty list and no error whatsoever — the
    // worst failure mode a till can have. The two states send the user in OPPOSITE directions
    // ("connect a printer" vs "grant this app permission"), so they must stay apart in the return
    // value.

    const ANDROID_LOCAL_NETWORK: &str = "android.permission.ACCESS_LOCAL_NETWORK";

    fn denied() -> LocalNetworkAccess {
        LocalNetworkAccess::Denied {
            permission: ANDROID_LOCAL_NETWORK.to_string(),
        }
    }

    /// Registry on a temp path: a test must never touch the machine's real `devices.json`.
    fn temp_registry(name: &str) -> DeviceRegistry {
        let dir = std::env::temp_dir().join(format!(
            "erplora-discovery-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        DeviceRegistry::load(dir.join(format!("{name}.json")))
    }

    #[test]
    fn granted_access_blocks_nothing() {
        assert_eq!(LocalNetworkAccess::Granted.blocked_by(), None);
    }

    #[test]
    fn denied_access_names_the_permission_the_user_has_to_grant() {
        // The name is what lets the screen point at one toggle instead of listing every setting.
        assert_eq!(denied().blocked_by(), Some(ANDROID_LOCAL_NETWORK));
    }

    #[test]
    fn an_empty_scan_is_an_answer_but_a_blocked_scan_is_not() {
        // `Some(&[])` vs `None` — the ONLY difference between "we looked, nothing there" and "we
        // were never allowed to look". Collapsing both into an empty Vec is the bug.
        assert_eq!(
            PrinterDiscovery::Scanned {
                printers: Vec::new()
            }
            .scanned_printers(),
            Some(&[][..])
        );
        assert_eq!(
            PrinterDiscovery::PermissionDenied {
                permission: ANDROID_LOCAL_NETWORK.to_string(),
            }
            .scanned_printers(),
            None
        );
    }

    #[test]
    fn only_a_blocked_run_names_a_permission() {
        assert_eq!(
            PrinterDiscovery::Scanned {
                printers: Vec::new()
            }
            .denied_permission(),
            None
        );
        assert_eq!(
            PrinterDiscovery::PermissionDenied {
                permission: ANDROID_LOCAL_NETWORK.to_string(),
            }
            .denied_permission(),
            Some(ANDROID_LOCAL_NETWORK)
        );
    }

    #[test]
    fn the_wire_carries_the_status_so_the_screen_can_branch_on_it() {
        // The shell command and the module SDK read `status`. If the difference only existed in a
        // log line, the user would still be looking at the same two identical screens.
        let blocked = serde_json::to_value(PrinterDiscovery::PermissionDenied {
            permission: ANDROID_LOCAL_NETWORK.to_string(),
        })
        .expect("serializable");
        assert_eq!(blocked["status"], LOCAL_NETWORK_PERMISSION_DENIED);
        assert_eq!(blocked["permission"], ANDROID_LOCAL_NETWORK);

        let empty = serde_json::to_value(PrinterDiscovery::Scanned {
            printers: Vec::new(),
        })
        .expect("serializable");
        assert_eq!(empty["status"], "scanned");
        assert_eq!(empty["printers"], serde_json::json!([]));
        assert!(
            empty.get("permission").is_none(),
            "an honest zero must not name a permission: there is nothing to grant"
        );
    }

    #[tokio::test]
    async fn a_denied_permission_skips_the_scan_instead_of_reporting_zero_printers() {
        // Not merely a different return value: with the permission denied the sweep can ONLY come
        // back empty, so running it burns the 1.5 s mDNS window to learn nothing. Answering well
        // inside that window is the proof that we never looked.
        let registry = temp_registry("denied");

        let outcome = tokio::time::timeout(
            Duration::from_millis(200),
            discover_printers(&registry, denied()),
        )
        .await
        .expect("a blocked scan must answer at once, not after the mDNS window")
        .expect("a denied permission is a state, not a transport failure");

        assert_eq!(
            outcome,
            PrinterDiscovery::PermissionDenied {
                permission: ANDROID_LOCAL_NETWORK.to_string(),
            }
        );
        assert_eq!(
            outcome.scanned_printers(),
            None,
            "an empty list here would be the very lie this type exists to prevent"
        );
        assert!(
            registry.get_all().is_empty(),
            "a scan that never ran must not register a single device"
        );
    }

    // ── A printer the scan cannot see, added by typing its address ───────────────────────────────
    //
    // The sweep only covers the device's own /24 and mDNS stops at the router; a printer on another
    // subnet or an isolated guest Wi-Fi never shows up. Every POS lets the owner type the IP from
    // the printer's self-test sheet. These pin the three answers that door can give.

    #[tokio::test]
    async fn a_typed_address_that_answers_is_registered_as_a_manual_network_printer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let registry = temp_registry("manual-ok");

        let printer = add_network_printer(&registry, "127.0.0.1", port, Duration::from_secs(2))
            .await
            .expect("a printer that answers must be added");

        let id = format!("network:127.0.0.1:{port}");
        assert_eq!(printer.id, id);
        assert_eq!(printer.kind, "network");
        assert_eq!(printer.status, "ready");
        let device = registry.get(&id).expect("the typed printer must be in the registry");
        assert!(device.manual, "the registry must remember it was typed, or the next scan loses it");
        assert_eq!(device.kind, "network", "the watchdog only watches `network` devices");
    }

    #[tokio::test]
    async fn a_typed_address_nobody_answers_is_refused_and_not_registered() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            l.local_addr().expect("addr").port()
        }; // dropped: nothing listens there any more
        let registry = temp_registry("manual-unreachable");

        let err = add_network_printer(&registry, "127.0.0.1", port, Duration::from_secs(2))
            .await
            .expect_err("nobody answers: it must not be added as if it worked");

        assert_eq!(err.code(), "printer_unreachable");
        assert!(registry.get_all().is_empty(), "a printer that did not answer must not be saved");
    }

    #[tokio::test]
    async fn a_typed_address_that_is_not_an_ipv4_is_refused_before_connecting() {
        let registry = temp_registry("manual-invalid");
        for (host, port) in [
            ("192.168.1", 9100),
            ("192.168.1.300", 9100),
            ("printer.local", 9100),
            ("", 9100),
            ("  ", 9100),
            ("192.168.1.20", 0),
        ] {
            let err = add_network_printer(&registry, host, port, Duration::from_secs(2))
                .await
                .expect_err("not a usable address");
            assert_eq!(err.code(), "invalid_printer_address", "{host}:{port}");
        }
        assert!(registry.get_all().is_empty());
    }

    #[tokio::test]
    async fn a_typed_address_is_trimmed_before_it_becomes_the_printer_id() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let registry = temp_registry("manual-trim");

        let printer = add_network_printer(&registry, " 127.0.0.1 ", port, Duration::from_secs(2))
            .await
            .expect("added");

        assert_eq!(printer.id, format!("network:127.0.0.1:{port}"));
    }

    fn scanned(printers: Vec<PrinterInfo>) -> PrinterDiscovery {
        PrinterDiscovery::Scanned { printers }
    }

    #[test]
    fn a_manual_printer_the_scan_does_not_see_stays_in_the_list() {
        let registry = temp_registry("manual-merge");
        registry.register_manual("10.9.9.9", 9100, "Network Printer (10.9.9.9)").expect("manual");
        // A printer found once by the scan and gone now is NOT kept: only what the owner typed.
        registry
            .register(None, "10.1.1.1", 9100, "Network Printer (10.1.1.1)", "network")
            .expect("scanned");

        let merged = with_manual_printers(scanned(vec![]), &registry);

        let ids: Vec<&str> = merged.scanned_printers().expect("scanned").iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["network:10.9.9.9:9100"]);
    }

    #[test]
    fn a_manual_printer_the_scan_also_found_is_listed_once() {
        let registry = temp_registry("manual-dedupe");
        registry.register_manual("10.9.9.9", 9100, "Network Printer (10.9.9.9)").expect("manual");
        let found = PrinterInfo {
            id: "network:10.9.9.9:9100".into(),
            name: "Network Printer (10.9.9.9)".into(),
            kind: "network".into(),
            category: crate::protocol::default_printer_category(),
            status: "ready".into(),
            paper_width: 80,
            mac: None,
        };

        let merged = with_manual_printers(scanned(vec![found]), &registry);

        assert_eq!(merged.scanned_printers().expect("scanned").len(), 1);
    }

    #[test]
    fn a_manual_printer_the_watchdog_saw_go_down_is_listed_offline() {
        let registry = temp_registry("manual-offline");
        registry.register_manual("10.9.9.9", 9100, "Network Printer (10.9.9.9)").expect("manual");
        registry.set_status("network:10.9.9.9:9100", "offline").expect("status");

        let merged = with_manual_printers(scanned(vec![]), &registry);

        assert_eq!(merged.scanned_printers().expect("scanned")[0].status, "offline");
    }

    #[test]
    fn a_blocked_scan_stays_blocked_even_with_manual_printers() {
        let registry = temp_registry("manual-denied");
        registry.register_manual("10.9.9.9", 9100, "Network Printer (10.9.9.9)").expect("manual");

        let merged = with_manual_printers(
            PrinterDiscovery::PermissionDenied { permission: ANDROID_LOCAL_NETWORK.into() },
            &registry,
        );

        assert_eq!(merged.denied_permission(), Some(ANDROID_LOCAL_NETWORK));
    }
}

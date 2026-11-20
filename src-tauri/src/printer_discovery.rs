//! Which printers this device can print on: the network sweep, the bonded Bluetooth printers and
//! the machine's own USB print queues, as one list.

use erplora_peripherals::discovery::{self, LocalNetworkAccess, PrinterDiscovery};
use erplora_peripherals::registry::DeviceRegistry;

use crate::hardware::{AddPrinterError, HardwareError, PeripheralsState};

/// Reads the Android runtime-permission map and says whether the printer sweep may run.
///
/// Pure on purpose: an emulator is the only other way to reach this branch, and a state that
/// decides between two opposite messages to the user has to be provable on a laptop.
///
/// A permission the map does not mention is one this platform does not gate — desktop answers with
/// an empty map, and so does an Android older than the release that introduced it. Reading that
/// silence as "denied" would ground a till that can see its printer perfectly well.
fn local_network_access(status: &std::collections::HashMap<String, bool>) -> LocalNetworkAccess {
    match status.get(tauri_plugin_erplora_android::ACCESS_LOCAL_NETWORK) {
        Some(false) => LocalNetworkAccess::Denied {
            permission: tauri_plugin_erplora_android::ACCESS_LOCAL_NETWORK.to_string(),
        },
        _ => LocalNetworkAccess::Granted,
    }
}

/// What the OS allows right now — asked without prompting: the PWA already requested the
/// permission before invoking this, so here we only read the answer it gave.
fn shell_local_network_access<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> LocalNetworkAccess {
    use tauri_plugin_erplora_android::ErploraAndroidExt;

    match app.erplora_android().check_permissions() {
        Ok(status) => local_network_access(&status),
        // The CHECK failing is not a denial — on desktop there is no Android activity to ask. If
        // we treated it as one, the shell would start claiming "no permission" on the platform
        // that has no such permission, which is the same lie pointing the other way.
        Err(e) => {
            log::warn!("shell: could not read the local network permission ({e}); scanning anyway");
            LocalNetworkAccess::Granted
        }
    }
}

/// Converts a bonded printer (as Kotlin announces it) into the shared `PrinterInfo` shape.
///
/// `category` stays `unknown` on purpose — same honesty rule as the port-9100 sweep: SPP says
/// "accepts bytes", not "speaks ESC/POS", and guessing "thermal" is how wrong paper gets printed.
fn bluetooth_printer_info(
    p: &tauri_plugin_erplora_android::BluetoothPrinter,
) -> erplora_peripherals::protocol::PrinterInfo {
    erplora_peripherals::protocol::PrinterInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        kind: "bluetooth".into(),
        category: erplora_peripherals::protocol::default_printer_category(),
        status: "ready".into(),
        paper_width: 80,
        mac: Some(p.mac.clone()),
    }
}

/// Folds the bonded Bluetooth printers into the network discovery outcome.
///
/// One list on purpose: "which printers can this device print on?" is ONE question to the person
/// setting up a till, whatever transport each answer arrives by. Deduplicated by id, so a
/// re-merge cannot turn one printer into two devices.
///
/// A `PermissionDenied` outcome stays a refusal WITHOUT printers: an answer that both names a
/// missing permission and lists printers would give the screen two contradictory instructions at
/// once. Discovery asks for both printer permissions up front, so that state is one
/// "allow" away from resolving itself.
fn merge_bluetooth_printers(
    outcome: PrinterDiscovery,
    bonded: Vec<tauri_plugin_erplora_android::BluetoothPrinter>,
) -> PrinterDiscovery {
    match outcome {
        PrinterDiscovery::Scanned { mut printers } => {
            for p in &bonded {
                if printers.iter().any(|known| known.id == p.id) {
                    continue;
                }
                printers.push(bluetooth_printer_info(p));
            }
            PrinterDiscovery::Scanned { printers }
        }
        blocked @ PrinterDiscovery::PermissionDenied { .. } => blocked,
    }
}

/// Folds the machine's USB print queues into the discovery outcome.
///
/// Same rule as the Bluetooth merge, and for the same reason: "which printers can this device
/// print on?" is ONE question to the person setting up a till, whatever cable each answer arrives
/// by. Deduplicated by id, so a re-scan cannot turn one printer into two.
///
/// A `PermissionDenied` outcome stays a refusal without printers. On desktop — the only place a
/// USB queue exists — that branch is unreachable (there is no local-network gate to deny), but the
/// shell having ONE merge rule beats it having two that differ in a case nobody can hit.
fn merge_usb_printers(
    outcome: PrinterDiscovery,
    usb: Vec<erplora_peripherals::protocol::PrinterInfo>,
) -> PrinterDiscovery {
    match outcome {
        PrinterDiscovery::Scanned { mut printers } => {
            for queue in usb {
                if printers.iter().any(|known| known.id == queue.id) {
                    continue;
                }
                printers.push(queue);
            }
            PrinterDiscovery::Scanned { printers }
        }
        blocked @ PrinterDiscovery::PermissionDenied { .. } => blocked,
    }
}

/// Puts the machine's own print queues into the device registry, so the owner can say which one
/// prints the kitchen's tickets.
///
/// The network and Bluetooth halves of discovery already register what they find; this is the
/// third. A queue has neither MAC nor socket, so it enters by its `printer_id` — the door
/// `DeviceRegistry::register_queue` exists for.
///
/// A refusal is **logged and skipped**, never propagated: a discovery is a batch, and one queue
/// with an id CUPS could not have produced must not cost the till the printer it does have. Logged
/// because a queue that silently never accepts a role is exactly the mute failure that sends a
/// user to press a button that does nothing.
fn register_discovered_queues(
    registry: &DeviceRegistry,
    queues: &[erplora_peripherals::protocol::PrinterInfo],
) {
    for queue in queues {
        if let Err(e) = registry.register_queue(&queue.id, &queue.name) {
            log::warn!(
                "shell: the print queue `{}` did not enter the device registry ({e}); it prints, \
                 but it will not accept a role",
                queue.id
            );
        }
    }
}

/// `erplora_discover_printers` — rescans the network (mDNS + subnet), lists the PAIRED Bluetooth
/// printers (Android only), registers and returns the printers. Mirrors the bridge's
/// `Command::DiscoverPrinters`.
///
/// Returns the OUTCOME (`{status, …}`), not a bare array: without the local network permission
/// there is no empty list to return, there is a reason — and «connect a printer» and «give the app
/// permission» are opposite instructions for the user.
#[tauri::command]
pub(crate) async fn erplora_discover_printers(
    app: tauri::AppHandle,
    state: tauri::State<'_, PeripheralsState>,
) -> Result<PrinterDiscovery, HardwareError> {
    use tauri_plugin_erplora_android::ErploraAndroidExt;

    let access = shell_local_network_access(&app);
    let outcome = discovery::discover_printers(&state.registry, access).await?;

    // Bonded SPP printers (Android; empty everywhere else). A bluetooth failure must not poison
    // the network half — a venue with a broken adapter still has its LAN printers.
    let bonded = app
        .erplora_android()
        .bluetooth_bonded_printers()
        .unwrap_or_else(|e| {
            log::warn!("shell: could not list bonded bluetooth printers ({e})");
            Vec::new()
        });
    // Registered like their network siblings, so roles (kitchen/bar/receipt) can be assigned to
    // them; the MAC is the key, ip/port stay empty — the watchdog knows to leave them alone.
    for p in &bonded {
        let _ = state.registry.register(Some(&p.mac), "", 0, &p.name, "bluetooth");
    }

    // The machine's own USB print queues (desktop only — Android has no queue to ask).
    // `spawn_blocking` because asking CUPS spawns a client and waits for it: on a wedged `cupsd`
    // that wait runs to its timeout, and doing it on the async runtime would stall every other
    // command for as long as it lasted.
    //
    // A CUPS failure must not poison the network half: a venue whose LAN printers work fine has to
    // keep seeing them when the CUPS client tools are missing. Logged, so it is not a silent zero.
    #[cfg(not(target_os = "android"))]
    let usb = tokio::task::spawn_blocking(|| {
        erplora_peripherals::usb::discovery::discover_usb_printers(&erplora_peripherals::usb::cups::SystemCups)
    })
    .await
    .unwrap_or_else(|e| {
        Err(erplora_peripherals::PeripheralError::Unreachable(format!(
            "the USB queue listing did not finish: {e}"
        )))
    })
    .unwrap_or_else(|e| {
        log::warn!("shell: could not list the OS print queues ({e}); USB printers will not appear");
        Vec::new()
    });
    #[cfg(target_os = "android")]
    let usb = Vec::new();

    // Registered like their network and bluetooth siblings, so roles (kitchen/bar/receipt) can be
    // assigned to them; the printer_id is the key, ip/port stay empty — the watchdog only monitors
    // `network`, so a queue never enters the health probe or the ARP recovery sweep.
    register_discovered_queues(&state.registry, &usb);

    // Printers the owner typed by address stay listed even when the sweep cannot see
    // them — that is why they were typed.
    let outcome = discovery::with_manual_printers(outcome, &state.registry);

    Ok(merge_usb_printers(
        merge_bluetooth_printers(outcome, bonded),
        usb,
    ))
}

/// `erplora_add_network_printer` — adds a network printer by the address the owner TYPED: the way
/// in when the scan cannot see it (another subnet, an isolated Wi-Fi, mDNS blocked by the router).
/// It connects first and only a printer that answers is saved; the next discovery keeps listing
/// it. Returns the printer as the scan would have.
#[tauri::command]
pub(crate) async fn erplora_add_network_printer(
    state: tauri::State<'_, PeripheralsState>,
    host: String,
    port: u16,
) -> Result<erplora_peripherals::protocol::PrinterInfo, AddPrinterError> {
    Ok(discovery::add_network_printer(
        &state.registry,
        &host,
        port,
        discovery::MANUAL_PRINTER_PROBE_TIMEOUT,
    )
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::tempdir;

    // ── Bonded Bluetooth printers join discovery ─────────────────────────────────────────────
    //
    // The LAN sweep and the bonded list are two halves of ONE question — "which printers can
    // this device print on?" — so they come back as one list. The merge is pure so the emulator
    // (which has no Bluetooth at all) is not the only place it can be exercised.

    fn a_bonded_printer() -> tauri_plugin_erplora_android::BluetoothPrinter {
        serde_json::from_value(serde_json::json!({
            "id": "bluetooth:AA:BB:CC:DD:EE:FF",
            "name": "Kitchen BT",
            "mac": "AA:BB:CC:DD:EE:FF"
        }))
        .expect("the wire shape Kotlin builds")
    }

    #[test]
    fn a_bonded_bluetooth_printer_joins_the_scanned_list() {
        let merged = merge_bluetooth_printers(
            PrinterDiscovery::Scanned { printers: vec![] },
            vec![a_bonded_printer()],
        );
        let printers = merged.scanned_printers().expect("still a scanned outcome");
        assert_eq!(printers.len(), 1);
        assert_eq!(printers[0].id, "bluetooth:AA:BB:CC:DD:EE:FF");
        assert_eq!(printers[0].kind, "bluetooth");
        assert_eq!(printers[0].name, "Kitchen BT");
        assert_eq!(printers[0].mac.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    }

    #[test]
    fn a_blocked_lan_scan_stays_blocked_and_carries_no_printers() {
        // A limit, stated on purpose: with the LAN permission denied the outcome is the
        // permission refusal, and the bonded list does NOT ride along — an outcome that both
        // names a missing permission and lists printers would give the screen two contradictory
        // instructions at once. The permission prompt for discovery asks for both, so
        // this state is one "allow" away from resolving itself.
        let merged = merge_bluetooth_printers(
            PrinterDiscovery::PermissionDenied {
                permission: "android.permission.ACCESS_LOCAL_NETWORK".into(),
            },
            vec![a_bonded_printer()],
        );
        assert_eq!(merged.scanned_printers(), None);
    }

    #[test]
    fn a_bonded_printer_already_listed_is_not_duplicated() {
        let once = merge_bluetooth_printers(
            PrinterDiscovery::Scanned { printers: vec![] },
            vec![a_bonded_printer()],
        );
        let twice = merge_bluetooth_printers(once, vec![a_bonded_printer()]);
        assert_eq!(
            twice.scanned_printers().expect("scanned").len(),
            1,
            "the same bonded printer merged twice must stay one device"
        );
    }

    // ── The machine's USB print queues join discovery ────────────────────────────────────────
    //
    // The USB thermal printer is the cheapest in the catalogue and the one a single-till bar or
    // salon actually buys. It reaches us through the OS print queue, so the merge is pure and
    // testable on any laptop — including the CI machines that have no printer plugged in.

    fn a_usb_printer() -> erplora_peripherals::protocol::PrinterInfo {
        erplora_peripherals::protocol::PrinterInfo {
            id: "usb:Star_TSP143".into(),
            name: "Star TSP143".into(),
            kind: "usb".into(),
            category: erplora_peripherals::protocol::default_printer_category(),
            status: "ready".into(),
            paper_width: 80,
            mac: None,
        }
    }

    #[test]
    fn a_usb_queue_joins_the_scanned_list() {
        let merged =
            merge_usb_printers(PrinterDiscovery::Scanned { printers: vec![] }, vec![a_usb_printer()]);
        let printers = merged.scanned_printers().expect("still a scanned outcome");
        assert_eq!(printers.len(), 1);
        assert_eq!(printers[0].id, "usb:Star_TSP143");
        assert_eq!(printers[0].kind, "usb");
    }

    #[test]
    fn a_usb_queue_already_listed_is_not_duplicated() {
        let once =
            merge_usb_printers(PrinterDiscovery::Scanned { printers: vec![] }, vec![a_usb_printer()]);
        let twice = merge_usb_printers(once, vec![a_usb_printer()]);
        assert_eq!(
            twice.scanned_printers().expect("scanned").len(),
            1,
            "the same queue merged twice must stay one device"
        );
    }

    #[test]
    fn a_blocked_scan_stays_blocked_and_carries_no_usb_queues() {
        let merged = merge_usb_printers(
            PrinterDiscovery::PermissionDenied {
                permission: "android.permission.ACCESS_LOCAL_NETWORK".into(),
            },
            vec![a_usb_printer()],
        );
        assert_eq!(merged.scanned_printers(), None);
    }

    // ── A discovered queue is a DEVICE, so the owner can say it is the kitchen's ─────────────
    //
    // Listing the queue is not enough: without an entry in the registry it can be printed to but
    // never named, so a venue with a USB printer at the counter and a network one in the kitchen
    // can only tell the hub about one of the two.

    #[test]
    fn a_discovered_usb_queue_enters_the_device_registry() {
        let registry = DeviceRegistry::load(tempdir().join("devices.json"));
        let second = erplora_peripherals::protocol::PrinterInfo {
            id: "usb:EPSON_TM".into(),
            name: "EPSON TM-T20III".into(),
            ..a_usb_printer()
        };

        register_discovered_queues(&registry, &[a_usb_printer(), second]);

        let mut keys: Vec<String> = registry.get_all().into_iter().map(|d| d.key).collect();
        keys.sort();
        assert_eq!(
            keys,
            ["usb:EPSON_TM", "usb:Star_TSP143"],
            "each queue keeps an identity of its own"
        );
        registry
            .set_role("usb:Star_TSP143", "kitchen")
            .expect("and can be told which paper it prints");
    }

    #[test]
    fn a_queue_that_cannot_be_registered_does_not_cost_the_others_theirs() {
        // A discovery is a batch: one id the registry refuses must not take the working printer
        // down with it, or a single odd queue would leave the till with no roles at all.
        let registry = DeviceRegistry::load(tempdir().join("devices.json"));
        let broken = erplora_peripherals::protocol::PrinterInfo {
            id: "usb:".into(),
            name: "nameless".into(),
            ..a_usb_printer()
        };

        register_discovered_queues(&registry, &[broken, a_usb_printer()]);

        let keys: Vec<String> = registry.get_all().into_iter().map(|d| d.key).collect();
        assert_eq!(keys, ["usb:Star_TSP143"], "the good one is registered anyway");
    }

    // ── "The OS won't let me look" is not "there are no printers" ────────────────────────────
    //
    // The shell is where the two states are told apart, because it is the only layer that can ask
    // Android what it granted. Get this wrong in either direction and the till hands the user the
    // opposite instruction to the one that would fix it.

    use erplora_peripherals::discovery::LocalNetworkAccess;
    use std::collections::HashMap;
    use tauri_plugin_erplora_android::{ACCESS_LOCAL_NETWORK, POST_NOTIFICATIONS};

    fn permission_status(pairs: &[(&str, bool)]) -> HashMap<String, bool> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn a_platform_that_never_answers_is_not_a_platform_that_said_no() {
        // Desktop returns an empty map — there is no such permission on Windows/macOS/Linux, and
        // Android below API 37 does not know it either. Reading that silence as a denial would
        // ground a till whose printer is right there.
        assert_eq!(
            local_network_access(&HashMap::new()),
            LocalNetworkAccess::Granted
        );
    }

    #[test]
    fn a_granted_permission_lets_the_scan_run() {
        assert_eq!(
            local_network_access(&permission_status(&[(ACCESS_LOCAL_NETWORK, true)])),
            LocalNetworkAccess::Granted
        );
    }

    #[test]
    fn a_denied_permission_names_itself_instead_of_faking_an_empty_venue() {
        // The name is what turns a dead end into an instruction: the screen can say which toggle
        // to flip rather than inviting the user to go hunting for a printer that is already on.
        assert_eq!(
            local_network_access(&permission_status(&[(ACCESS_LOCAL_NETWORK, false)])),
            LocalNetworkAccess::Denied {
                permission: ACCESS_LOCAL_NETWORK.to_string(),
            }
        );
    }

    #[test]
    fn saying_no_to_notifications_does_not_ground_printer_discovery() {
        // Two independent permissions asked in the same dialog run. Refusing the notifications one
        // must not make the till claim it cannot reach a network it can reach.
        assert_eq!(
            local_network_access(&permission_status(&[
                (POST_NOTIFICATIONS, false),
                (ACCESS_LOCAL_NETWORK, true),
            ])),
            LocalNetworkAccess::Granted
        );
        assert_eq!(
            local_network_access(&permission_status(&[(POST_NOTIFICATIONS, false)])),
            LocalNetworkAccess::Granted
        );
    }

    #[test]
    fn denying_both_still_blames_the_permission_that_actually_blocks_the_printers() {
        assert_eq!(
            local_network_access(&permission_status(&[
                (POST_NOTIFICATIONS, false),
                (ACCESS_LOCAL_NETWORK, false),
            ])),
            LocalNetworkAccess::Denied {
                permission: ACCESS_LOCAL_NETWORK.to_string(),
            }
        );
    }
}

//! Network printer discovery, and parsing of `printer_id` (**all three transports**).
//!
//! Only the **network** discovery lives here: the OS's USB print queues are listed by
//! [`crate::usb`], and the bonded Bluetooth printers by the Kotlin plugin.
//!
//! Methods: a /24 subnet sweep on port 9100 plus mDNS (`_pdl-datastream._tcp`, `_ipp._tcp`),
//! deduplicated and enriched with the MAC (ARP) so they can be registered in
//! `registry::DeviceRegistry`.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

use crate::protocol::PrinterInfo;
use crate::registry::DeviceRegistry;
use crate::{Result, ESCPOS_NETWORK_PORT};

/// TCP `connect` timeout during the subnet sweep.
///
/// A `connect` to a powered printer on the same LAN takes milliseconds, so a 300 ms timeout looks
/// generous — and it is not: the margin is not eaten by the printer but by CONTENTION. With the sweep
/// competing with itself (the `registry` watchdog sweeps this same subnet) or with a loaded
/// machine, the `connect` to the REAL printer goes past 300 ms and is dropped **without a single
/// error**: «no printers».
///
/// Measured against a powered thermal printer with two sweeps in parallel: at 300 ms it was lost
/// 3 times out of 3; capping the concurrency at [`SCAN_MAX_IN_FLIGHT`] brought it down to 1 of 3;
/// at 1 s, 0 of 3. Both halves are needed.
///
/// The cost is the worst case of a sweep with nobody on the other end: 254 addresses in batches of
/// 64 → 4 batches × 1 s ≈ 4 s. Discovery is asked for by hand and seldom; losing the printer costs
/// far more than waiting four seconds.
const SCAN_CONNECT_TIMEOUT: Duration = Duration::from_millis(1000);
const MDNS_BROWSE_WINDOW: Duration = Duration::from_millis(1500);
const MDNS_SERVICE_TYPES: [&str; 2] = ["_pdl-datastream._tcp.local.", "_ipp._tcp.local."];

/// Network print destination: a TCP host and port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkTarget {
    pub host: String,
    pub port: u16,
}

impl NetworkTarget {
    /// `host:port`, as `tokio::net::TcpStream::connect` takes it.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Bluetooth Classic SPP print destination — Android only.
///
/// The MAC is the whole identity: RFCOMM connects to a bonded device by address, there is no
/// host/port. Desktop and iOS never produce nor accept this variant — iOS because of Apple's MFi
/// restriction on Bluetooth Classic, desktop because the cheap printer there arrives by cable and
/// is served by the OS print queue instead ([`UsbTarget`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothTarget {
    /// Normalized (`AA:BB:CC:DD:EE:FF`): the same spelling the device registry keys on and the
    /// Kotlin transport connects with — two spellings of one printer would be two devices.
    pub mac: String,
}

/// Raw print destination through the operating system's own print queue — desktop only.
///
/// The queue name is the whole identity. There is no host, no port and no MAC: the cable is the
/// OS's business, and what we address is the spooler entry the user (or the vendor installer)
/// created for the printer. That is deliberate — it is what lets us support USB *without shipping
/// a driver per OS*. The vendor writes the driver; we keep emitting the same ESC/POS bytes we emit
/// over TCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbTarget {
    /// A CUPS destination name, already validated: non-empty, no SPACE/TAB/`/`/`#`, no leading
    /// `-`, at most [`CUPS_MAX_QUEUE_NAME`] bytes. See [`parse_print_target`].
    pub queue: String,
}

/// Where a print job goes, parsed from a `printer_id`.
///
/// `network:{ip}:{port}` feeds the TCP path everywhere; `bluetooth:{mac}` exists ONLY on Android,
/// where the shell hands the rendered ESC/POS bytes to the Kotlin SPP transport. The rendering
/// stays in Rust either way — the transport is the only thing that changes.
#[derive(Debug, Clone, PartialEq)]
pub enum PrintTarget {
    Network(NetworkTarget),
    Bluetooth(BluetoothTarget),
    Usb(UsbTarget),
}

/// The longest destination name `cupsd` will accept (`printer-name` is capped at 127 bytes).
pub const CUPS_MAX_QUEUE_NAME: usize = 127;

/// Whether `name` could be a CUPS destination at all.
///
/// CUPS forbids SPACE, TAB, `/` and `#` in a destination name, so anything carrying one names a
/// queue that *cannot exist* — better said here than as an `lp` exit code nobody can trace back to
/// the printer the cashier picked. The leading `-` is ours: `lp -d <queue>` would read it as a
/// flag, and a job that quietly prints somewhere else is worse than one that fails.
pub(crate) fn is_cups_queue_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= CUPS_MAX_QUEUE_NAME
        && !name.starts_with('-')
        && !name
            .chars()
            .any(|c| c == ' ' || c == '\t' || c == '/' || c == '#' || c.is_control())
}

/// Parses a `printer_id` into the transport it names: `network:{ip}:{port}`, `bluetooth:{mac}`
/// or `usb:{queue}`. Anything else — an unknown scheme, a malformed MAC, a queue name the OS could
/// never have — is refused HERE, at the contract's edge, instead of travelling on to blow up inside
/// a socket connect or an `lp` exit code with a message nobody can map back to the id.
pub fn parse_print_target(printer_id: &str) -> Result<PrintTarget> {
    match printer_id.split_once(':') {
        Some(("bluetooth", mac)) => {
            if !crate::registry::is_mac(mac) {
                return Err(crate::PeripheralError::InvalidPrinterId(format!(
                    "malformed bluetooth MAC in: {printer_id}"
                )));
            }
            Ok(PrintTarget::Bluetooth(BluetoothTarget {
                mac: crate::registry::normalize_mac(mac),
            }))
        }
        Some(("usb", queue)) => {
            if !is_cups_queue_name(queue) {
                return Err(crate::PeripheralError::InvalidPrinterId(format!(
                    "not a usable OS print queue name in: {printer_id}"
                )));
            }
            Ok(PrintTarget::Usb(UsbTarget {
                queue: queue.to_string(),
            }))
        }
        _ => parse_printer_id(printer_id).map(PrintTarget::Network),
    }
}

/// Parses a `network:{ip}:{port}` `printer_id` into a [`NetworkTarget`].
/// It is the parser of the TCP path (print queue, drawer over a socket, registry enrichment), so it
/// refuses `bluetooth:` and `usb:` — those variants reach hardware only through
/// [`parse_print_target`] (Android's SPP transport and the OS's raw print queue).
pub fn parse_printer_id(printer_id: &str) -> Result<NetworkTarget> {
    // The scheme is split off at the FIRST ':'.
    let (scheme, rest) = match printer_id.split_once(':') {
        Some((s, r)) => (s, r),
        None => (printer_id, ""),
    };

    if scheme != "network" {
        return Err(crate::PeripheralError::InvalidPrinterId(format!(
            "not a network printer id (this is the TCP path only): {scheme}"
        )));
    }

    // Split at the LAST ':': the host may contain ':' (IPv6), the port is always last.
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => {
            let port = p.parse::<u16>().map_err(|_| {
                crate::PeripheralError::InvalidPrinterId(format!("invalid port in: {printer_id}"))
            })?;
            (h.to_string(), port)
        }
        None => (rest.to_string(), ESCPOS_NETWORK_PORT),
    };

    if host.is_empty() {
        return Err(crate::PeripheralError::InvalidPrinterId(format!(
            "empty host in: {printer_id}"
        )));
    }

    Ok(NetworkTarget { host, port })
}

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
/// Longer than the sweep's [`SCAN_CONNECT_TIMEOUT`] on purpose: this is ONE connect the owner is
/// waiting on, often to another subnet through a router, and refusing a printer that was merely
/// slow sends them to check a cable that is fine.
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

/// Subnet sweep: a TCP `connect` to `port` on `{prefix}.1..=254` of every network the device is on
/// ([`local_subnet_prefixes`]).
pub async fn discover_subnet_scan(port: u16) -> Result<Vec<PrinterInfo>> {
    let prefixes = local_subnet_prefixes();
    if prefixes.is_empty() {
        tracing::warn!("no IPv4 network to sweep for printers on :{port}");
        return Ok(Vec::new());
    }

    tracing::info!("sweeping {prefixes:?} (/24) for printers on :{port}");

    let ips = sweep_targets(&prefixes);
    let found = sweep_addresses(ips, move |ip| async move {
        let addr = format!("{ip}:{port}");
        matches!(
            tokio::time::timeout(SCAN_CONNECT_TIMEOUT, TcpStream::connect(&addr)).await,
            Ok(Ok(_))
        )
    })
    .await;

    let mut printers = Vec::new();
    {
        for ip in found {
            tracing::debug!("network printer found at {ip}:{port}");
            printers.push(PrinterInfo {
                id: format!("network:{ip}:{port}"),
                name: format!("Network Printer ({ip})"),
                kind: "network".into(),
                // Answering on 9100 says nothing about the language it speaks: an office A4 laser
                // listens there too. Without more evidence it stays unclassified.
                category: crate::protocol::default_printer_category(),
                status: "ready".into(),
                paper_width: 80,
                mac: None,
            });
        }
    }

    Ok(printers)
}

/// How many probes may be IN FLIGHT at once during the sweep.
///
/// Launching all 254 at once loses real printers: each probe has [`SCAN_CONNECT_TIMEOUT`] to
/// complete its `connect`, so a loaded machine — or a second sweep at the same time, which does
/// happen: the `registry` watchdog sweeps this same subnet — is enough for the `connect` to the
/// REAL printer to run out of time and be dropped.
///
/// And the failure is SILENT: the result is «no printers», exactly what a venue without a printer
/// looks like. Measured against a powered thermal printer: with two simultaneous sweeps it was lost
/// 3 times out of 3; one after the other, 0 of 2.
///
/// 64 keeps the sweep fast (4 batches) while giving every `connect` a window contention does not
/// eat.
pub const SCAN_MAX_IN_FLIGHT: usize = 64;

/// The sweep, kept apart from HOW it connects.
///
/// The split lets a test pin the limit with no network and no printer: what has to be guaranteed is
/// how many probes coexist, and with real sockets that can only be observed by causing the very
/// congestion the limit is there to avoid.
async fn sweep_addresses<C, F>(addresses: Vec<String>, probe: C) -> Vec<String>
where
    C: Fn(String) -> F + Clone + Send + 'static,
    F: std::future::Future<Output = bool> + Send + 'static,
{
    // The semaphore does NOT reduce what is probed: all 254 are probed anyway, in batches of
    // SCAN_MAX_IN_FLIGHT. What it caps is how many compete at once for the network and for the
    // connect timeout.
    let permisos = std::sync::Arc::new(tokio::sync::Semaphore::new(SCAN_MAX_IN_FLIGHT));

    let mut set: JoinSet<Option<String>> = JoinSet::new();
    for addr in addresses {
        let probe = probe.clone();
        let permisos = permisos.clone();
        set.spawn(async move {
            // `acquire_owned` only fails if the semaphore is closed, and it is never closed here.
            let _permiso = permisos.acquire_owned().await.ok()?;
            probe(addr.clone()).await.then_some(addr)
        });
    }

    let mut hits = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(Some(addr)) = joined {
            hits.push(addr);
        }
    }
    hits
}

/// mDNS/Bonjour discovery. On any error it returns an empty `Vec` instead of propagating it.
pub async fn discover_mdns() -> Result<Vec<PrinterInfo>> {
    // The `mdns-sd` daemon uses synchronous `flume` channels, so it runs on a blocking thread
    // rather than hogging the async runtime for the whole listening window.
    let result = tokio::task::spawn_blocking(browse_mdns_blocking).await;

    match result {
        Ok(printers) => Ok(printers),
        Err(e) => {
            tracing::error!("mDNS discovery failed (join): {e}");
            Ok(Vec::new())
        }
    }
}

/// The blocking half of mDNS: starts the daemon, browses the service types, listens for the window
/// and maps every `ServiceResolved` to a `PrinterInfo`. Never fails: on an error it logs and goes
/// on.
fn browse_mdns_blocking() -> Vec<PrinterInfo> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let mut printers: Vec<PrinterInfo> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("could not start the mDNS daemon: {e}");
            return printers;
        }
    };

    // The service type is kept next to its receiver: it is the ONLY thing that tells an office
    // printer (`_ipp._tcp`) from a thermal one.
    let mut receivers = Vec::new();
    for service_type in MDNS_SERVICE_TYPES {
        match daemon.browse(service_type) {
            Ok(rx) => receivers.push((service_type, rx)),
            Err(e) => tracing::debug!("could not browse {service_type}: {e}"),
        }
    }

    let deadline = std::time::Instant::now() + MDNS_BROWSE_WINDOW;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline - now;

        // Poll every receiver with a short timeout until the window runs out.
        let mut progressed = false;
        for (service_type, rx) in &receivers {
            let slice = remaining.min(Duration::from_millis(100));
            match rx.recv_timeout(slice) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    progressed = true;
                    if let Some(printer) = service_to_printer(&info, service_type) {
                        if seen_ids.insert(printer.id.clone()) {
                            tracing::debug!(
                                "mDNS printer found: {} ({})",
                                printer.name,
                                printer.id
                            );
                            printers.push(printer);
                        }
                    }
                }
                Ok(_) => {
                    progressed = true;
                }
                Err(_) => {
                    // This receiver timed out or disconnected: try the next one.
                }
            }
        }

        if !progressed {
            // Nothing pending: avoid a busy loop until the window runs out.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    let _ = daemon.shutdown();
    printers
}

/// Maps a resolved `ServiceInfo` to a network `PrinterInfo`. `None` when it has no address.
fn service_to_printer(info: &mdns_sd::ServiceInfo, service_type: &str) -> Option<PrinterInfo> {
    let ip = info.get_addresses().iter().next()?.to_string();
    let port = match info.get_port() {
        0 => ESCPOS_NETWORK_PORT,
        p => p,
    };
    // The name is the first segment of the full name.
    let fullname = info.get_fullname();
    let name = match fullname.split('.').next() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => format!("mDNS Printer ({ip})"),
    };

    Some(PrinterInfo {
        id: format!("network:{ip}:{port}"),
        name,
        kind: "network".into(),
        category: category_for_service(service_type).to_string(),
        status: "ready".into(),
        paper_width: 80,
        mac: None,
    })
}

/// The /24 networks to sweep, as prefixes (`192.168.1`): the one of the default route and those of
/// the device's own addresses. The default route alone is not enough: a shop network without
/// internet has none, and nothing would be swept (app#17). Nobody outside is asked.
///
/// `pub(crate)` because the `registry` watchdog sweeps exactly the same networks, and two copies of
/// one rule drift apart.
pub(crate) fn local_subnet_prefixes() -> Vec<String> {
    let interfaces: Vec<IpAddr> = match if_addrs::get_if_addrs() {
        Ok(list) => list.iter().map(if_addrs::Interface::ip).collect(),
        Err(e) => {
            tracing::warn!(
                "cannot list the network interfaces, sweeping the default route only: {e}"
            );
            Vec::new()
        }
    };
    subnet_prefixes(default_route_ip(), &interfaces)
}

/// Every host address (`.1..=254`) of each /24 prefix, in order.
pub(crate) fn sweep_targets(prefixes: &[String]) -> Vec<String> {
    prefixes
        .iter()
        .flat_map(|prefix| (1u8..=254).map(move |i| format!("{prefix}.{i}")))
        .collect()
}

/// The local address the default route leaves by, if there is one. `connect` on UDP only asks the
/// routing table; without a route out it fails with ENETUNREACH.
fn default_route_ip() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    let local: SocketAddr = sock.local_addr().ok()?;
    Some(local.ip())
}

/// Which /24 networks a device with these addresses sweeps, each once, in this order: the one the
/// default route leaves by (whatever its address), then the one of every PRIVATE IPv4 address it
/// holds (RFC 1918: Wi-Fi, cable). Loopback, link-local, public or carrier addresses and IPv6 are
/// not a shop network: sweeping a public /24 knocks on strangers' doors.
pub(crate) fn subnet_prefixes(route: Option<IpAddr>, interfaces: &[IpAddr]) -> Vec<String> {
    let private = interfaces
        .iter()
        .filter(|ip| matches!(ip, IpAddr::V4(v4) if v4.is_private()));
    let mut prefixes: Vec<String> = Vec::new();
    for ip in route.iter().chain(private) {
        if let Some(prefix) = subnet_prefix_of(&ip.to_string()) {
            if !prefixes.contains(&prefix) {
                prefixes.push(prefix);
            }
        }
    }
    prefixes
}

/// The first three octets of an IPv4 (`192.168.1.42` → `192.168.1`). Anything else — IPv6, a short
/// string — has no /24 subnet to sweep and returns `None`.
fn subnet_prefix_of(ip: &str) -> Option<String> {
    let mut parts = ip.split('.');
    let a = parts.next()?;
    let b = parts.next()?;
    let c = parts.next()?;
    // A fourth octet must be there, or this is no IPv4.
    parts.next()?;
    Some(format!("{a}.{b}.{c}"))
}

/// Classifies a printer by the mDNS service that announced it.
///
/// `_ipp._tcp` is published by office multifunction and AirPrint printers and by hardly any ESC/POS
/// thermal printer, so it is the only reliable sign of "A4". `_pdl-datastream._tcp` (raw printing
/// on 9100) is announced by **both families**, so it classifies nothing.
pub fn category_for_service(service_type: &str) -> &'static str {
    if service_type.starts_with("_ipp.") {
        crate::protocol::PRINTER_CATEGORY_A4
    } else {
        crate::protocol::PRINTER_CATEGORY_UNKNOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PRINTER_CATEGORY_A4, PRINTER_CATEGORY_UNKNOWN};

    /// The sweep cannot launch all 254 probes at once.
    ///
    /// Each probe has only `SCAN_CONNECT_TIMEOUT` to connect; one waiting behind 253 others runs
    /// out of time and **the real printer is dropped silently** — the user sees «no printers» and
    /// nothing in the logs tells it apart from a venue without a printer. Measured against real
    /// hardware: two simultaneous sweeps lost it 3 times out of 3.
    #[tokio::test]
    async fn the_sweep_never_opens_more_probes_than_it_can_serve() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let en_vuelo = Arc::new(AtomicUsize::new(0));
        let pico = Arc::new(AtomicUsize::new(0));

        let direcciones: Vec<String> = (1..=254).map(|i| format!("10.0.0.{i}")).collect();
        let (v, p) = (en_vuelo.clone(), pico.clone());
        sweep_addresses(direcciones, move |_addr| {
            let (v, p) = (v.clone(), p.clone());
            async move {
                let ahora = v.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(ahora, Ordering::SeqCst);
                // Without the wait a probe ends before the next one starts and the peak never
                // rises: the congestion being measured needs them to overlap.
                tokio::time::sleep(Duration::from_millis(20)).await;
                v.fetch_sub(1, Ordering::SeqCst);
                false
            }
        })
        .await;

        let pico = pico.load(Ordering::SeqCst);
        assert!(
            pico <= SCAN_MAX_IN_FLIGHT,
            "the sweep had {pico} probes in flight at once, above the limit of \
             {SCAN_MAX_IN_FLIGHT}: the ones waiting behind run out of time and the printer is \
             lost WITHOUT an error"
        );
    }

    /// The limit caps, it does not cut: all 254 addresses are probed anyway, only in batches.
    /// Without this, «fixing» the concurrency by probing fewer addresses would leave half the /24
    /// unswept and the test above would stay green.
    #[tokio::test]
    async fn the_sweep_still_probes_every_address() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let sondeadas = Arc::new(AtomicUsize::new(0));
        let s = sondeadas.clone();
        let encontradas = sweep_addresses(
            (1..=254).map(|i| format!("10.0.0.{i}")).collect(),
            move |addr| {
                let s = s.clone();
                async move {
                    s.fetch_add(1, Ordering::SeqCst);
                    addr.ends_with(".42")
                }
            },
        )
        .await;

        assert_eq!(sondeadas.load(Ordering::SeqCst), 254, "some addresses were never probed");
        assert_eq!(encontradas, vec!["10.0.0.42".to_string()], "it must return only the hits");
    }

    #[test]
    fn ipp_gives_away_an_office_printer() {
        assert_eq!(category_for_service("_ipp._tcp.local."), PRINTER_CATEGORY_A4);
    }

    #[test]
    fn raw_printing_classifies_nothing() {
        // `_pdl-datastream._tcp` means "I take bytes on 9100": a thermal printer and an A4 laser
        // both say it. Calling it thermal would be guessing.
        assert_eq!(
            category_for_service("_pdl-datastream._tcp.local."),
            PRINTER_CATEGORY_UNKNOWN
        );
    }

    #[test]
    fn a_printer_found_only_by_the_port_sweep_stays_unclassified() {
        // The 9100 sweep brings no signal: answering there says nothing about the language spoken.
        assert_eq!(crate::protocol::default_printer_category(), PRINTER_CATEGORY_UNKNOWN);
    }

    // ── `bluetooth:{mac}` in the printer_id contract ──────────────────────────────────────────
    //
    // A Kotlin plugin runs in-process with the full Android APIs, so the cheap bonded Bluetooth
    // thermal printer at the table can be driven from the app. Only Android produces and accepts
    // the variant.

    #[test]
    fn a_bluetooth_printer_id_is_a_valid_print_target_now() {
        match parse_print_target("bluetooth:aa:bb:cc:dd:ee:ff") {
            Ok(PrintTarget::Bluetooth(t)) => {
                // Normalized on the way in: the MAC is the device's identity in the registry and
                // in the Kotlin transport, and two spellings of one printer would be two devices.
                assert_eq!(t.mac, "AA:BB:CC:DD:EE:FF");
            }
            other => panic!("bluetooth:{{mac}} must parse as a Bluetooth target, got {other:?}"),
        }
    }

    #[test]
    fn a_network_print_target_parses_exactly_as_before() {
        match parse_print_target("network:10.0.0.5:9100") {
            Ok(PrintTarget::Network(t)) => {
                assert_eq!(t.host, "10.0.0.5");
                assert_eq!(t.port, 9100);
            }
            other => panic!("network ids must keep parsing unchanged, got {other:?}"),
        }
    }

    // ── `usb:{queue}` in the printer_id contract, on the desktop ──────────────────────────────
    //
    // USB does not need a driver per OS from us: the OS print queue IS the generic transport, the
    // vendor writes the driver, and we keep emitting the ESC/POS we already emit. The USB thermal
    // printer is the cheapest in the catalogue and the one a single-till bar or salon actually
    // buys.

    #[test]
    fn a_usb_printer_id_names_an_os_queue() {
        match parse_print_target("usb:Star_TSP143") {
            Ok(PrintTarget::Usb(t)) => assert_eq!(t.queue, "Star_TSP143"),
            other => panic!("usb:{{queue}} must parse as a USB target, got {other:?}"),
        }
    }

    #[test]
    fn a_queue_name_the_os_could_never_have_is_refused_at_the_edge() {
        // CUPS forbids SPACE, TAB, `/` and `#` in a destination name, so an id carrying one names
        // a queue that cannot exist. Refusing it HERE keeps it from travelling on to come back as
        // an `lp` exit code nobody can map to the printer the cashier picked.
        assert!(parse_print_target("usb:").is_err(), "an empty queue names nothing");
        assert!(parse_print_target("usb:Star TSP143").is_err(), "CUPS forbids spaces");
        assert!(parse_print_target("usb:kitchen/star").is_err(), "CUPS forbids `/`");
        assert!(parse_print_target("usb:star#1").is_err(), "CUPS forbids `#`");
        assert!(parse_print_target("usb:star\tone").is_err(), "CUPS forbids TAB");
    }

    #[test]
    fn a_leading_dash_is_refused_so_a_queue_can_never_become_an_lp_flag() {
        // `lp -d <queue>`: a destination that starts with `-` is an argument `lp` would read as a
        // flag. There is no shell here (argv, never a string), so this is not an injection — it
        // is a queue that would silently print somewhere else, which is worse than an error.
        assert!(parse_print_target("usb:-oraw").is_err());
        assert!(parse_print_target("usb:--help").is_err());
    }

    #[test]
    fn a_malformed_bluetooth_mac_is_still_refused() {
        // A `bluetooth:` id whose rest is not a MAC must fail HERE, not travel to Kotlin to blow
        // up inside the socket connect with a message nobody maps back to the id.
        assert!(parse_print_target("bluetooth:not-a-mac").is_err());
        assert!(parse_print_target("bluetooth:").is_err());
        assert!(parse_print_target("bluetooth:AA:BB:CC:DD:EE").is_err(), "5 groups is not a MAC");
    }

    #[test]
    fn the_network_only_parser_still_refuses_usb() {
        // `parse_printer_id` is the NETWORK parser: the print queue (`PrintJob.target`), the
        // drawer's socket path and the registry enrichment all feed TCP and cannot take a queue
        // name. USB reaches hardware exclusively through `parse_print_target`'s USB arm.
        assert!(parse_printer_id("usb:Star_TSP143").is_err());
    }

    #[test]
    fn the_network_only_parser_still_refuses_bluetooth() {
        // `parse_printer_id` is the NETWORK parser: the print queue (`PrintJob.target`), the
        // drawer and the registry enrichment feed TCP sockets and cannot take a MAC. Bluetooth
        // ids reach hardware exclusively through `parse_print_target`'s Bluetooth arm (the
        // Android SPP transport), so this door stays closed.
        assert!(parse_printer_id("bluetooth:AA:BB:CC:DD:EE:FF").is_err());
        assert!(parse_printer_id("network:10.0.0.5:9100").is_ok());
    }

    /// **The /24 prefix is derived from the local IP, and that derivation is what gets tested.**
    /// The socket half (`local_subnet_prefixes`) asks the OS for its addresses and its route out,
    /// which CI cannot test; the pure half is what the sweep and the watchdog share, and what this
    /// pins.
    #[test]
    fn the_subnet_prefix_is_the_first_three_octets_of_an_ipv4() {
        assert_eq!(subnet_prefix_of("192.168.1.42"), Some("192.168.1".to_string()));
        assert_eq!(subnet_prefix_of("10.0.2.15"), Some("10.0.2".to_string()));
    }

    /// A prefix that is not a /24 IPv4 must come back as `None`, never as a truncated string: the
    /// sweep builds `{prefix}.{1..=254}` out of it, so a wrong prefix means 254 connects to
    /// addresses nobody owns — a scan that finds zero printers and blames the printer.
    #[test]
    fn anything_that_is_not_an_ipv4_has_no_scannable_subnet() {
        // IPv6: no /24 to sweep.
        assert_eq!(subnet_prefix_of("fe80::1"), None);
        // Too few octets — `"10.0.2"` would otherwise be taken for a prefix as-is.
        assert_eq!(subnet_prefix_of("10.0.2"), None);
        assert_eq!(subnet_prefix_of(""), None);
    }

    // ── A shop network without internet is still swept (app#17) ─────────────────────────────
    //
    // A shop Wi-Fi that is just the till and the printer, or a router with no uplink, has no
    // default route: looking it up fails with ENETUNREACH, so the default route alone would sweep
    // nothing and the ticket printer on 9100 would never show up. The device's own addresses say
    // which networks it is on, without asking anybody outside.

    fn ip(s: &str) -> std::net::IpAddr {
        s.parse().expect("test address")
    }

    #[test]
    fn without_a_default_route_the_network_of_the_wifi_is_swept() {
        assert_eq!(
            subnet_prefixes(None, &[ip("127.0.0.1"), ip("192.168.1.42")]),
            vec!["192.168.1".to_string()],
        );
    }

    #[test]
    fn every_private_network_the_device_is_on_is_swept_once() {
        // Cable and Wi-Fi on two networks, the cable one also holding a second address in the
        // same /24, and the default route leaving by the Wi-Fi.
        assert_eq!(
            subnet_prefixes(
                Some(ip("192.168.1.42")),
                &[
                    ip("10.0.0.7"),
                    ip("10.0.0.8"),
                    ip("192.168.1.42"),
                    ip("172.16.5.9")
                ],
            ),
            vec![
                "192.168.1".to_string(),
                "10.0.0".to_string(),
                "172.16.5".to_string()
            ],
        );
    }

    #[test]
    fn addresses_that_are_not_a_shop_network_are_not_swept() {
        // Loopback, link-local (no DHCP answered), a public or carrier address, IPv6: none is the
        // shop's LAN, and sweeping a public /24 would knock on 254 strangers' doors.
        assert_eq!(
            subnet_prefixes(
                None,
                &[
                    ip("127.0.0.1"),
                    ip("169.254.10.20"),
                    ip("100.72.3.4"),
                    ip("8.8.4.4"),
                    ip("fe80::1")
                ],
            ),
            Vec::<String>::new(),
        );
    }

    #[test]
    fn the_sweep_knocks_on_every_host_of_each_network() {
        let targets = sweep_targets(&["192.168.1".to_string(), "10.0.0".to_string()]);
        assert_eq!(targets.len(), 2 * 254);
        assert_eq!(targets[0], "192.168.1.1");
        assert_eq!(targets[253], "192.168.1.254");
        assert_eq!(targets[254], "10.0.0.1");
        assert_eq!(targets[507], "10.0.0.254");
    }

    #[test]
    fn the_network_of_the_default_route_is_swept_whatever_its_address() {
        // A LAN on a public range is still found when it has the route out.
        assert_eq!(
            subnet_prefixes(Some(ip("81.45.12.30")), &[ip("81.45.12.30")]),
            vec!["81.45.12".to_string()],
        );
    }

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

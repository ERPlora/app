//! The /24 subnet sweep: a TCP `connect` to the raw printing port on every host of each network
//! the device is on.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::task::JoinSet;

use crate::protocol::PrinterInfo;
use crate::Result;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PRINTER_CATEGORY_UNKNOWN;

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
    fn a_printer_found_only_by_the_port_sweep_stays_unclassified() {
        // The 9100 sweep brings no signal: answering there says nothing about the language spoken.
        assert_eq!(crate::protocol::default_printer_category(), PRINTER_CATEGORY_UNKNOWN);
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
}

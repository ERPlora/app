//! Self-healing watchdog: it checks the known devices periodically and sweeps the network for the
//! offline ones, to find them by MAC when DHCP has moved them to a new IP.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::discovery::sweep::{local_subnet_prefixes, sweep_targets};
use crate::protocol::Device;
use crate::registry::mac::get_mac_for_ip;
use crate::registry::DeviceRegistry;
use crate::ESCPOS_NETWORK_PORT;

/// Events the watchdog reports to its consumer (they map to `device_recovered`/`device_lost`).
#[derive(Debug, Clone)]
pub enum WatchdogEvent {
    Recovered(Device),
    Lost(Device),
}

/// Watchdog intervals.
#[derive(Debug, Clone, Copy)]
pub struct WatchdogConfig {
    pub check_interval_s: u64,
    pub recovery_interval_s: u64,
    pub connect_timeout_ms: u64,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self { check_interval_s: 30, recovery_interval_s: 120, connect_timeout_ms: 1000 }
    }
}

/// Background watchdog: checks the health of the known devices and, for the offline ones, sweeps
/// the network to find them by MAC at a new IP.
pub struct Watchdog {
    config: WatchdogConfig,
    /// Optional channel to the consumer: every `WatchdogEvent` is published here as well as
    /// through `tracing`. With no channel the watchdog only logs.
    events: Option<tokio::sync::mpsc::UnboundedSender<WatchdogEvent>>,
}

impl Watchdog {
    pub fn new(config: WatchdogConfig) -> Self {
        Self { config, events: None }
    }

    /// Wires the event channel to the consumer.
    pub fn with_events(mut self, events: tokio::sync::mpsc::UnboundedSender<WatchdogEvent>) -> Self {
        self.events = Some(events);
        self
    }

    fn emit(&self, event: WatchdogEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(event);
        }
    }

    /// Main loop: `check_devices` every `check_interval`, `recovery_scan` every
    /// `recovery_interval`.
    pub async fn run(&self, registry: &DeviceRegistry) {
        let check_interval = Duration::from_secs(self.config.check_interval_s);
        let recovery_interval = Duration::from_secs(self.config.recovery_interval_s);

        // Give the system a moment to settle before the first check.
        tokio::time::sleep(Duration::from_secs(5)).await;

        let mut last_recovery = Instant::now() - recovery_interval;

        loop {
            self.check_devices(registry).await;

            if last_recovery.elapsed() >= recovery_interval {
                self.recovery_scan(registry).await;
                last_recovery = Instant::now();
            }

            tokio::time::sleep(check_interval).await;
        }
    }

    /// Does the watchdog watch over this device at all?
    ///
    /// Only NETWORK devices: the whole toolbox here is IP-shaped — a TCP probe for health, an ARP
    /// sweep for recovery. A Bluetooth SPP printer has neither an IP to probe nor a subnet to be
    /// found on, so "monitoring" it could only mark it offline forever, and the recovery sweep —
    /// which hunts by MAC, the one thing a BT printer always has — would happily "recover" it
    /// onto some LAN device's IP. BT devices get no health path on purpose.
    pub fn monitors(device: &Device) -> bool {
        device.kind == "network"
    }

    /// Checks whether every known device is reachable.
    async fn check_devices(&self, registry: &DeviceRegistry) {
        for device in registry.get_all() {
            if !Self::monitors(&device) {
                continue;
            }
            // The health check only needs the IP and goes by `key`, so printers without a MAC (all
            // of them on Android) are watched too. The MAC is only needed by the recovery scan
            // below.
            if device.ip.is_empty() {
                continue;
            }

            let port = if device.port == 0 { ESCPOS_NETWORK_PORT } else { device.port };
            let reachable = tcp_check(&device.ip, port, self.config.connect_timeout_ms).await;

            if reachable && device.status != "online" {
                if let Err(e) = registry.set_status(&device.key, "online") {
                    tracing::warn!("could not save the online status of {}: {e}", device.key);
                }
                tracing::info!(
                    "device {} ({}) back online at {}:{}",
                    device.name,
                    device.key,
                    device.ip,
                    port
                );
                let recovered = registry.get(&device.key).unwrap_or_else(|| device.clone());
                self.emit(WatchdogEvent::Recovered(recovered));
            } else if !reachable && device.status == "online" {
                if let Err(e) = registry.set_status(&device.key, "offline") {
                    tracing::warn!("could not save the offline status of {}: {e}", device.key);
                }
                tracing::warn!(
                    "device {} ({}) went offline at {}:{}",
                    device.name,
                    device.key,
                    device.ip,
                    port
                );
                let lost = registry.get(&device.key).unwrap_or_else(|| device.clone());
                self.emit(WatchdogEvent::Lost(lost));
            }
        }
    }

    /// Full recovery scan: for every offline device, sweeps the subnet looking for its MAC at a
    /// new IP.
    async fn recovery_scan(&self, registry: &DeviceRegistry) {
        // Only devices WITH a MAC take part: recognising a printer at a new IP needs an identity
        // that does not depend on the IP, and the key of those without a MAC *is* the IP. Without
        // ARP there is no recovery after a DHCP change — a real limit, not an omission: on Android
        // the user has to discover the printer again.
        let mut mac_lookup: HashMap<String, Device> = registry
            .get_all()
            .into_iter()
            // A Bluetooth printer always has a MAC and never has an IP — the perfect FALSE
            // candidate for an ARP-based recovery. It is not lost on the LAN; it was never there.
            .filter(Self::monitors)
            .filter(|d| d.status == "offline")
            .filter_map(|d| d.mac.clone().map(|mac| (mac, d)))
            .collect();

        if mac_lookup.is_empty() {
            return;
        }

        tracing::info!("recovery scan: looking for {} offline device(s)", mac_lookup.len());

        let prefixes = local_subnet_prefixes();
        if prefixes.is_empty() {
            tracing::warn!("recovery scan: no IPv4 network to sweep");
            return;
        }

        for ip in sweep_targets(&prefixes) {
            if mac_lookup.is_empty() {
                break; // Every offline device recovered.
            }

            if !tcp_check(&ip, ESCPOS_NETWORK_PORT, self.config.connect_timeout_ms).await {
                continue;
            }

            // Something answers on 9100: resolve its MAC.
            let mac = match get_mac_for_ip(&ip) {
                Some(m) => m,
                None => continue,
            };

            if let Some(device) = mac_lookup.remove(&mac) {
                tracing::info!(
                    "RECOVERED device {} ({}): {} -> {}",
                    device.name,
                    mac,
                    device.ip,
                    ip
                );
                if let Err(e) = registry.update_ip(&mac, &ip) {
                    tracing::warn!("could not update the IP of {mac}: {e}");
                }
                if let Err(e) = registry.set_status(&mac, "online") {
                    tracing::warn!("could not mark {mac} online: {e}");
                }
                let recovered = registry.get_by_mac(&mac).unwrap_or(device);
                self.emit(WatchdogEvent::Recovered(recovered));
            }
        }

        let still_offline = mac_lookup.len();
        if still_offline > 0 {
            tracing::info!("recovery scan done: {still_offline} device(s) still offline");
        }
    }
}

/// Whether a TCP port answers within `timeout_ms`.
pub async fn tcp_check(host: &str, port: u16, timeout_ms: u64) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            tokio::net::TcpStream::connect((host, port)),
        )
        .await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── tcp_check: the watchdog's reachability probe ────────────────────────────────────────────

    /// **The probe tells a live printer from a dead one, and it is the only thing that does.** The
    /// watchdog marks `offline` and triggers the recovery sweep from this answer: if it always said
    /// `true`, a printer DHCP moved to another IP would stay online forever and the tickets would
    /// go to an address that is no longer its own.
    #[tokio::test]
    async fn the_probe_says_reachable_only_while_something_is_listening() {
        let mock = crate::test_support::MockPrinter::start().await;
        assert!(
            tcp_check(&mock.target.host, mock.target.port, 500).await,
            "with a listener up the probe has to see the printer"
        );

        let dead = crate::test_support::unreachable_target();
        assert!(
            !tcp_check(&dead.host, dead.port, 500).await,
            "a closed port is a printer that is off, not a live one"
        );
    }

    /// A destination that does not even route cannot hang the probe: the watchdog sweeps 254
    /// addresses one after the other, so a single wait with no ceiling freezes the whole recovery.
    #[tokio::test]
    async fn the_probe_gives_up_within_its_timeout() {
        // `192.0.2.0/24` is TEST-NET-1 (RFC 5737): it is not routed, so the connect never answers.
        let started = Instant::now();
        assert!(!tcp_check("192.0.2.1", ESCPOS_NETWORK_PORT, 120).await);
        assert!(
            started.elapsed() < Duration::from_millis(2_000),
            "the probe must give up on its own timeout, not the OS's (took {:?})",
            started.elapsed()
        );
    }

    // ── The watchdog is a NETWORK mechanism, and says so ───────────────────────────────────────
    //
    // Its whole toolbox is IP-shaped: a TCP probe to decide alive/dead and an ARP sweep to find a
    // printer that DHCP moved. A Bluetooth SPP printer has neither an IP to probe nor a subnet to
    // be found on — running either against it can only produce noise (a permanent "offline") or a
    // false recovery onto some LAN device that happens to share the MAC. BT devices get NO health
    // path on purpose; this predicate is where that decision lives.

    fn device_of_kind(kind: &str, ip: &str) -> Device {
        Device {
            key: "AA:BB:CC:DD:EE:FF".into(),
            mac: Some("AA:BB:CC:DD:EE:FF".into()),
            ip: ip.into(),
            port: 0,
            name: "Printer".into(),
            role: None,
            kind: kind.into(),
            first_seen: "2026-08-13T00:00:00".into(),
            last_seen: "2026-08-13T00:00:00".into(),
            status: "online".into(),
            manual: false,
        }
    }

    #[test]
    fn the_watchdog_monitors_network_devices_and_leaves_bluetooth_alone() {
        assert!(Watchdog::monitors(&device_of_kind("network", "10.0.0.5")));
        assert!(
            !Watchdog::monitors(&device_of_kind("bluetooth", "")),
            "a BT printer has no IP to probe: 'monitoring' it can only mark it offline forever"
        );
    }

    #[test]
    fn a_bluetooth_device_never_enters_the_recovery_sweep() {
        // The recovery sweep hunts offline devices BY MAC across the local subnet — and a BT
        // printer always HAS a MAC, so without this gate it is the perfect false candidate: any
        // LAN device answering 9100 with a matching-looking MAC would "recover" it onto an IP it
        // never had.
        let bt = {
            let mut d = device_of_kind("bluetooth", "");
            d.status = "offline".into();
            d
        };
        assert!(!Watchdog::monitors(&bt));
    }
}

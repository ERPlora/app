//! Persistent device registry (JSON). MAC resolution through ARP lives in [`mac`] and the
//! self-healing watchdog in [`watchdog`].

pub mod mac;
pub mod watchdog;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use self::mac::normalize_mac;
use crate::protocol::Device;
use crate::Result;

/// Transport of an OS print queue in the registry (`Device::kind`).
///
/// The same scheme as its `printer_id` (`usb:{queue}`), in a field of its own so a screen can group
/// by cable without parsing the id — like `"network"` and `"bluetooth"`.
pub const KIND_USB: &str = "usb";

/// Stable identity of a device **with a socket**: network and Bluetooth.
///
/// The normalized MAC when the system could resolve it through ARP; otherwise the `printer_id`
/// (`network:{ip}:{port}`). Without this fallback a device with no MAC could not be registered and
/// so **could not take a role** (kitchen/bar/till) — which is exactly what the `printing` module
/// needs. It always happens on Android, and with a VPN, containers or a firewall on the desktop.
///
/// **Not for an OS queue** (`usb:{queue}`): it has no MAC to resolve and no `ip:port` to fall back
/// on, so this function would give `network::0` to **every** queue and the second one registered
/// would overwrite the first, together with the role its owner gave it. That identity comes from
/// [`DeviceRegistry::register_queue`], which uses the whole `printer_id`.
pub fn device_key(mac: Option<&str>, ip: &str, port: u16) -> String {
    match mac {
        Some(m) if !m.trim().is_empty() => normalize_mac(m.trim()),
        _ => format!("network:{ip}:{port}"),
    }
}

/// Local ISO timestamp, to the second.
fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Persistent key → device registry, backed by `devices.json` in the config directory.
///
/// The key is [`device_key`]: the normalized MAC when it is known and the `printer_id`
/// (`network:{ip}:{port}`) when it is not, so printers whose MAC ARP cannot resolve — all of them
/// on Android — still get in and can take a role.
pub struct DeviceRegistry {
    path: PathBuf,
    /// In-memory key → device map. A synchronous `RwLock`: the operations are quick and the lock
    /// is never held across an `await`.
    devices: RwLock<HashMap<String, Device>>,
}

impl DeviceRegistry {
    /// Loads the registry from `path` (empty if the file is missing or corrupt).
    pub fn load(path: PathBuf) -> Self {
        let devices = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(text) => match serde_json::from_str::<DevicesFile>(&text) {
                    Ok(file) => Self::backfill_keys(file.devices),
                    Err(exc) => {
                        tracing::warn!("could not parse the device registry {path:?}: {exc}");
                        HashMap::new()
                    }
                },
                Err(exc) => {
                    tracing::warn!("could not read the device registry {path:?}: {exc}");
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Self { path, devices: RwLock::new(devices) }
    }

    /// Fills in `key` for files written before the field existed (they were keyed by MAC, so the
    /// map key **is** the identity). `devices.json` is user data: a customer with roles already
    /// assigned must not lose them on an update.
    fn backfill_keys(devices: HashMap<String, Device>) -> HashMap<String, Device> {
        devices
            .into_iter()
            .map(|(map_key, mut device)| {
                if device.key.trim().is_empty() {
                    device.key = map_key.clone();
                }
                (device.key.clone(), device)
            })
            .collect()
    }

    pub fn save(&self) -> Result<()> {
        let payload = {
            let map = self.devices.read().expect("registry lock poisoned");
            serde_json::to_string_pretty(&serde_json::json!({ "devices": &*map }))?
        };

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, payload)?;
        Ok(())
    }

    /// Adds or updates a device **with a socket** (network, Bluetooth) under [`device_key`];
    /// keeps `first_seen` and `role`.
    ///
    /// `mac` is an `Option` on purpose: when ARP cannot resolve it the device **is registered
    /// anyway**, identified by its `printer_id`.
    ///
    /// An OS queue does not come in through here: its door is [`Self::register_queue`].
    pub fn register(
        &self,
        mac: Option<&str>,
        ip: &str,
        port: u16,
        name: &str,
        kind: &str,
    ) -> Result<Device> {
        let mac = mac
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(normalize_mac);
        // Neither a MAC nor a host: [`device_key`] answers `network::0` — the SAME key for every
        // caller in that state, so the next one silently overwrites this entry and inherits the
        // role its owner had assigned. A transport with no socket has its own door,
        // [`Self::register_queue`]; anything else arriving here is a discovery that failed
        // upstream, and it must say so instead of writing a shared ghost.
        if mac.is_none() && ip.trim().is_empty() {
            return Err(crate::PeripheralError::InvalidPrinterId(format!(
                "`{name}` has neither a MAC nor an address: it cannot be identified in the \
                 registry (an OS print queue registers through its printer_id)"
            )));
        }
        self.upsert(device_key(mac.as_deref(), ip, port), mac, ip, port, name, kind, false)
    }

    /// Adds or updates an **OS print queue** (`usb:{queue}`), identified by its whole
    /// `printer_id`.
    ///
    /// A queue has no MAC to resolve through ARP and no `ip:port` to fall back on, so
    /// [`device_key`] would give `network::0` to all of them and the second would overwrite the
    /// first. Its `printer_id` is **already** unique on the machine — a CUPS destination name is —
    /// so it IS its identity, and with it the queue enters the registry like any other printer: it
    /// shows up in `get_devices` and takes a role (kitchen/bar/till), which is what the owner
    /// expects to be able to say about the counter printer even when it is on a cable.
    ///
    /// The `printer_id` is validated with the SAME parser as the print path
    /// ([`crate::print_target::parse_print_target`]): an id with no queue behind it — or one of
    /// another transport, which has its own identity — would create a ghost entry the next queue
    /// would inherit, role included.
    pub fn register_queue(&self, printer_id: &str, name: &str) -> Result<Device> {
        let printer_id = printer_id.trim();
        match crate::print_target::parse_print_target(printer_id) {
            Ok(crate::print_target::PrintTarget::Usb(_)) => {}
            Ok(_) => {
                return Err(crate::PeripheralError::InvalidPrinterId(format!(
                    "`{printer_id}` is not an OS print queue: a transport with a socket registers \
                     by its MAC or its address"
                )))
            }
            Err(exc) => return Err(exc),
        }
        self.upsert(printer_id.to_string(), None, "", 0, name, KIND_USB, false)
    }

    /// Adds a network printer whose address the owner TYPED: the same entry a scan would leave,
    /// marked [`Device::manual`] so the next discovery keeps listing it even when it cannot see it.
    pub fn register_manual(&self, ip: &str, port: u16, name: &str) -> Result<Device> {
        if ip.trim().is_empty() {
            return Err(crate::PeripheralError::InvalidPrinterId(format!(
                "`{name}` has no address to register"
            )));
        }
        self.upsert(device_key(None, ip, port), None, ip, port, name, "network", true)
    }

    /// The write itself, once the identity is settled. Keeps `first_seen`, `role` and the
    /// [`Device::manual`] mark: a scan that sees a typed printer does not demote it to scanned.
    #[expect(clippy::too_many_arguments, reason = "excused until its fields travel as one struct")]
    fn upsert(
        &self,
        key: String,
        mac: Option<String>,
        ip: &str,
        port: u16,
        name: &str,
        kind: &str,
        manual: bool,
    ) -> Result<Device> {
        let now = now_iso();

        let device = {
            let mut map = self.devices.write().expect("registry lock poisoned");
            match map.get_mut(&key) {
                Some(existing) => {
                    existing.ip = ip.to_string();
                    existing.port = port;
                    existing.name = name.to_string();
                    existing.kind = kind.to_string();
                    existing.last_seen = now;
                    existing.manual |= manual;
                    // A MAC that shows up later (e.g. ARP answers on the second scan) enriches
                    // the entry, but a known one is never erased.
                    if mac.is_some() {
                        existing.mac = mac;
                    }
                    existing.clone()
                }
                None => {
                    let device = Device {
                        key: key.clone(),
                        mac,
                        ip: ip.to_string(),
                        port,
                        name: name.to_string(),
                        role: None,
                        kind: kind.to_string(),
                        first_seen: now.clone(),
                        last_seen: now,
                        status: "online".to_string(),
                        manual,
                    };
                    map.insert(key.clone(), device.clone());
                    device
                }
            }
        };

        self.save()?;
        Ok(device)
    }

    /// Resolves an entry from its key **or** its MAC.
    ///
    /// The JSON protocol calls this parameter `mac` (`SetDeviceRole { mac }`) and callers pass
    /// whichever they have at hand, so both forms are accepted: first as is (a `printer_id`), then
    /// normalized as a MAC.
    fn resolve_key(&self, key_or_mac: &str) -> Option<String> {
        let map = self.devices.read().expect("registry lock poisoned");
        if map.contains_key(key_or_mac) {
            return Some(key_or_mac.to_string());
        }
        let normalized = normalize_mac(key_or_mac);
        map.contains_key(&normalized).then_some(normalized)
    }

    pub fn get(&self, key_or_mac: &str) -> Option<Device> {
        let key = self.resolve_key(key_or_mac)?;
        let map = self.devices.read().expect("registry lock poisoned");
        map.get(&key).cloned()
    }

    /// Same as [`Self::get`]: it takes a key or a MAC too.
    pub fn get_by_mac(&self, mac: &str) -> Option<Device> {
        self.get(mac)
    }

    pub fn get_by_ip(&self, ip: &str) -> Option<Device> {
        let map = self.devices.read().expect("registry lock poisoned");
        map.values().find(|d| d.ip == ip).cloned()
    }

    /// Every device, most recently seen first.
    pub fn get_all(&self) -> Vec<Device> {
        let map = self.devices.read().expect("registry lock poisoned");
        let mut devices: Vec<Device> = map.values().cloned().collect();
        devices.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        devices
    }

    /// Assigns the role (`receipt`/`kitchen`/`bar`…). Takes a key or a MAC.
    ///
    /// A key the registry does not hold is an ERROR, not a no-op: the only way to get here is a
    /// registration that failed upstream, and "your click did nothing" must not be how the user
    /// finds out. A discovered `usb:{queue}` is registered like any other printer and takes its
    /// role the same way.
    pub fn set_role(&self, key_or_mac: &str, role: &str) -> Result<()> {
        let Some(key) = self.resolve_key(key_or_mac) else {
            return Err(crate::PeripheralError::InvalidPrinterId(format!(
                "no registered device under `{key_or_mac}`; scan for printers again and retry"
            )));
        };
        let mutated = {
            let mut map = self.devices.write().expect("registry lock poisoned");
            match map.get_mut(&key) {
                Some(device) => {
                    device.role = Some(role.to_string());
                    true
                }
                None => false,
            }
        };
        if mutated {
            self.save()?;
        }
        Ok(())
    }

    /// Marks the device online or offline. Takes a key or a MAC.
    pub fn set_status(&self, key_or_mac: &str, status: &str) -> Result<()> {
        let Some(key) = self.resolve_key(key_or_mac) else {
            return Ok(());
        };
        let mutated = {
            let mut map = self.devices.write().expect("registry lock poisoned");
            match map.get_mut(&key) {
                Some(device) => {
                    device.status = status.to_string();
                    true
                }
                None => false,
            }
        };
        if mutated {
            self.save()?;
        }
        Ok(())
    }

    /// Renames the device. Takes a key or a MAC.
    pub fn set_name(&self, key_or_mac: &str, name: &str) -> Result<()> {
        let Some(key) = self.resolve_key(key_or_mac) else {
            return Ok(());
        };
        let mutated = {
            let mut map = self.devices.write().expect("registry lock poisoned");
            match map.get_mut(&key) {
                Some(device) => {
                    device.name = name.to_string();
                    true
                }
                None => false,
            }
        };
        if mutated {
            self.save()?;
        }
        Ok(())
    }

    /// Updates the IP (self-healing after a DHCP change).
    /// Takes a key or a MAC — but it only makes sense with a MAC: without one the device cannot be
    /// recognised at a new IP (and the key, which IS the IP, would no longer hold).
    pub fn update_ip(&self, key_or_mac: &str, new_ip: &str) -> Result<()> {
        let Some(key) = self.resolve_key(key_or_mac) else {
            return Ok(());
        };
        let mutated = {
            let mut map = self.devices.write().expect("registry lock poisoned");
            match map.get_mut(&key) {
                Some(device) => {
                    device.ip = new_ip.to_string();
                    device.last_seen = now_iso();
                    true
                }
                None => false,
            }
        };
        if mutated {
            self.save()?;
        }
        Ok(())
    }

    /// Removes the device from the registry. Takes a key or a MAC.
    pub fn remove(&self, key_or_mac: &str) -> Result<()> {
        let Some(key) = self.resolve_key(key_or_mac) else {
            return Ok(());
        };
        let mutated = {
            let mut map = self.devices.write().expect("registry lock poisoned");
            map.remove(&key).is_some()
        };
        if mutated {
            self.save()?;
        }
        Ok(())
    }
}

/// The `{ "devices": { key: Device } }` file, as read from disk.
#[derive(serde::Deserialize)]
struct DevicesFile {
    #[serde(default)]
    devices: HashMap<String, Device>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path of its own per test under the system temp dir (the registry writes to disk).
    fn temp_registry(name: &str) -> (DeviceRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("erplora-registry-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("devices.json");
        (DeviceRegistry::load(path.clone()), path)
    }

    // ── device_key: the stable identity ──────────────────────────────────────────────────────────

    #[test]
    fn the_key_is_the_normalized_mac_when_known() {
        assert_eq!(device_key(Some("aa-bb-cc-dd-ee-f1"), "10.0.0.5", 9100), "AA:BB:CC:DD:EE:F1");
    }

    #[test]
    fn the_key_falls_back_to_the_printer_id_without_a_mac() {
        // Android NEVER has a MAC (there is no `arp` binary and /proc/net/arp is restricted since
        // Android 10), and on the desktop it fails with a VPN, containers or a firewall.
        assert_eq!(device_key(None, "10.0.0.5", 9100), "network:10.0.0.5:9100");
        assert_eq!(device_key(Some("   "), "10.0.0.5", 9100), "network:10.0.0.5:9100");
    }

    // ── A printer without a MAC is registered and takes a role ──────────────────────────────────

    #[test]
    fn registers_a_printer_without_a_mac() {
        let (registry, _) = temp_registry("registra-sin-mac");
        let device = registry.register(None, "10.0.0.5", 9100, "Caja", "network").unwrap();

        assert_eq!(device.key, "network:10.0.0.5:9100");
        assert_eq!(device.mac, None, "without ARP no MAC is made up");
        assert_eq!(registry.get_all().len(), 1, "the printer MUST enter the registry");
    }

    #[test]
    fn assigns_a_role_to_a_printer_without_a_mac() {
        // Without a MAC the device would not reach `devices.json`, `get_devices` would return []
        // and no printer could be marked kitchen/bar/till: the `printing` module would be blocked
        // on Android.
        let (registry, _) = temp_registry("rol-sin-mac");
        let device = registry.register(None, "10.0.0.7", 9100, "Cocina", "network").unwrap();

        registry.set_role(&device.key, "kitchen").unwrap();

        let stored = registry.get(&device.key).expect("the device is still in the registry");
        assert_eq!(stored.role.as_deref(), Some("kitchen"));
    }

    // ── With a MAC, the MAC stays the key ────────────────────────────────────────────────────────

    #[test]
    fn with_a_mac_the_key_is_still_the_normalized_mac() {
        let (registry, _) = temp_registry("clave-es-mac");
        let device = registry
            .register(Some("aa-bb-cc-dd-ee-f2"), "10.0.0.8", 9100, "Barra", "network")
            .unwrap();

        assert_eq!(device.key, "AA:BB:CC:DD:EE:F2");
        assert_eq!(device.mac.as_deref(), Some("AA:BB:CC:DD:EE:F2"));
    }

    #[test]
    fn mutations_accept_either_the_mac_or_the_key() {
        // The JSON protocol calls this parameter `mac` (`SetDeviceRole { mac }`) and callers pass
        // whichever they have at hand.
        let (registry, _) = temp_registry("mac-o-clave");
        registry
            .register(Some("AA:BB:CC:DD:EE:F3"), "10.0.0.9", 9100, "Caja", "network")
            .unwrap();

        registry.set_role("aa-bb-cc-dd-ee-f3", "receipt").unwrap();
        assert_eq!(
            registry.get("AA:BB:CC:DD:EE:F3").unwrap().role.as_deref(),
            Some("receipt"),
            "an unnormalized MAC must resolve to the same device"
        );

        registry.set_name("AA:BB:CC:DD:EE:F3", "Caja principal").unwrap();
        assert_eq!(registry.get("AA:BB:CC:DD:EE:F3").unwrap().name, "Caja principal");
    }

    #[test]
    fn re_registering_keeps_first_seen_and_the_role() {
        let (registry, _) = temp_registry("preserva");
        let first = registry.register(None, "10.0.0.10", 9100, "Caja", "network").unwrap();
        registry.set_role(&first.key, "receipt").unwrap();

        let again = registry.register(None, "10.0.0.10", 9100, "Caja renombrada", "network").unwrap();

        assert_eq!(again.first_seen, first.first_seen, "first_seen is not overwritten");
        assert_eq!(again.role.as_deref(), Some("receipt"), "the assigned role survives");
        assert_eq!(registry.get_all().len(), 1, "the entry is not duplicated");
    }

    // ── A role on a printer the registry does not hold is an ERROR, not a no-op ────────────────
    //
    // The printing settings screen offers the role control for a USB queue and sends
    // `usb:{queue}` as the key. If that queue never made it into the registry, answering `Ok` is a
    // click that does nothing, with no message — a silent failure.

    #[test]
    fn a_role_on_an_unregistered_usb_queue_is_refused_instead_of_silently_dropped() {
        let (registry, _) = temp_registry("rol-cola-usb");

        let err = registry
            .set_role("usb:Star_TSP143", "kitchen")
            .expect_err("a queue the registry cannot hold must not accept a role in silence");

        assert!(matches!(err, crate::PeripheralError::InvalidPrinterId(_)), "got {err:?}");
        assert!(registry.get_all().is_empty(), "nothing may have been written");
    }

    #[test]
    fn a_role_on_a_device_the_registry_does_not_know_is_an_error_too() {
        // Same rule for any unknown key: the only way to reach it is a registration that failed
        // upstream, and "your click did nothing" must not be how the user finds out.
        let (registry, _) = temp_registry("rol-desconocido");
        registry.register(None, "10.0.0.7", 9100, "Cocina", "network").unwrap();

        let err = registry
            .set_role("network:10.0.0.99:9100", "bar")
            .expect_err("an unknown key cannot take a role");

        assert!(matches!(err, crate::PeripheralError::InvalidPrinterId(_)), "got {err:?}");
        assert_eq!(registry.get_all().len(), 1, "the registered one is untouched");
        assert_eq!(registry.get_all()[0].role, None, "and it did not receive the role by mistake");
    }

    // ── An OS queue enters the registry with an identity of its OWN ─────────────────────────────
    //
    // A USB queue has neither a MAC to resolve by ARP nor an `ip:port` to fall back on, so
    // `device_key` answers `network::0` for EVERY one of them: the second queue registered would
    // overwrite the first and the owner would end up with one entry for two printers. Its
    // `printer_id` (`usb:{queue}`) IS its identity — a CUPS destination name is unique per machine.

    #[test]
    fn two_usb_queues_are_two_devices_and_not_one() {
        let (registry, _) = temp_registry("dos-colas-usb");

        let star = registry
            .register_queue("usb:Star_TSP143", "Star TSP143")
            .expect("a discovered queue enters the registry");
        let epson = registry
            .register_queue("usb:EPSON_TM", "EPSON TM-T20III")
            .expect("and so does the second one");

        assert_eq!(star.key, "usb:Star_TSP143", "the printer_id IS the identity");
        assert_eq!(epson.key, "usb:EPSON_TM");
        assert_eq!(star.kind, "usb", "the transport travels in its own field");
        assert_eq!(star.mac, None, "a cable has no MAC to invent");
        assert_eq!(
            registry.get_all().len(),
            2,
            "the second queue must not overwrite the first"
        );
    }

    #[test]
    fn a_registered_usb_queue_takes_a_role_like_any_other_printer() {
        // "This one is the kitchen's" has to work over a cable too.
        let (registry, _) = temp_registry("rol-cola-usb-registrada");
        registry
            .register_queue("usb:Star_TSP143", "Star TSP143")
            .unwrap();

        registry.set_role("usb:Star_TSP143", "kitchen").unwrap();

        assert_eq!(
            registry.get("usb:Star_TSP143").unwrap().role.as_deref(),
            Some("kitchen")
        );
    }

    #[test]
    fn re_registering_a_queue_keeps_its_role_and_its_first_seen() {
        // Every discovery re-registers what it finds. A queue that loses its role on the next scan
        // is a role the owner has to set again every time they open the screen.
        let (registry, _) = temp_registry("re-registra-cola-usb");
        let first = registry
            .register_queue("usb:Star_TSP143", "Star TSP143")
            .unwrap();
        registry.set_role("usb:Star_TSP143", "kitchen").unwrap();

        let again = registry
            .register_queue("usb:Star_TSP143", "Star TSP143 (barra)")
            .unwrap();

        assert_eq!(again.first_seen, first.first_seen, "first_seen is not overwritten");
        assert_eq!(again.role.as_deref(), Some("kitchen"), "the role survives");
        assert_eq!(again.name, "Star TSP143 (barra)", "the name does refresh");
        assert_eq!(registry.get_all().len(), 1, "the entry is not duplicated");
    }

    #[test]
    fn a_printer_id_that_is_not_a_usable_queue_is_refused() {
        // Anything without a queue name behind `usb:` would be the collapsing key all over again:
        // one ghost entry that every later queue inherits, roles included. Another transport's id
        // is refused at this door too — it has its own, with its own identity.
        let (registry, _) = temp_registry("cola-usb-invalida");

        for bad in [
            "",
            "   ",
            "usb:",
            "usb:   ",
            "network:10.0.0.5:9100",
            "bluetooth:AA:BB:CC:DD:EE:F1",
            "Star_TSP143",
        ] {
            let err = registry
                .register_queue(bad, "Star TSP143")
                .expect_err("must not enter the registry");
            assert!(
                matches!(err, crate::PeripheralError::InvalidPrinterId(_)),
                "{bad:?} got {err:?}"
            );
        }
        assert!(registry.get_all().is_empty(), "nothing may have been written");
    }

    #[test]
    fn a_device_with_neither_mac_nor_socket_is_refused_by_the_network_door() {
        // `register(None, "", 0, …)` is what registering a queue through the NETWORK door looks
        // like, and `device_key` answers `network::0` for every caller that does it. Refusing it
        // is the guard that keeps the collision from coming back through another route: a silent
        // overwrite of somebody else's role.
        let (registry, _) = temp_registry("clave-colapsada");

        let err = registry
            .register(None, "", 0, "Star TSP143", "usb")
            .expect_err("a device with no identity at all cannot be registered");

        assert!(
            matches!(err, crate::PeripheralError::InvalidPrinterId(_)),
            "got {err:?}"
        );
        assert!(registry.get_all().is_empty(), "nothing may have been written");
    }

    #[test]
    fn a_bluetooth_printer_still_registers_with_no_ip_at_all() {
        // The guard above must not catch the bonded-SPP path, which registers with an empty ip on
        // purpose: its MAC is its identity and the watchdog knows to leave it alone.
        let (registry, _) = temp_registry("bluetooth-sin-ip");

        let bt = registry
            .register(Some("AA:BB:CC:DD:EE:F1"), "", 0, "Star SM-L200", "bluetooth")
            .expect("a bonded printer has a MAC: that IS its identity");

        assert_eq!(bt.key, "AA:BB:CC:DD:EE:F1");
        assert_eq!(bt.kind, "bluetooth");
    }

    #[test]
    fn the_keys_already_on_disk_do_not_move_when_a_queue_is_added() {
        // `devices.json` is user data. An entry whose key changed would lose the role its owner
        // assigned — the migration of the old entries is that they DO NOT MOVE.
        let (_, path) = temp_registry("claves-existentes");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"devices":{
                 "AA:BB:CC:DD:EE:F5":{"key":"AA:BB:CC:DD:EE:F5","mac":"AA:BB:CC:DD:EE:F5",
                   "ip":"10.0.0.12","port":9100,"name":"Cocina","role":"kitchen","type":"network",
                   "first_seen":"2026-01-01T00:00:00","last_seen":"2026-01-01T00:00:00",
                   "status":"online"},
                 "network:10.0.0.13:9100":{"key":"network:10.0.0.13:9100","mac":null,
                   "ip":"10.0.0.13","port":9100,"name":"Barra","role":"bar","type":"network",
                   "first_seen":"2026-01-01T00:00:00","last_seen":"2026-01-01T00:00:00",
                   "status":"online"}}}"#,
        )
        .unwrap();

        let registry = DeviceRegistry::load(path);
        registry
            .register_queue("usb:Star_TSP143", "Star TSP143")
            .unwrap();

        let mac_keyed = registry.get("AA:BB:CC:DD:EE:F5").expect("still there");
        assert_eq!(mac_keyed.role.as_deref(), Some("kitchen"), "role untouched");
        let socket_keyed = registry.get("network:10.0.0.13:9100").expect("still there");
        assert_eq!(socket_keyed.role.as_deref(), Some("bar"), "role untouched");
        assert_eq!(registry.get_all().len(), 3, "the queue is an ADDITION");
    }

    #[test]
    fn loads_an_old_devices_json_without_a_key_field() {
        // `devices.json` is user data: files written before the `key` field existed are keyed by
        // MAC and carry no `key`. They must still load and keep working.
        let (_, path) = temp_registry("legacy");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"devices":{"AA:BB:CC:DD:EE:F4":{"mac":"AA:BB:CC:DD:EE:F4","ip":"10.0.0.11",
               "port":9100,"name":"Cocina","role":"kitchen","type":"network",
               "first_seen":"2026-01-01T00:00:00","last_seen":"2026-01-01T00:00:00",
               "status":"online"}}}"#,
        )
        .unwrap();

        let registry = DeviceRegistry::load(path);
        let device = registry.get("AA:BB:CC:DD:EE:F4").expect("the old device loads");

        assert_eq!(device.key, "AA:BB:CC:DD:EE:F4", "the key is filled in from the MAC");
        assert_eq!(device.role.as_deref(), Some("kitchen"), "the assigned role is not lost");
    }

    // ── A printer typed by address is remembered as such ─────────────────────────────────────────

    #[test]
    fn a_rescan_that_finds_a_typed_printer_keeps_it_manual_and_keeps_its_role() {
        let (registry, path) = temp_registry("manual-survives-rescan");
        registry.register_manual("10.9.9.9", 9100, "Network Printer (10.9.9.9)").expect("manual");
        registry.set_role("network:10.9.9.9:9100", "receipt").expect("role");

        registry
            .register(None, "10.9.9.9", 9100, "Network Printer (10.9.9.9)", "network")
            .expect("rescan");

        let device = registry.get("network:10.9.9.9:9100").expect("device");
        assert!(device.manual, "a scan that also sees it must not demote a typed printer");
        assert_eq!(device.role.as_deref(), Some("receipt"));
        let reloaded = DeviceRegistry::load(path);
        assert!(
            reloaded.get("network:10.9.9.9:9100").expect("persisted").manual,
            "the mark has to survive a restart of the app"
        );
    }

    #[test]
    fn a_scanned_printer_is_not_manual() {
        let (registry, _) = temp_registry("scanned-not-manual");
        let device = registry
            .register(None, "10.1.1.1", 9100, "Network Printer (10.1.1.1)", "network")
            .expect("scanned");
        assert!(!device.manual);
    }
}

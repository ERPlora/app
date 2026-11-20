//! MAC addresses: resolving the MAC of an IP through the system's ARP table, recognising one and
//! normalizing it.

use std::process::Command;

/// Resolves the MAC of an IP from the system's ARP table (a ping to fill the cache, then `arp`).
/// Never fails: it returns `None` on any error. Cross-platform.
pub fn get_mac_for_ip(ip: &str) -> Option<String> {
    let is_windows = std::env::consts::OS == "windows";

    // One ping to fill the ARP cache. A failure here is logged and the lookup goes on.
    let ping = if is_windows {
        Command::new("ping").args(["-n", "1", "-w", "1000", ip]).output()
    } else {
        Command::new("ping").args(["-c", "1", "-W", "1", ip]).output()
    };
    if let Err(exc) = ping {
        tracing::warn!("ping to {ip} failed: {exc}");
    }

    let arp = if is_windows {
        Command::new("arp").args(["-a", ip]).output()
    } else {
        Command::new("arp").args(["-n", ip]).output()
    };

    let output = match arp {
        Ok(o) => o,
        Err(exc) => {
            tracing::warn!("arp command for {ip} failed: {exc}");
            return None;
        }
    };

    if !output.status.success() {
        tracing::warn!(
            "ARP lookup for {ip} failed (rc={:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match find_mac(&stdout) {
        Some(mac) => Some(normalize_mac(&mac)),
        None => {
            tracing::warn!("no MAC in the ARP output for {ip}: {}", stdout.trim());
            None
        }
    }
}

/// Finds a MAC-shaped token (6 hex groups separated by `:` or `-`) in arbitrary text.
fn find_mac(text: &str) -> Option<String> {
    // `arp` separates its columns with whitespace; the MAC is a whole token.
    for token in text.split(|c: char| c.is_whitespace()) {
        if is_mac_token(token) {
            return Some(token.to_string());
        }
    }
    None
}

/// `true` if `token` is a well-formed MAC address: 6 groups of 1–2 hex digits, separated
/// consistently by `:` or `-`. Public face of [`is_mac_token`] for the `bluetooth:{mac}`
/// printer_id variant: the contract validates the MAC at the parsing edge, with the same rule the
/// registry itself uses to recognise one.
pub fn is_mac(token: &str) -> bool {
    is_mac_token(token)
}

fn is_mac_token(token: &str) -> bool {
    let sep = if token.contains(':') {
        ':'
    } else if token.contains('-') {
        '-'
    } else {
        return false;
    };

    let groups: Vec<&str> = token.split(sep).collect();
    if groups.len() != 6 {
        return false;
    }
    groups
        .iter()
        .all(|g| (1..=2).contains(&g.len()) && g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Normalizes a MAC to upper case, separated by `:`, with 2-digit octets.
pub fn normalize_mac(mac: &str) -> String {
    let unified = mac.replace(['-', '.'], ":").to_uppercase();
    unified
        .split(':')
        .map(|part| format!("{:0>2}", part))
        .collect::<Vec<_>>()
        .join(":")
}

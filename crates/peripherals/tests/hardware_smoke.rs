//! Smoke test contra hardware REAL. `#[ignore]` a propósito: exige una impresora encendida en la
//! LAN, así que no puede correr en CI ni en el gate — se lanza a mano cuando hay una delante.
//!
//! ```bash
//! cargo test -p erplora-peripherals --test hardware_smoke -- --ignored --nocapture
//! ```
//!
//! Existe porque el emulador de Android no puede dar **el descubrimiento**, y conviene ser exacto
//! sobre por qué — medido el 2026-08-09 contra una térmica real en `192.168.100.196:9100`:
//!
//!  - **Salir SÍ sale.** La NAT del emulador enruta la conexión TCP saliente por el host, así que
//!    desde `10.0.2.15` se llega a una IP de la LAN: `nc -w 3 -z 192.168.100.196 9100` → OK. O sea
//!    que **imprimir por IP a una impresora ya conocida funciona en el emulador**.
//!  - **Encontrarla NO.** El barrido /24 saca el prefijo de la IP local, que ahí es `10.0.2` — barre
//!    la red del emulador, no la de casa. Y el mDNS es multicast, que la NAT no cruza.
//!
//! Por eso el emulador vale para el camino de impresión pero no para el de descubrimiento, y la
//! única forma de verificar ESE es una impresora de verdad delante.
//!
//! Gasta papel: `smoke_imprime_pagina_de_prueba` solo imprime si se pide con
//! `ERPLORA_SMOKE_PRINT=1`, para poder buscar la impresora sin dejar tiques por el suelo.

use erplora_peripherals::discovery::{discover_printers, parse_printer_id, LocalNetworkAccess};
use erplora_peripherals::escpos::render_test_page;
use erplora_peripherals::queue::{PrintJob, PrintQueue, RetryPolicy};
use erplora_peripherals::registry::DeviceRegistry;

/// Registro en un temporal: el smoke test no debe pisar el `devices.json` real de la máquina.
/// `nombre` lo hace único por test, que comparten proceso.
fn registro_temporal(nombre: &str) -> DeviceRegistry {
    let dir = std::env::temp_dir().join(format!("erplora-smoke-{}-{nombre}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("crear temporal");
    DeviceRegistry::load(dir.join("devices.json"))
}

#[tokio::test]
#[ignore = "necesita una impresora de red encendida en la LAN"]
async fn smoke_descubre_la_impresora_de_la_lan() {
    let registry = registro_temporal("descubre");

    // Con una impresora delante el permiso está concedido por definición; si no lo estuviera, el
    // outcome lo diría en vez de devolver una lista vacía (hub#338).
    let encontradas = discover_printers(&registry, LocalNetworkAccess::Granted)
        .await
        .expect("el descubrimiento no debe fallar")
        .scanned_printers()
        .expect("con permiso concedido el escaneo SÍ corre")
        .to_vec();

    for p in &encontradas {
        println!("  · {} — {} [{}] {}mm", p.id, p.name, p.category, p.paper_width);
    }
    assert!(
        !encontradas.is_empty(),
        "ninguna impresora en la LAN. Ojo: cero NO es un error del código — un firewall, una VLAN \
         o la impresora apagada dan exactamente este resultado, sin excepción."
    );

    // El id ES el contrato con el frontend y con `devices.json`: si deja de ser parseable, los
    // roles (cocina/barra/caja) dejan de poder asignarse.
    for p in &encontradas {
        parse_printer_id(&p.id).unwrap_or_else(|e| panic!("id no parseable {}: {e}", p.id));
    }
}

#[tokio::test]
#[ignore = "necesita una impresora de red encendida en la LAN"]
async fn smoke_el_descubrimiento_registra_los_dispositivos() {
    // El bug que arregló `device_key`: si el descubrimiento no registra, `devices.json` queda
    // vacío, `erplora_get_devices` devuelve [] y NO se pueden asignar roles — con el módulo de
    // impresión inservible y sin un solo error que lo explique.
    let registry = registro_temporal("registra");

    let encontradas = discover_printers(&registry, LocalNetworkAccess::Granted)
        .await
        .expect("descubrimiento")
        .scanned_printers()
        .expect("con permiso concedido el escaneo SÍ corre")
        .to_vec();
    assert!(!encontradas.is_empty(), "sin impresora no se puede comprobar el registro");

    let registrados = registry.get_all();
    assert!(
        !registrados.is_empty(),
        "descubrió {} impresora(s) pero registró 0: los roles no se podrían asignar",
        encontradas.len()
    );
    for d in &registrados {
        println!("  · key={} mac={:?} ip={}", d.key, d.mac, d.ip);
        assert!(!d.key.is_empty(), "un dispositivo sin key no se puede direccionar");
    }
}

#[tokio::test]
#[ignore = "IMPRIME EN PAPEL: exige ERPLORA_SMOKE_PRINT=1"]
async fn smoke_imprime_pagina_de_prueba() {
    if std::env::var("ERPLORA_SMOKE_PRINT").ok().as_deref() != Some("1") {
        println!("saltado: exporta ERPLORA_SMOKE_PRINT=1 para gastar papel de verdad");
        return;
    }
    let registry = registro_temporal("imprime");
    let encontradas = discover_printers(&registry, LocalNetworkAccess::Granted)
        .await
        .expect("descubrimiento")
        .scanned_printers()
        .expect("con permiso concedido el escaneo SÍ corre")
        .to_vec();
    let destino = encontradas
        .iter()
        // Una A4 de oficina también escucha en el 9100 pero habla PCL/PostScript: mandarle ESC/POS
        // escupe folios de basura. Solo se imprime a lo que NO se ha identificado como A4.
        .find(|p| p.category != "a4")
        .expect("ninguna impresora térmica encontrada");

    println!("imprimiendo en {} ({})", destino.id, destino.name);
    let target = parse_printer_id(&destino.id).expect("id parseable");
    let bytes = render_test_page(&destino.id, &serde_json::json!({}));
    // Por la MISMA vía que la app instalable: `erplora_print` encola y el worker de `PrintQueue`
    // envía. Aquí no hace falta el worker — `send_once` es el envío que ese worker hace.
    PrintQueue::new(RetryPolicy::default())
        .send_once(&PrintJob {
            job_id: None,
            target,
            payload: bytes.clone(),
            attempts: 0,
        })
        .await
        .expect("la impresión debe llegar a la impresora");
    println!("enviados {} bytes de ESC/POS", bytes.len());
}

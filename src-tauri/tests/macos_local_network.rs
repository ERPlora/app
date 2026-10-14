//! Guardia del permiso de RED LOCAL de macOS.
//!
//! Desde **macOS 15 (Sequoia)** el sistema exige consentimiento explícito para hablar con la red
//! local, y la restricción se aplica **por debajo de la API**: alcanza a `URLSession`, al framework
//! Network y a los **BSD sockets** — es decir, también al `TcpStream::connect` de Rust contra el
//! puerto 9100 de una impresora. No es solo cosa de Bonjour/mDNS.
//!
//! Sin `NSLocalNetworkUsageDescription` en el `Info.plist` el sistema **no muestra el diálogo** y
//! las conexiones simplemente **fallan en silencio** (timeout, no error): el descubrimiento
//! devuelve cero impresoras y parece que no hay ninguna en la red. Por eso hay test: el síntoma no
//! se distingue de "el local no tiene impresoras".
//!
//! Tauri v2 fusiona en el bundle el `Info.plist` que esté junto a `tauri.conf.json`
//! (`tauri-build` lo declara como `rerun-if-changed`).

const INFO_PLIST: &str = include_str!("../Info.plist");

#[test]
fn declara_el_motivo_de_uso_de_la_red_local() {
    assert!(
        INFO_PLIST.contains("NSLocalNetworkUsageDescription"),
        "sin esta clave macOS 15+ NO pide permiso y los sockets a la impresora expiran en silencio"
    );
}

#[test]
fn el_motivo_no_esta_vacio() {
    // Apple rechaza en revisión los usage descriptions vacíos, y al usuario le aparecería un
    // diálogo sin explicación.
    let after_key = INFO_PLIST
        .split("NSLocalNetworkUsageDescription")
        .nth(1)
        .expect("la clave debe existir");
    let value = after_key
        .split("<string>")
        .nth(1)
        .and_then(|s| s.split("</string>").next())
        .expect("la clave debe ir seguida de un <string>");

    assert!(
        value.trim().len() > 20,
        "el motivo mostrado al usuario está vacío o es demasiado escueto: {value:?}"
    );
}

#[test]
fn declara_los_servicios_bonjour_que_se_navegan() {
    // `discovery::discover_mdns` navega EXACTAMENTE estos dos tipos. macOS solo deja resolver los
    // que estén declarados aquí: si falta uno, ese descubrimiento devuelve vacío sin avisar.
    for service in ["_pdl-datastream._tcp", "_ipp._tcp"] {
        assert!(
            INFO_PLIST.contains(service),
            "falta {service} en NSBonjourServices — mDNS no encontrará esas impresoras"
        );
    }
    assert!(INFO_PLIST.contains("NSBonjourServices"));
}

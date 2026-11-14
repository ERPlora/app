use super::*;

/// Once the shell takes the Android Back button (Tauri's `onBackButtonPress`),
/// Tauri no longer leaves the app on its own when the WebView has no history left: the shell
/// has to ask for it, and Tauri's own `exit` has no ACL permission to grant. `leave_app` is that
/// door. Four places have to agree or the press dies in silence: the build (which generates the
/// permission), the invoke handler, the default permission set the hub capability grants, and
/// the Kotlin command that actually leaves.
const PLUGIN_KT: &str = include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
const BUILD_RS: &str = include_str!("../build.rs");
const LIB_RS: &str = include_str!("lib.rs");
const DEFAULT_PERMISSIONS: &str = include_str!("../permissions/default.toml");

#[test]
fn leave_app_is_declared_wired_granted_and_implemented() {
    assert!(BUILD_RS.contains("\"leave_app\""), "build.rs does not declare leave_app: no permission is generated");
    // Only the code above the tests: this very assertion spells the handler too.
    let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
    assert!(
        production
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .is_some_and(|handler| handler.split(',').any(|c| c.trim() == "leave_app")),
        "leave_app is not wired to the invoke handler"
    );
    assert!(
        DEFAULT_PERMISSIONS.contains("\"allow-leave-app\""),
        "erplora-android:default does not grant allow-leave-app: the hub's capability would refuse it"
    );
    let kotlin = PLUGIN_KT.split("fun leaveApp(invoke: Invoke)").nth(1).expect("no Kotlin leaveApp command");
    let before = PLUGIN_KT.split("fun leaveApp(invoke: Invoke)").next().unwrap_or_default();
    assert!(before.trim_end().ends_with("@Command"), "Kotlin leaveApp is not a @Command");
    let body = kotlin.split("\n    }").next().unwrap_or_default();
    assert!(body.contains("moveTaskToBack(true)"), "leaveApp does not leave the way the system Back does");
    assert!(body.contains("invoke.resolve()"), "leaveApp never answers: the web would wait forever");
}

/// Once Android stops showing a permission dialog, the only way back is the app's
/// page in the device settings, and the shell offers a button that takes the owner there.
/// Four places have to agree or the tap dies in silence on a real device only: the build
/// (which generates the permission), the invoke handler, the default permission set the hub
/// capability grants, and the Kotlin command that opens the page.
const PLUGIN_KT_SOURCE: &str =
    include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
const BUILD_RS_SOURCE: &str = include_str!("../build.rs");
const LIB_RS_SOURCE: &str = include_str!("lib.rs");
const DEFAULT_PERMISSIONS_TOML: &str = include_str!("../permissions/default.toml");

#[test]
fn open_app_settings_is_declared_wired_granted_and_implemented() {
    assert!(
        BUILD_RS_SOURCE.contains("\"open_app_settings\""),
        "build.rs does not declare open_app_settings: no permission is generated"
    );
    // Only the code above the tests: this very assertion spells the command too.
    let production = LIB_RS_SOURCE.split("#[cfg(test)]").next().unwrap_or_default();
    let handler = production
        .split("generate_handler![")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("no invoke handler");
    assert!(
        handler.split(',').any(|c| c.trim() == "open_app_settings"),
        "open_app_settings is not wired to the invoke handler"
    );
    assert!(
        DEFAULT_PERMISSIONS_TOML.contains("\"allow-open-app-settings\""),
        "erplora-android:default does not grant allow-open-app-settings: the hub's capability \
         would refuse it"
    );
    let signature = "fun openAppSettings(invoke: Invoke)";
    let before = PLUGIN_KT_SOURCE.split(signature).next().unwrap_or_default();
    assert!(before.trim_end().ends_with("@Command"), "Kotlin openAppSettings is not a @Command");
    let body = PLUGIN_KT_SOURCE
        .split(signature)
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("no Kotlin openAppSettings command");
    assert!(
        body.contains("Settings.ACTION_APPLICATION_DETAILS_SETTINGS"),
        "openAppSettings does not open the app's own page in the device settings"
    );
    assert!(
        body.contains("Uri.fromParts(\"package\", activity.packageName, null)"),
        "openAppSettings does not point the settings at THIS app"
    );
    assert!(body.contains("invoke.resolve()"), "openAppSettings never answers: the web would wait forever");
    assert!(
        body.contains("invoke.reject("),
        "openAppSettings swallows a device with no settings page: the web must hear it to fall back"
    );
}

// ── Listening with the screen off ─────────────────────────────────────────────────────────
//
// The notices come out of the page (the bell's poll, the event socket), so they only arrive while
// Android keeps the app running. A foreground service is what keeps it: six places have to agree
// on it and none of them is seen by the compiler — the build (the permission), the handler, the
// default set the hub's capability grants, the Kotlin command, the service class and the manifest
// that declares it with the type and permissions Android 14 demands. Any one missing and the app
// behaves exactly as before, with every other test green.

/// The service and its decisions, read when the test runs: without the file the tests below say
/// so instead of the crate not building.
fn notice_listening_kt() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/android/src/main/java/com/erplora/android/NoticeListening.kt");
    std::fs::read_to_string(path).unwrap_or_else(|_| panic!("there is no NoticeListening.kt: nothing keeps the app listening"))
}

/// The plugin's Android library manifest, read as the merger reads it.
const PLUGIN_MANIFEST: &str = include_str!("../android/src/main/AndroidManifest.xml");

#[test]
fn keep_listening_is_declared_wired_granted_and_implemented() {
    assert!(BUILD_RS.contains("\"keep_listening\""), "build.rs does not declare keep_listening");
    let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
    let handler = production
        .split("generate_handler![")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("no invoke handler");
    assert!(
        handler.split(',').any(|c| c.trim() == "keep_listening"),
        "keep_listening is not wired to the invoke handler"
    );
    assert!(
        production.contains("run_mobile_plugin::<KeepListeningAnswer>(\"keepListening\", args)"),
        "Rust does not reach Kotlin under the name Kotlin answers to"
    );
    // On a computer the command answers `Ok` whatever it is handed, so no test can see what
    // reaches Kotlin: with `on` or one of the words dropped here, the tablet never listens (or
    // never stops) and every other test stays green.
    assert!(
        production.contains("keep_listening(KeepListeningArgs { on, title, body, channel })"),
        "the command does not hand Kotlin the order and the words exactly as the page sent them"
    );
    assert!(
        DEFAULT_PERMISSIONS.contains("\"allow-keep-listening\""),
        "erplora-android:default does not grant allow-keep-listening: the hub's capability would refuse it"
    );
    let signature = "fun keepListening(invoke: Invoke)";
    let before = PLUGIN_KT.split(signature).next().unwrap_or_default();
    assert!(before.trim_end().ends_with("@Command"), "Kotlin keepListening is not a @Command");
    let body = PLUGIN_KT
        .split(signature)
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("no Kotlin keepListening command");
    assert!(body.contains("NoticeListeningService.start("), "keepListening never starts the service");
    assert!(body.contains("NoticeListeningService.stop("), "keepListening never stops the service");
    assert!(body.contains("invoke.resolve()"), "keepListening never answers: the web would wait forever");
    assert!(body.contains("invoke.reject("), "keepListening swallows a refusal the page must hear");
}

#[test]
fn the_listening_request_crosses_to_kotlin_under_the_keys_kotlin_reads() {
    let on = serde_json::to_value(KeepListeningArgs {
        on: true,
        title: Some("t".into()),
        body: Some("b".into()),
        channel: Some("c".into()),
    })
    .expect("serializes");
    assert_eq!(on, serde_json::json!({ "on": true, "title": "t", "body": "b", "channel": "c" }));
    let off = serde_json::to_value(KeepListeningArgs { on: false, title: None, body: None, channel: None })
        .expect("serializes");
    assert_eq!(off, serde_json::json!({ "on": false }));
    for key in ["\"on\"", "\"title\"", "\"body\"", "\"channel\""] {
        assert!(PLUGIN_KT.contains(key), "Kotlin does not read {key} from the request");
    }
}

#[test]
fn the_app_is_not_kept_listening_once_its_page_is_gone() {
    // The service keeps the PROCESS; the page is what listens. With the activity destroyed or the
    // app swiped away, a notification saying «listening» would be a promise nothing keeps.
    let on_destroy = PLUGIN_KT
        .split("override fun onDestroy(activity: AppCompatActivity)")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("the plugin does not stop anything when the activity is destroyed");
    assert!(on_destroy.contains("NoticeListeningService.stop("), "onDestroy leaves the service running");
    let notice_listening_kt = notice_listening_kt();
    let task_removed = notice_listening_kt
        .split("override fun onTaskRemoved(")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("the service does not react to the app being swiped away");
    assert!(task_removed.contains("stopSelf()"), "swiping the app away leaves the service running");
    assert!(
        notice_listening_kt.contains("return NoticeListening.START_MODE"),
        "onStartCommand does not answer with the pinned start mode"
    );
}

#[test]
fn the_page_is_kept_running_exactly_while_the_app_listens() {
    // The service keeps the process, `PageKeeper` keeps the page: without it Chromium freezes
    // the hidden page at 60 s and no notice is born, with the service and its notification in
    // place (measured on the device). What it does has its own test in Kotlin
    // (`PageKeeperTest`); what only lives in the plugin is WHEN it is told — installed with the
    // WebView, on once the service started, off with every way the listening ends.
    let kotlin_fn = |signature: &str| {
        PLUGIN_KT
            .split(signature)
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .unwrap_or_else(|| panic!("the plugin has no `{signature}`"))
    };
    let load = kotlin_fn("override fun load(webView: WebView)");
    assert!(load.contains("PageKeeper(webView)"), "no keeper is built for the WebView");
    assert!(load.contains("keeper.install(root, owner)"), "the keeper never hears the app leave the screen");
    assert!(load.contains("pageKeeper = keeper"), "the commands cannot reach the keeper");

    let command = kotlin_fn("fun keepListening(invoke: Invoke)");
    let (off, on) = command
        .split_once("NoticeListening.textsOf(")
        .expect("keepListening does not read the words of the notification");
    assert!(off.contains("pageKeeper?.setListening(false)"), "told to stop, the page is still kept running");
    let started = on
        .split_once("NoticeListeningService.start(")
        .map(|(_, after)| after)
        .and_then(|after| after.split("catch").next())
        .expect("keepListening never starts the service");
    assert!(
        started.contains("pageKeeper?.setListening(true)"),
        "the service starts and the page still freezes: the keeper is never told to listen"
    );
    assert!(
        !on.split("NoticeListeningService.start(").next().unwrap_or_default().contains("setListening(true)"),
        "the page is kept running before the service is known to have started"
    );

    let on_destroy = kotlin_fn("override fun onDestroy(activity: AppCompatActivity)");
    assert!(
        on_destroy.contains("pageKeeper?.setListening(false)"),
        "the activity is gone and its page is still kept running"
    );
}

#[test]
fn the_listening_service_is_declared_where_no_generator_can_drop_it() {
    let manifest = without_comments(PLUGIN_MANIFEST);
    let declared = declared_permissions(PLUGIN_MANIFEST);
    for permission in [
        "android.permission.FOREGROUND_SERVICE",
        "android.permission.FOREGROUND_SERVICE_SPECIAL_USE",
    ] {
        assert!(declared.iter().any(|p| p == permission), "{permission} is not declared: startForeground throws");
    }
    let service = manifest
        .split("<service")
        .nth(1)
        .and_then(|rest| rest.split("</service>").next())
        .expect("no <service> in the plugin manifest");
    assert!(
        service.contains("android:name=\"com.erplora.android.NoticeListeningService\""),
        "the declared service is not NoticeListeningService"
    );
    assert!(
        service.contains("android:foregroundServiceType=\"specialUse\""),
        "the service does not declare the type Android 14 demands"
    );
    assert!(service.contains("android:exported=\"false\""), "another app could start or stop the service");
    assert!(
        service.contains("android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE"),
        "a specialUse service without its subtype is refused by Google Play's review"
    );
}

/// wry hands the microphone and the position to WHATEVER page asks. The
/// origin check is `WebPermissionGate.kt` (and its JUnit tests), and it only bites if the plugin
/// puts it in front of wry's client: from `load` (which Tauri calls from wry's
/// `on_webview_created`, after `setWebChromeClient`), through `post`, so it finds the client
/// whatever that order becomes.
const WEB_PERMISSION_GATE_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/WebPermissionGate.kt");

/// The Rust half of the app, which decides «one of ours» for the navigation and the hub link.
const SHELL_LIB_RS: &str = include_str!("../../../src-tauri/src/lib.rs");

#[test]
fn the_webview_permissions_are_guarded_by_origin_from_load() {
    let load = PLUGIN_KT
        .split("override fun load(webView: WebView)")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("no Kotlin load(webView)");
    assert!(
        load.contains("webView.post { WebPermissionGate.install(webView) }"),
        "the plugin never puts the origin check in front of wry's WebChromeClient: once granted, \
         any page the window loads gets the microphone and the position"
    );
    // One domain on both halves: a hub the Rust side trusts must keep its microphone.
    assert!(SHELL_LIB_RS.contains("const HUB_DOMAIN_SUFFIX: &str = \".erplora.com\";"));
    assert!(WEB_PERMISSION_GATE_KT.contains("const val DOMAIN = \"erplora.com\""));
    // And the install only reads wry's client where the WebView can hand it back:
    // `WebViewCompat.getWebChromeClient` THROWS on API 24-25 with the factory WebView, and from
    // `post` that is a crash at start (`clientFor` and its JUnit test decide; this is the wiring).
    let install = WEB_PERMISSION_GATE_KT
        .split("fun install(webView: WebView)")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("no Kotlin install(webView)");
    assert!(
        install.contains("val canRead = WebViewFeature.isFeatureSupported(WebViewFeature.GET_WEB_CHROME_CLIENT)")
            && install.contains("clientFor(canRead)"),
        "install reads the WebChromeClient without asking whether this WebView can hand it back: \
         on Android 7 with its factory WebView that throws inside `post` and kills the app at start"
    );
}

#[test]
fn the_status_serializes_as_a_map_of_permission_to_bool() {
    // The contract the page reads: `{ "android.permission.X": true }`.
    let mut estado = PermissionStatus::new();
    estado.insert("android.permission.POST_NOTIFICATIONS".into(), true);
    let json = serde_json::to_string(&estado).unwrap();
    assert!(json.contains("\"android.permission.POST_NOTIFICATIONS\":true"));
}

/// The Kotlin policy is the only place that can actually ASK for these permissions, and the
/// map it returns is keyed by the raw string. If the two sides ever spelled one differently,
/// Rust would read `None` for a permission Android had denied and the till would go back to
/// reporting zero printers with a straight face.
const PERMISSION_POLICY_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/PermissionPolicy.kt");

#[test]
fn rust_and_kotlin_spell_the_permissions_the_same_way() {
    for permission in [ACCESS_LOCAL_NETWORK, POST_NOTIFICATIONS, BLUETOOTH_CONNECT] {
        assert!(
            PERMISSION_POLICY_KT.contains(permission),
            "{permission} is not in PermissionPolicy.kt — the status map would never mention it"
        );
    }
}

/// Every `android.permission.*` the Kotlin policy names, read out of its source.
///
/// Read rather than listed so the guards below cannot drift from the one place that decides
/// what the shell ASKS for: a permission added there and forgotten everywhere else is the bug
/// this file is about.
fn permissions_the_plugin_can_ask_for() -> Vec<&'static str> {
    PERMISSION_POLICY_KT
        .match_indices("\"android.permission.")
        .filter_map(|(at, _)| PERMISSION_POLICY_KT[at + 1..].split('"').next())
        .collect()
}

/// Everything outside XML comments. Commenting a tag out is the easiest way to "temporarily"
/// drop a permission, and the edit nobody remembers to undo.
fn without_comments(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("-->") else { return out };
        rest = &rest[start + end + "-->".len()..];
    }
    out.push_str(rest);
    out
}

/// Permissions actually DECLARED by a manifest, read out of its live `<uses-permission>` tags.
///
/// Not a text search: manifests here carry long comments naming the permissions and explaining
/// why they are there, so `contains()` would keep passing on the prose after the tag itself was
/// deleted — a guard that goes green on the wreckage is worse than no guard.
fn declared_permissions(manifest: &str) -> Vec<String> {
    without_comments(manifest)
        .split("<uses-permission")
        .skip(1)
        .filter_map(|tag| {
            tag.split("android:name=\"")
                .nth(1)
                .and_then(|value| value.split('"').next())
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn a_commented_out_permission_does_not_count_as_declared() {
    // How a permission usually disappears: someone comments it out "for a minute". The prose
    // around it still names it, so a text search would call it declared and let the guard
    // below wave the regression through.
    let manifest = r#"<manifest>
            <uses-permission android:name="android.permission.INTERNET" />
            <!-- <uses-permission android:name="android.permission.ACCESS_LOCAL_NETWORK" /> -->
        </manifest>"#;

    assert_eq!(declared_permissions(manifest), ["android.permission.INTERNET"]);
}

/// Asking for a permission the merged manifest never declared is not a dialog the user can
/// say yes to: Android answers DENIED at once, forever, and shows nothing. The till would
/// then tell the user to grant local network access in the system settings and send
/// them looking for a toggle that does not exist.
///
/// The plugin declares them in its OWN Android library manifest, which the manifest merger
/// folds into the app at build time. That is what makes the declaration survive
/// `cargo tauri android init` — the generator rewrites `src-tauri/gen/android`, it does not
/// reach into `crates/`.
#[test]
fn the_plugin_declares_every_permission_it_can_ask_for() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/android/src/main/AndroidManifest.xml");
    let manifest = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "{path}: {e}\n\
             The plugin ships no Android manifest, so these permissions exist only in the \
             GENERATED gen/android project. Lose that file and `cargo tauri android init` \
             writes Tauri's template back — INTERNET and nothing else — and the till stops \
             finding printers with a perfectly green build."
        )
    });

    let declared = declared_permissions(&manifest);
    let asked_for = permissions_the_plugin_can_ask_for();
    assert!(!asked_for.is_empty(), "PermissionPolicy.kt names no permission at all");
    for permission in asked_for {
        assert!(
            declared.iter().any(|d| d == permission),
            "PermissionPolicy asks for {permission}, but no manifest of this plugin declares \
             it: Android would answer that request DENIED without ever showing a dialog.\n\
             Declared right now: {declared:?}"
        );
    }
}

/// The scope of a permission request has to reach Kotlin under the key Kotlin reads.
/// If either side renamed it, the argument would silently vanish and the plugin
/// would fall back to requesting its whole batch — the exact out-of-context notifications
/// dialog this contract exists to end, with no error anywhere.
#[test]
fn the_permission_scope_travels_under_the_key_kotlin_reads() {
    let args = RequestPermissionsArgs {
        permissions: Some(vec![ACCESS_LOCAL_NETWORK.to_string()]),
    };
    let json = serde_json::to_value(&args).expect("serializable");
    assert_eq!(json["permissions"][0], ACCESS_LOCAL_NETWORK);

    const PLUGIN_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
    assert!(
        PLUGIN_KT.contains("optJSONArray(\"permissions\")"),
        "ErploraAndroidPlugin.kt no longer reads the \"permissions\" scope: every request \
         would go back to asking for the whole batch"
    );
}

/// An absent scope must serialize as an ABSENT key, not as `null`: Kotlin's `optJSONArray`
/// treats both as "no scope", but the wire contract is "an old web sends nothing", and
/// pinning it keeps the fallback path honest.
#[test]
fn no_scope_still_serializes_and_means_the_whole_batch() {
    let args = RequestPermissionsArgs { permissions: None };
    let json = serde_json::to_value(&args).expect("serializable");
    assert!(json["permissions"].is_null());
}

// ── Bluetooth Classic SPP ────────────────────────────────────────────────────────────────
//
// The transport lives in Kotlin (`BluetoothSpp.kt`); what Rust owns is the CONTRACT across
// the JNI boundary — command names, argument keys, the shape of the answer, and the refusal
// code. Every one of those is a bare literal on both sides, so each gets a drift guard: a
// typo in either one is invisible until a real till stops finding its bluetooth printer.

const ERPLORA_ANDROID_PLUGIN_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");

#[test]
fn the_print_payload_crosses_to_kotlin_under_the_keys_kotlin_reads() {
    // ESC/POS init (`ESC @`) — two bytes, so the base64 is checkable by eye.
    let args = BluetoothPrintArgs::new("AA:BB:CC:DD:EE:FF", &[0x1b, 0x40]);
    let json = serde_json::to_value(&args).expect("serializable");
    assert_eq!(json["mac"], "AA:BB:CC:DD:EE:FF");
    assert_eq!(json["payloadBase64"], "G0A=");

    for key in ["getString(\"mac\"", "getString(\"payloadBase64\""] {
        assert!(
            ERPLORA_ANDROID_PLUGIN_KT.contains(key),
            "ErploraAndroidPlugin.kt no longer reads {key}) — the print job would cross the \
             boundary and vanish"
        );
    }
}

#[test]
fn a_bonded_printer_list_deserializes_from_what_kotlin_sends() {
    let list: BluetoothPrinterList = serde_json::from_value(serde_json::json!({
        "printers": [
            { "id": "bluetooth:AA:BB:CC:DD:EE:FF", "name": "Kitchen", "mac": "AA:BB:CC:DD:EE:FF" }
        ]
    }))
    .expect("the wire shape Kotlin builds");
    assert_eq!(list.printers.len(), 1);
    assert_eq!(list.printers[0].id, "bluetooth:AA:BB:CC:DD:EE:FF");
    assert_eq!(list.printers[0].mac, "AA:BB:CC:DD:EE:FF");
}

#[test]
fn rust_and_kotlin_spell_the_bluetooth_refusal_and_commands_the_same_way() {
    for literal in [BLUETOOTH_PERMISSION_DENIED, "bluetoothBondedPrinters", "bluetoothPrint"] {
        assert!(
            ERPLORA_ANDROID_PLUGIN_KT.contains(literal),
            "{literal} is not in ErploraAndroidPlugin.kt — the mobile call would answer \
             `command not found` (or the refusal would lose its name) on a real device only"
        );
    }
}

#[test]
fn desktop_stays_network_only_by_construction() {
    // Only Android produces or accepts `bluetooth:{mac}`. On desktop the bonded list is
    // honestly empty and a bluetooth print is an ERROR, never a silent no-op: a resolved print
    // that put no paper out would tell the till the ticket was printed.
    let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
    assert_eq!(
        desktop
            .bluetooth_bonded_printers()
            .expect("an empty list, not an error")
            .len(),
        0
    );
    assert!(desktop.bluetooth_print("AA:BB:CC:DD:EE:FF", &[0x1b]).is_err());
}

// ── Reading a badge off the device's own NFC reader ──────────────────────────────────────
//
// Same shape as the bluetooth block above: the radio lives in Kotlin (`NfcBadge.kt`), and what
// Rust owns is the CONTRACT across the JNI boundary — the command name, the argument key, the
// shape of the answer and the three refusal codes. All bare literals on both sides.

const NFC_BADGE_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/NfcBadge.kt");

#[test]
fn the_read_timeout_crosses_to_kotlin_under_the_key_kotlin_reads() {
    let args = NfcReadArgs { timeout_ms: 15_000 };
    let json = serde_json::to_value(&args).expect("serializable");
    assert_eq!(json["timeoutMs"], 15_000);
    assert!(
        ERPLORA_ANDROID_PLUGIN_KT.contains("\"timeoutMs\""),
        "ErploraAndroidPlugin.kt no longer reads timeoutMs — every read would silently take \
         the default, and a screen that asked for a short poll would hold the radio open"
    );
}

#[test]
fn a_tapped_card_deserializes_from_what_kotlin_sends() {
    let read: NfcBadgeRead =
        serde_json::from_value(serde_json::json!({ "badge": "04A23B5C6D7E80" }))
            .expect("the wire shape Kotlin builds");
    assert_eq!(read.badge.as_deref(), Some("04A23B5C6D7E80"));
}

#[test]
fn nothing_tapped_is_an_answer_and_not_a_failure() {
    // Kotlin resolves with the key ABSENT when the window closed with no card on it. That is
    // the ordinary outcome of every poll — the shell loops on it — so it must not arrive as an
    // error, or a till waiting for a badge would log a failure every fifteen seconds.
    let read: NfcBadgeRead =
        serde_json::from_value(serde_json::json!({})).expect("an empty answer is valid");
    assert!(read.badge.is_none());
}

#[test]
fn rust_and_kotlin_spell_the_nfc_refusals_and_the_command_the_same_way() {
    for literal in [NFC_UNAVAILABLE, NFC_DISABLED, NFC_RANDOM_UID] {
        assert!(
            NFC_BADGE_KT.contains(literal),
            "{literal} is not in NfcBadge.kt — the refusal would reach the page under a name \
             nothing recognises, and the user would be told the wrong thing to do"
        );
    }
    assert!(
        ERPLORA_ANDROID_PLUGIN_KT.contains("nfcRead"),
        "ErploraAndroidPlugin.kt has no nfcRead command — the mobile call would answer \
         `command not found`, on a real device only"
    );
}

#[test]
fn each_nfc_refusal_is_classified_as_itself() {
    // Three refusals because there are three different things to do about it: buy a reader,
    // switch NFC on, use another card. Collapsing them into one would send the user to the
    // settings screen for a tablet that has no reader to enable.
    assert!(matches!(
        classify_nfc_error(&format!("[{NFC_UNAVAILABLE}] - no reader on this device")),
        Error::NfcUnavailable
    ));
    assert!(matches!(
        classify_nfc_error(&format!("[{NFC_DISABLED}] - NFC is off")),
        Error::NfcDisabled
    ));
    assert!(matches!(
        classify_nfc_error(&format!("[{NFC_RANDOM_UID}] - this card randomises its id")),
        Error::NfcRandomUid
    ));
}

#[test]
fn any_other_nfc_failure_keeps_its_own_words() {
    for message in ["java.lang.SecurityException", "activity is not resumed", ""] {
        assert!(
            matches!(classify_nfc_error(message), Error::PluginInvoke(_)),
            "{message:?} is not one of the three refusals"
        );
    }
}

#[test]
fn the_nfc_refusals_reach_the_page_under_the_names_the_shell_reads() {
    // The shell branches on these words: «tap the card» only appears where there IS a reader,
    // and «switch NFC on» only where switching it on would help.
    for (error, word) in [
        (Error::NfcUnavailable, "nfc_unavailable"),
        (Error::NfcDisabled, "nfc_disabled"),
        (Error::NfcRandomUid, "nfc_random_uid"),
    ] {
        assert_eq!(serde_json::to_string(&error).unwrap(), format!("\"{word}\""));
    }
}

#[test]
fn a_platform_without_nfc_refuses_cleanly_instead_of_waiting() {
    // Desktop has no reader mode. The answer has to be the refusal `nfc_unavailable` and not a
    // resolved `None`: the shell POLLS this command, and a resolved nothing would spin a
    // fifteen-second loop forever on a machine that can never produce a card.
    let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
    assert!(matches!(
        desktop.nfc_read(NFC_DEFAULT_TIMEOUT_MS),
        Err(Error::NfcUnavailable)
    ));
}

/// NFC is an install-time (`normal`) permission, so it must NOT join the runtime policy.
///
/// Putting it there would be worse than useless: `requestPermissionForAliases` on a normal
/// permission shows no dialog, and the batch would grow an entry that can never be denied and
/// can never be granted by the user — noise in the very map the shell reads to decide what it
/// may do.
#[test]
fn nfc_is_not_a_runtime_permission() {
    assert!(
        !PERMISSION_POLICY_KT.contains("android.permission.NFC"),
        "NFC joined the runtime permission policy; it is a `normal` permission and no dialog \
         will ever be shown for it"
    );
}

#[test]
fn the_error_reaches_the_frontend_as_plain_text() {
    // The same as the rest of the shell: the promise is rejected with a readable message.
    let e = Error::PluginInvoke("sin actividad".into());
    assert_eq!(serde_json::to_string(&e).unwrap(), "\"sin actividad\"");
}

// ── Publishing a file into the public Downloads collection ───────────────────────────────────

const DOWNLOAD_PUBLISHER_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/DownloadPublisher.kt");

/// The one refusal the user can act on has to survive the trip back from Kotlin.
///
/// Kotlin rejects with a **code**, Tauri renders the rejection as `[code] - message`, and by
/// the time it reaches Rust it is a plain string. If that string is not recognised the shell
/// reports a generic failure and the page prints *«could not save»* instead of *«open your
/// business in a browser»* — the sentence that tells the user what to do instead.
#[test]
fn an_android_too_old_to_publish_comes_back_as_downloads_unreachable() {
    let rejected = format!("[{DOWNLOADS_UNREACHABLE}] - Android 9 has no Downloads collection");
    assert!(matches!(
        classify_publish_error(&rejected),
        Error::DownloadsUnreachable
    ));
}

#[test]
fn every_other_failure_keeps_its_own_words() {
    // A full disk, a revoked provider, a Kotlin exception: none of them is "this device cannot
    // save files", and dressing them up as that would send the user to a browser for a problem
    // a browser does not fix.
    for message in [
        "java.io.IOException: No space left on device",
        "insert into MediaStore returned no row",
        "",
    ] {
        assert!(
            matches!(classify_publish_error(message), Error::PluginInvoke(_)),
            "{message:?} is not the phone refusal"
        );
    }
}

#[test]
fn the_refusal_reaches_the_page_under_the_name_the_shell_reads() {
    // `src-tauri` maps this variant onto `ShellError::DownloadsUnreachable`, which serializes
    // to the same word the web checks for (`save-download.ts`). One spelling, end to end.
    assert_eq!(
        serde_json::to_string(&Error::DownloadsUnreachable).unwrap(),
        "\"downloads_unreachable\""
    );
}

#[test]
fn rust_and_kotlin_spell_the_refusal_the_same_way() {
    // Same trap as the permission strings above: the code is a bare literal on both sides, so
    // a typo in either one is invisible until a real Android 9 till says the wrong sentence.
    assert!(
        DOWNLOAD_PUBLISHER_KT.contains(DOWNLOADS_UNREACHABLE),
        "DownloadPublisher.kt never rejects with {DOWNLOADS_UNREACHABLE}, so Rust would \
         classify an old Android as a generic failure"
    );
}

/// Kotlin source with its comments taken out.
///
/// `DownloadPublisher` EXPLAINS why it does not use `WRITE_EXTERNAL_STORAGE`, and a plain
/// `contains` cannot tell an explanation from a call. Same reason as [`without_comments`] one
/// screen up: a guard has to read the code, not the documentation about the code.
fn without_kotlin_comments(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    loop {
        let block = rest.find("/*");
        let line = rest.find("//");
        let (start, close, skip) = match (block, line) {
            (Some(b), Some(l)) if b < l => (b, "*/", 2),
            (Some(b), None) => (b, "*/", 2),
            (_, Some(l)) => (l, "\n", 0),
            (None, None) => break,
        };
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(close) else {
            break;
        };
        rest = &rest[start + end + skip..];
    }
    out.push_str(rest);
    out
}

#[test]
fn publishing_needs_no_permission_the_plugin_would_have_to_ask_for() {
    // Why `MediaStore` and not `getExternalStoragePublicDirectory`: inserting into the public
    // Downloads collection needs NO permission from API 29 on. If this ever grows a
    // `WRITE_EXTERNAL_STORAGE`, it also grows a runtime dialog the save command has to wait
    // for — a different design, not a line to slip in.
    let code = without_kotlin_comments(DOWNLOAD_PUBLISHER_KT);
    assert!(
        !code.contains("WRITE_EXTERNAL_STORAGE"),
        "publishing now wants a runtime permission; the save path has to ask for it first"
    );
}

#[test]
fn the_comment_stripper_keeps_the_code_and_drops_the_prose() {
    let source = "// asks for WRITE_EXTERNAL_STORAGE one day\nval a = 1\n/* WRITE_EXTERNAL_STORAGE */\nval b = 2";
    let code = without_kotlin_comments(source);
    assert!(!code.contains("WRITE_EXTERNAL_STORAGE"), "{code:?}");
    assert!(
        code.contains("val a = 1") && code.contains("val b = 2"),
        "{code:?}"
    );
}

// ── The A4 document through Android's own print service ──────────────────────────────────
//
// The WebView that prints lives in Kotlin (`HtmlPrinter.kt`); Rust owns the command name and
// the argument key, both bare literals on the two sides.

const HTML_PRINTER_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/HtmlPrinter.kt");

// ── The question before a link links another hub ───────────────────────────────────────

fn question_in(language: &str) -> Wording {
    Wording {
        title: format!("title-{language}"),
        message: format!("message-{language}"),
        open: format!("open-{language}"),
        cancel: format!("cancel-{language}"),
    }
}

#[test]
fn the_hub_question_crosses_to_kotlin_in_both_languages_under_the_keys_kotlin_reads() {
    let (en, es) = (question_in("en"), question_in("es"));
    let json = serde_json::to_value(AskToOpenHubArgs { en: &en, es: &es }).expect("serializable");
    for language in ["en", "es"] {
        for key in ["title", "message", "open", "cancel"] {
            assert_eq!(json[language][key], format!("{key}-{language}"), "{language}.{key}");
        }
    }
    let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
    let mut literals = vec![
        format!("fun {ASK_TO_OPEN_HUB_COMMAND}(invoke: Invoke)"),
        "put(\"open\"".to_string(),
    ];
    literals.extend(["title", "message", "open", "cancel"].map(|key| format!("getString(\"{key}\"")));
    for literal in literals {
        assert!(
            code.contains(&literal),
            "{literal} is not in ErploraAndroidPlugin.kt — a link to another hub would never be asked \
             about on a real device"
        );
    }
    let languages = without_kotlin_comments(HUB_QUESTION_KT);
    for literal in ["\"es\"", "\"en\""] {
        assert!(languages.contains(literal), "{literal} is not in HubQuestion.kt");
    }
}

const HUB_QUESTION_KT: &str =
    include_str!("../android/src/main/java/com/erplora/android/HubQuestion.kt");

#[test]
fn only_an_explicit_open_opens() {
    let read = |answer| serde_json::from_value::<AskToOpenHubAnswer>(answer).map(|a| a.open);
    assert_eq!(read(serde_json::json!({ "open": true })).ok(), Some(true));
    assert_eq!(read(serde_json::json!({ "open": false })).ok(), Some(false));
    assert_eq!(read(serde_json::json!({})).ok(), Some(false), "an answer without `open` linked the hub");
}

#[test]
fn no_page_can_show_or_answer_the_hub_question() {
    // Only Rust asks: a command in `build.rs` gets an `allow-*` permission a capability could
    // grant, and the page is exactly who must not be able to answer.
    let build = include_str!("../build.rs");
    let permissions = include_str!("../permissions/default.toml");
    for name in ["ask_to_open_hub", "ask-to-open-hub", ASK_TO_OPEN_HUB_COMMAND] {
        assert!(!build.contains(name), "build.rs declares {name}");
        assert!(!permissions.contains(name), "the default permissions grant {name}");
    }
}

#[test]
fn the_a4_document_crosses_to_kotlin_under_the_key_kotlin_reads() {
    let json = serde_json::to_value(PrintHtmlArgs {
        html: "<p>F-1</p>".into(),
    })
    .expect("serializable");
    assert_eq!(json["html"], "<p>F-1</p>");

    let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
    for literal in [
        "getString(\"html\"".to_string(),
        format!("fun {PRINT_HTML_COMMAND}(invoke: Invoke)"),
    ] {
        assert!(
            code.contains(&literal),
            "{literal} is not in ErploraAndroidPlugin.kt — the invoice would reach Kotlin as \
             `command not found` or with no document, on a real device only"
        );
    }
}

#[test]
fn an_answer_with_no_data_is_a_print_screen_that_opened() {
    // `invoke.resolve()` with no data arrives as `null` (and a `JSObject()` as `{}`): both
    // mean the print screen opened. Reading either as an error sent the invoice to the till
    // roll right behind the dialog the user was looking at.
    for answer in [serde_json::Value::Null, serde_json::json!({})] {
        assert!(
            serde_json::from_value::<PrintHtmlAnswer>(answer.clone()).is_ok(),
            "{answer} must read as success"
        );
    }
}

#[test]
fn the_printed_document_runs_no_code() {
    // The same frontier as the desktop print window's CSP: the html comes from a
    // remote page, so the WebView that renders it runs no script and exposes no bridge.
    let code = without_kotlin_comments(HTML_PRINTER_KT);
    assert!(code.contains("javaScriptEnabled = false"), "{code}");
    assert!(!code.contains("javaScriptEnabled = true"), "{code}");
    assert!(!code.contains("addJavascriptInterface"), "{code}");
}

#[test]
fn desktop_refuses_to_print_through_the_android_plugin() {
    // Unreachable by construction (the shell only routes here on Android), and a refusal
    // anyway: a resolved call would read as a dialog that opened.
    let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
    assert!(desktop.print_html("<p>F-1</p>").is_err());
}

// ── Kotlin answers with no data ─────────────────────────────────────────────────────────
//
// `invoke.resolve()` with no data reaches Rust as JSON `null`. Read as a struct, that `null`
// is a deserialize error, so a call that DID its job came back as a failure: the ticket was
// on paper and the print host reported it `failed`.

#[test]
fn a_bluetooth_answer_with_no_data_is_a_ticket_on_paper() {
    for answer in [serde_json::Value::Null, serde_json::json!({})] {
        assert!(
            serde_json::from_value::<BluetoothPrintAnswer>(answer.clone()).is_ok(),
            "{answer} must read as a printed ticket"
        );
    }
}

/// The Rust source outside this test module.
fn production_rust() -> &'static str {
    const LIB_RS: &str = include_str!("lib.rs");
    LIB_RS
        .split("#[cfg(test)]\nmod tests")
        .next()
        .expect("lib.rs has code before its tests")
}

/// Commands whose Kotlin side resolves with a bare `invoke.resolve()` somewhere.
fn kotlin_commands_resolving_with_no_data(kotlin: &str) -> Vec<String> {
    let code = without_kotlin_comments(kotlin);
    let mut names = Vec::new();
    let mut rest = code.as_str();
    while let Some(at) = rest.find("(invoke: Invoke)") {
        let name = rest[..at]
            .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or_default()
            .to_string();
        rest = &rest[at + "(invoke: Invoke)".len()..];
        let body_end = rest.find("(invoke: Invoke)").unwrap_or(rest.len());
        if rest[..body_end].contains("invoke.resolve()") {
            names.push(name);
        }
    }
    names
}

/// The type each `run_mobile_plugin::<T>(command, …)` reads its answer as, by command name.
fn rust_answer_types(rust: &str) -> Vec<(String, String)> {
    const CALL: &str = "run_mobile_plugin::<";
    let mut found = Vec::new();
    let mut rest = rust;
    while let Some(at) = rest.find(CALL) {
        rest = &rest[at + CALL.len()..];
        let Some(close) = rest.find(">(") else { break };
        let ty = rest[..close].trim().to_string();
        let arg = rest[close + 2..].trim_start();
        let command = if let Some(quoted) = arg.strip_prefix('"') {
            quoted.split('"').next().unwrap_or_default().to_string()
        } else {
            let ident: String = arg
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let decl = format!("const {ident}: &str = \"");
            rust.split(&decl)
                .nth(1)
                .and_then(|v| v.split('"').next())
                .unwrap_or_default()
                .to_string()
        };
        found.push((command, ty));
    }
    found
}

#[test]
fn every_command_kotlin_resolves_with_no_data_is_read_whatever_its_shape() {
    let rust = production_rust();
    let empty_answers = kotlin_commands_resolving_with_no_data(ERPLORA_ANDROID_PLUGIN_KT);
    // Positive control: the scan sees the two commands known to answer with nothing.
    for known in ["bluetoothPrint", "printHtml"] {
        assert!(
            empty_answers.iter().any(|n| n == known),
            "{known} not found by the scan: {empty_answers:?}"
        );
    }
    let answers = rust_answer_types(rust);
    for command in &empty_answers {
        let ty = answers
            .iter()
            .find(|(c, _)| c == command)
            .map(|(_, t)| t.as_str())
            .unwrap_or_else(|| panic!("no run_mobile_plugin::<T> call for {command}"));
        let reads_anything = ty == "serde::de::IgnoredAny"
            || rust.contains(&format!("type {ty} = serde::de::IgnoredAny;"));
        assert!(
            reads_anything,
            "{command} answers `null` from Kotlin but Rust reads it as {ty}: a call that did \
             its job would come back as a failure"
        );
    }
}

// ── The tap that started the app ─────────────────────────────────────────────────────────────
//
// The notification plugin reports it while it is still loading, before the page listens, and the
// event is lost. Kotlin keeps it and the shell asks for it once — three places that have to agree
// on one command name and one answer shape, none of which the compiler sees.

#[test]
fn the_kept_launch_tap_reads_with_and_without_a_tap() {
    let kept: NoticeTapAnswer =
        serde_json::from_str(r#"{"tap":{"id":7,"notification":"{\"extra\":{\"path\":\"/m/kds\"}}"}}"#)
            .expect("a kept tap reads");
    assert_eq!(
        kept.tap,
        Some(LaunchNoticeTap { id: 7, notification: Some(r#"{"extra":{"path":"/m/kds"}}"#.into()), path: None })
    );
    let none: NoticeTapAnswer = serde_json::from_str(r#"{"tap":null}"#).expect("no tap reads");
    assert_eq!(none.tap, None);
    let bare: NoticeTapAnswer = serde_json::from_str("{}").expect("an empty answer reads");
    assert_eq!(bare.tap, None);
}

#[test]
fn kotlin_keeps_the_launch_tap_and_hands_it_over_once() {
    const NOTICE_TAPS_KT: &str = include_str!("../android/src/main/java/com/erplora/android/NoticeTaps.kt");
    let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
    assert!(
        production.contains("run_mobile_plugin::<NoticeTapAnswer>(\"takeNoticeTap\", Empty {})"),
        "Rust no longer asks Kotlin for the kept tap by the name Kotlin answers to"
    );
    let before = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").next().unwrap_or_default();
    assert!(before.trim_end().ends_with("@Command"), "Kotlin takeNoticeTap is not a @Command");
    let command = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").nth(1).expect("no Kotlin takeNoticeTap");
    let body = command.split("\n    }").next().unwrap_or_default();
    assert!(body.contains("NoticeTaps.take()"), "takeNoticeTap does not hand over the tap the box kept");
    let load = PLUGIN_KT.split("override fun load(webView: WebView)").nth(1).expect("Kotlin does not look at the launch");
    let load = load.split("\n    }").next().unwrap_or_default();
    assert!(
        load.contains("NoticeTaps.pageLoading(activity, activity.intent)"),
        "load does not hand the box the intent that started the app"
    );
    // The memory has to outlive the process: back through the icon after the system killed it,
    // the task's intent is the old tap again.
    assert!(
        NOTICE_TAPS_KT.contains("NoticeTapBox(PreferencesMemory(context.applicationContext))"),
        "the box no longer remembers the kept tap outside the process"
    );
    assert!(
        NOTICE_TAPS_KT.contains("prefs.edit().putString(LAST_KEPT, key).apply()"),
        "the kept tap is not written to the app's preferences"
    );
}

// ── A tap comes back without its notice ─────────────────────────────────────────────────────
//
// The notification plugin never stores the notice on the intent of its tap (`sourceJson` is
// never set, tauri-apps/plugins-workspace#3550): its `actionPerformed` says neither the number
// nor the screen, and the tap Kotlin kept had no screen either — the page stayed where it was.
// The app writes each notice's screen down as it goes out, hands it back with the tap, and tells
// the page a tap waits while it listens: names and keys the compiler does not see.

#[test]
fn the_screen_of_a_notice_crosses_to_kotlin_under_the_keys_kotlin_reads() {
    assert_eq!(
        serde_json::to_value(RememberNoticeArgs { id: 8, path: Some("/kitchen/display") }).expect("serializable"),
        serde_json::json!({ "id": 8, "path": "/kitchen/display" })
    );
    assert_eq!(
        serde_json::to_value(RememberNoticeArgs { id: 9, path: None }).expect("serializable"),
        serde_json::json!({ "id": 9, "path": null })
    );
    let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
    let command = format!("fun {REMEMBER_NOTICE_COMMAND}(invoke: Invoke)");
    let before = code.split(&command).next().unwrap_or_default();
    assert!(before.trim_end().ends_with("@Command"), "Kotlin {REMEMBER_NOTICE_COMMAND} is not a @Command");
    let body = code.split(&command).nth(1).expect("no Kotlin rememberNotice");
    let body = body.split("\n    }").next().unwrap_or_default();
    for literal in ["optInt(\"id\"", "getString(\"path\"", "NoticeTaps.remember(activity, id, path)"] {
        assert!(
            body.contains(literal),
            "{literal} is not in Kotlin rememberNotice — a tap on a notice would come back without its screen"
        );
    }
    assert!(
        production_rust().contains("run_mobile_plugin::<RememberNoticeAnswer>(REMEMBER_NOTICE_COMMAND,"),
        "Rust no longer hands Kotlin the screen of a notice by the name Kotlin answers to"
    );
}

#[test]
fn the_kept_tap_brings_back_the_screen_the_app_wrote_down() {
    let kept: NoticeTapAnswer =
        serde_json::from_str(r#"{"tap":{"id":8,"notification":null,"path":"/kitchen/display"}}"#)
            .expect("a kept tap reads");
    assert_eq!(
        kept.tap,
        Some(LaunchNoticeTap { id: 8, notification: None, path: Some("/kitchen/display".into()) })
    );
    let command = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").nth(1).expect("no Kotlin takeNoticeTap");
    let body = command.split("\n    }").next().unwrap_or_default();
    assert!(
        body.contains("kept.put(\"path\", tap.path)"),
        "takeNoticeTap does not hand back the screen the app wrote down"
    );
}

#[test]
fn kotlin_tells_rust_when_a_tap_waits_for_the_page() {
    const NOTICE_TAPS_KT: &str = include_str!("../android/src/main/java/com/erplora/android/NoticeTaps.kt");
    let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
    let command = format!("fun {WATCH_NOTICE_TAPS_COMMAND}(invoke: Invoke)");
    let before = code.split(&command).next().unwrap_or_default();
    assert!(before.trim_end().ends_with("@Command"), "Kotlin {WATCH_NOTICE_TAPS_COMMAND} is not a @Command");
    let body = code.split(&command).nth(1).expect("no Kotlin watchNoticeTaps");
    let body = body.split("\n    }").next().unwrap_or_default();
    for literal in ["parseArgs(WatchNoticeTapsArgs::class.java)", "NoticeTaps.onTapWaiting(activity)", "handler.send(JSObject())"] {
        assert!(body.contains(literal), "{literal} is not in Kotlin watchNoticeTaps — the page would never hear a tap with the app open");
    }
    assert!(code.contains("lateinit var handler: Channel"), "the channel does not cross to Kotlin under `handler`");
    let taps = without_kotlin_comments(NOTICE_TAPS_KT);
    for literal in [
        "NoticeTargets(PreferencesTargets(context.applicationContext))",
        "{ targets(context).pathOf(it) }",
        "box(context).onTapWaiting(listener)",
        "targets(context).remember(id, path)",
    ] {
        assert!(taps.contains(literal), "{literal} is not in NoticeTaps.kt");
    }
    let production = production_rust();
    assert!(
        production.contains("run_mobile_plugin::<WatchNoticeTapsAnswer>(") && production.contains("WATCH_NOTICE_TAPS_COMMAND,"),
        "Rust no longer hands Kotlin the channel by the name Kotlin answers to"
    );
    assert!(production.contains("handler: Channel::new("), "Rust hands Kotlin no channel to call back on");
}

#[test]
fn no_page_can_write_down_a_notice_nor_watch_the_taps() {
    // Only Rust calls them: a command in `build.rs` gets an `allow-*` permission a capability
    // could grant, and a page that wrote down screens could send a tap anywhere.
    let build = include_str!("../build.rs");
    let permissions = include_str!("../permissions/default.toml");
    for name in [
        "remember_notice",
        "remember-notice",
        REMEMBER_NOTICE_COMMAND,
        "watch_notice_taps",
        "watch-notice-taps",
        WATCH_NOTICE_TAPS_COMMAND,
    ] {
        assert!(!build.contains(name), "build.rs declares {name}");
        assert!(!permissions.contains(name), "the default permissions grant {name}");
    }
}

// ── The system voice ───────────────────────────────────────────────────────────────────────

/// The Android WebView has no `speechSynthesis`, so the assistant's «Read aloud» reads through
/// these three commands in the app. Five places have to agree or the assistant stays mute on a
/// real device only: the build (which generates the permissions), the invoke handler, the
/// default set the hub capability grants, the name Rust calls Kotlin by and the Kotlin command.
const SYSTEM_VOICE_KT: &str = include_str!("../android/src/main/java/com/erplora/android/SystemVoice.kt");

#[test]
fn the_system_voice_commands_are_declared_wired_granted_and_implemented() {
    let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
    let handler = production
        .split("generate_handler![")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("no invoke handler");
    for (command, kotlin, permission) in [
        ("tts_available", "ttsAvailable", "allow-tts-available"),
        ("tts_speak", "ttsSpeak", "allow-tts-speak"),
        ("tts_stop", "ttsStop", "allow-tts-stop"),
    ] {
        assert!(BUILD_RS.contains(&format!("\"{command}\"")), "build.rs does not declare {command}");
        assert!(
            handler.split(',').any(|c| c.trim() == command),
            "{command} is not wired to the invoke handler"
        );
        assert!(
            DEFAULT_PERMISSIONS.contains(&format!("\"{permission}\"")),
            "erplora-android:default does not grant {permission}: the hub's capability would refuse it"
        );
        assert!(
            production.contains(&format!("(\"{kotlin}\"")),
            "Rust does not reach Kotlin under the name Kotlin answers to ({kotlin})"
        );
        let signature = format!("fun {kotlin}(invoke: Invoke)");
        let before = PLUGIN_KT.split(&signature).next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin {kotlin} is not a @Command");
        let body = PLUGIN_KT
            .split(&signature)
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .unwrap_or_else(|| panic!("no Kotlin {kotlin} command"));
        assert!(body.contains("invoke.resolve("), "{kotlin} never answers: the page would wait forever");
        assert!(body.contains("systemVoice."), "{kotlin} does not go through the system voice");
    }
}

#[test]
fn the_speech_request_crosses_to_kotlin_under_the_keys_kotlin_reads() {
    let speak = serde_json::to_value(TtsSpeakArgs { text: "Hola".into(), language: "es-ES".into() })
        .expect("serializes");
    assert_eq!(speak, serde_json::json!({ "text": "Hola", "language": "es-ES" }));
    let available = serde_json::to_value(TtsLanguageArgs { language: "en-US".into() }).expect("serializes");
    assert_eq!(available, serde_json::json!({ "language": "en-US" }));
    for key in ["getString(\"text\"", "getString(\"language\""] {
        assert!(PLUGIN_KT.contains(key), "Kotlin does not read {key}) from the request");
    }
}

#[test]
fn the_availability_reads_from_what_kotlin_sends_and_reaches_the_page_as_it() {
    let read: TtsAvailability = serde_json::from_str(r#"{"available":true}"#).expect("reads");
    assert!(read.available);
    let none: TtsAvailability = serde_json::from_str("{}").expect("an empty answer reads");
    assert!(!none.available, "an answer without the flag is no voice, never a voice");
    assert_eq!(
        serde_json::to_value(TtsAvailability { available: true }).expect("serializes"),
        serde_json::json!({ "available": true })
    );
    assert!(PLUGIN_KT.contains("put(\"available\""), "Kotlin does not answer under the key Rust reads");
}

#[test]
fn rust_and_kotlin_spell_the_voice_refusals_the_same_way() {
    for literal in [TTS_UNAVAILABLE, TTS_FAILED] {
        assert!(
            SYSTEM_VOICE_KT.contains(&format!("\"{literal}\"")),
            "{literal} is not in SystemVoice.kt: the page could not tell «no voice» from «it broke»"
        );
    }
}

#[test]
fn each_voice_refusal_is_classified_as_itself() {
    // `tts_unavailable` sends the page to the browser's voice; `tts_failed` is a reading that broke.
    assert!(matches!(
        classify_tts_error(&format!("[{TTS_UNAVAILABLE}] - no voice for the language")),
        Error::TtsUnavailable
    ));
    assert!(matches!(classify_tts_error(&format!("[{TTS_FAILED}] - the engine broke")), Error::TtsFailed));
    for message in ["java.lang.IllegalStateException", "the webview is unreachable", ""] {
        assert!(
            matches!(classify_tts_error(message), Error::PluginInvoke(_)),
            "{message:?} is not one of the two refusals"
        );
    }
}

#[test]
fn the_voice_refusals_reach_the_page_under_the_names_the_page_reads() {
    for (error, word) in [(Error::TtsUnavailable, "tts_unavailable"), (Error::TtsFailed, "tts_failed")] {
        assert_eq!(serde_json::to_string(&error).unwrap(), format!("\"{word}\""));
    }
}

#[test]
fn a_computer_has_no_system_voice_and_says_so() {
    // Windows (WebView2) and the other desktops read with the page's own `speechSynthesis`:
    // «not available» sends the page there, and reading here refuses instead of pretending.
    let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
    assert!(!desktop.tts_available(TtsLanguageArgs { language: "es-ES".into() }).expect("answers"));
    assert!(matches!(
        desktop.tts_speak(TtsSpeakArgs { text: "Hola".into(), language: "es-ES".into() }),
        Err(Error::TtsUnavailable)
    ));
    assert!(desktop.tts_stop().is_ok());
}

#[test]
fn the_reading_waits_off_the_async_workers() {
    // `ttsSpeak` answers when the WHOLE text has been read: blocking a worker of the async runtime
    // that long would stall every other command of the app while the assistant speaks.
    let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
    for command in ["async fn tts_speak<", "async fn tts_available<"] {
        let body = production
            .split(command)
            .nth(1)
            .and_then(|rest| rest.split("\n}").next())
            .unwrap_or_else(|| panic!("no {command}"));
        assert!(body.contains("spawn_blocking("), "{command} blocks an async worker while it waits");
    }
}

#[test]
fn the_system_voice_is_shut_down_with_the_activity() {
    let on_destroy = PLUGIN_KT
        .split("override fun onDestroy(activity: AppCompatActivity)")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("no Kotlin onDestroy");
    assert!(
        on_destroy.contains("systemVoice.shutdown()"),
        "the engine outlives its activity: its service stays bound and a reading waits forever"
    );
}

#[test]
fn the_app_can_see_the_system_voice_engine() {
    // From Android 11 (API 30) an app only sees the services it declares it looks for: without
    // this query `TextToSpeech` finds no engine, its init fails and the assistant stays mute,
    // with every test green.
    let manifest = without_comments(PLUGIN_MANIFEST);
    let queries = manifest.split("<queries>").nth(1).and_then(|rest| rest.split("</queries>").next());
    assert!(
        queries.is_some_and(|q| q.contains("android:name=\"android.intent.action.TTS_SERVICE\"")),
        "the plugin manifest does not query android.intent.action.TTS_SERVICE"
    );
}

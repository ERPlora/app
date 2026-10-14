package com.erplora.android

import android.app.Activity
import android.app.AlertDialog
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.view.ViewGroup
import android.webkit.WebView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.util.Locale

/**
 * Permisos de runtime del TPV en Android.
 *
 * Existe porque **declararlos en el manifest no basta**, y su ausencia no da ningún error:
 *
 *  - sin `ACCESS_LOCAL_NETWORK` (API 37+) el barrido del puerto 9100 son 254 **timeouts**, así que
 *    el descubrimiento devuelve una lista vacía y parece que el local no tiene impresoras;
 *  - sin `POST_NOTIFICATIONS` (API 33+) la notificación no aparece, y la comanda entra en cocina
 *    sin que nadie se entere.
 *
 * Los dos fallan **en silencio**, que es lo peor que puede pasarle a un TPV: el usuario no ve un
 * error, ve un producto que «no funciona». Verificado en el emulador API 37 antes de existir este
 * plugin — `discover_printers` devolvía `[]` sin la menor queja hasta conceder el permiso con `adb`.
 *
 * Es además el vehículo por el que el shell puede llevar código Kotlin a Android: aquí irá lo que
 * Rust no alcanza (`NsdManager`, quiosco, Bluetooth SPP si algún día vuelve).
 *
 * La clase es pegamento: toda decisión vive en las funciones puras del companion, probadas en la
 * JVM sin emulador.
 */
@TauriPlugin(
    // El ALIAS ES el string del permiso, a propósito: lo que devuelve `check_permissions` se lee
    // igual que lo que dice `dumpsys`, sin una segunda nomenclatura que mantener.
    //
    // Declararlos aquí NO es decorativo: `requestPermissionForAliases` resuelve el alias contra
    // esta anotación y, si no lo encuentra, **no hace nada** — ni diálogo ni excepción, y el
    // `invoke` queda colgado para siempre. Hay un test que lo verifica contra `PermissionPolicy`.
    permissions = [
        Permission(strings = [PermissionPolicy.POST_NOTIFICATIONS], alias = PermissionPolicy.POST_NOTIFICATIONS),
        Permission(strings = [PermissionPolicy.ACCESS_LOCAL_NETWORK], alias = PermissionPolicy.ACCESS_LOCAL_NETWORK),
        Permission(strings = [PermissionPolicy.BLUETOOTH_CONNECT], alias = PermissionPolicy.BLUETOOTH_CONNECT),
    ]
)
class ErploraAndroidPlugin(private val activity: Activity) : Plugin(activity) {

    /** Keeps the page running while listening for notices (hub#2307); `null` until the WebView loads. */
    private var pageKeeper: PageKeeper? = null

    /**
     * hub#2360 — a new page is loading: the tap that STARTED the app waits for it. The notification
     * plugin reports that tap as `actionPerformed` from its own `load`, before the page listens, and
     * Tauri drops an event nobody is listening to: the tap opened the app on its first screen
     * instead of the notice's. A tap that reaches a dead process through `onNewIntent` is kept by
     * `MainActivity` into the same box.
     *
     * hub#2307 — the same WebView gets its `PageKeeper`, which the listening commands switch on/off.
     */
    override fun load(webView: WebView) {
        super.load(webView)
        NoticeTaps.pageLoading(activity, activity.intent)
        val keeper = PageKeeper(webView)
        val owner = activity as? AppCompatActivity
        val root = activity.window?.decorView as? ViewGroup
        if (owner != null && root != null) {
            keeper.install(root, owner)
            pageKeeper = keeper
        }
    }

    /** `take_notice_tap` (hub#2360) — hands the kept tap over, once: `{ tap: {id, notification} | null }`. */
    @Command
    fun takeNoticeTap(invoke: Invoke) {
        val tap = NoticeTaps.take()
        val answer = JSObject()
        if (tap == null) {
            answer.put("tap", null as Any?)
        } else {
            val kept = JSObject()
            kept.put("id", tap.id)
            kept.put("notification", tap.notification)
            answer.put("tap", kept)
        }
        invoke.resolve(answer)
    }

    /**
     * `leave_app` (hub#1906) — the app goes to the background, which is what the system Back does
     * on a root screen since Android 12 (the task moves back; nothing is finished or killed).
     *
     * Needed because the shell now HOLDS the Back button through Tauri's `onBackButtonPress`: with a
     * listener registered, Tauri no longer leaves on its own when the WebView has no history, and
     * its own `exit` command has no permission a capability could grant.
     */
    @Command
    fun leaveApp(invoke: Invoke) {
        invoke.resolve()
        activity.moveTaskToBack(true)
    }

    /**
     * `open_app_settings` (hub#1886) — opens ERPlora's own page in the device settings.
     *
     * Refused twice, Android stops showing a permission dialog for the life of the install, and
     * from then on that page is the only place to turn it back on. Sending the owner to look for it
     * among ten differently organised settings apps is what this replaces. A device with no such
     * page (a locked-down kiosk build) REJECTS, so the shell can fall back to saying where to go.
     */
    @Command
    fun openAppSettings(invoke: Invoke) {
        val intent = Intent(
            Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
            Uri.fromParts("package", activity.packageName, null),
        )
        try {
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: ActivityNotFoundException) {
            invoke.reject("app_settings_unavailable")
        }
    }

    /**
     * `askToOpenHub` (hub#2644) — «Open <hub> on this device?» before a link links another hub.
     *
     * Only Rust calls it (`ErploraAndroid::ask_to_open_hub`): it is in neither `build.rs` nor the
     * default permissions, so no page can show it. The words come in both languages and the
     * device's is picked here ([HubQuestion]). Answers `{ open: true }` for «Open» only: «Cancel»,
     * Back, a tap outside or a dialog that goes away for any other reason are a no. Words missing,
     * REJECTS: the app reads a failure as a no.
     */
    @Command
    fun askToOpenHub(invoke: Invoke) {
        val all = invoke.getArgs()
        val language = all.getJSObject(HubQuestion.languageOf(Locale.getDefault().language))
        val words = HubQuestion.wordsOf(
            language?.getString("title", null),
            language?.getString("message", null),
            language?.getString("open", null),
            language?.getString("cancel", null),
        )
        if (words == null) {
            invoke.reject("ask_to_open_hub needs the words of the question")
            return
        }
        activity.runOnUiThread {
            var answered = false
            fun answer(open: Boolean) {
                if (answered) return
                answered = true
                invoke.resolve(JSObject().put("open", open))
            }
            AlertDialog.Builder(activity)
                .setTitle(words.title)
                .setMessage(words.message)
                .setPositiveButton(words.open) { _, _ -> answer(true) }
                .setNegativeButton(words.cancel) { _, _ -> answer(false) }
                .setOnDismissListener { answer(false) }
                .show()
        }
    }

    /**
     * `keep_listening` (hub#2307) — keeps the app running with the screen off so the notices born in
     * the page still arrive (`on: true`, with the words of the ongoing notification), or lets Android
     * reclaim it again (`on: false`). See [NoticeListening].
     *
     * Android refuses to start a foreground service from the background (12+); the page only asks
     * while somebody is using it, and a refusal is REJECTED so the page can log it — the notices keep
     * working with the app on screen, as before.
     */
    @Command
    fun keepListening(invoke: Invoke) {
        val args = invoke.getArgs()
        if (!args.optBoolean("on", false)) {
            pageKeeper?.setListening(false)
            NoticeListeningService.stop(activity)
            invoke.resolve()
            return
        }
        val texts = NoticeListening.textsOf(
            args.getString("title", null),
            args.getString("body", null),
            args.getString("channel", null),
        )
        if (texts == null) {
            invoke.reject(NoticeListening.TEXT_MISSING)
            return
        }
        try {
            NoticeListeningService.start(activity, texts)
            pageKeeper?.setListening(true)
            invoke.resolve()
        } catch (e: IllegalStateException) {
            // `ForegroundServiceStartNotAllowedException` (API 31+) is one of these.
            invoke.reject(NoticeListening.START_REFUSED)
        } catch (e: SecurityException) {
            invoke.reject(NoticeListening.START_REFUSED)
        }
    }

    /**
     * The page is what listens; with its activity gone, the service would only keep a notification
     * saying «listening» over nothing (hub#2307). A configuration change recreates the page, which
     * asks again on its boot.
     */
    override fun onDestroy(activity: AppCompatActivity) {
        pageKeeper?.setListening(false)
        NoticeListeningService.stop(activity)
    }

    /**
     * `check_permissions` — qué hay concedido AHORA, sin molestar al usuario.
     *
     * `@PermissionCallback` no es opcional ni heredado: es también el callback que cierra el
     * `request_permissions` de abajo cuando el usuario responde.
     */
    @Command
    @PermissionCallback
    override fun checkPermissions(invoke: Invoke) {
        invoke.resolve(estadoActual())
    }

    /**
     * `request_permissions` — asks for what is missing, scoped to the OPERATION that asks
     * (hub#758).
     *
     * The caller names the permissions it is about to use in `args.permissions`; only those are
     * requested. Unscoped, the plugin asked for its whole batch, so tapping «Re-scan» popped the
     * local-network dialog and then — with no visible relation to anything — the notifications
     * one: an opportunistic-looking request the user rightly denies. A call WITHOUT the argument
     * (a web older than the scope) still gets the whole batch: the two halves ship separately and
     * an old web must not break.
     *
     * Idempotent: if everything in scope is already granted it resolves at once, with no dialog.
     * That matters because the PWA calls this before every discovery.
     *
     * The request goes through [requestPermissionForAliases] and **not**
     * `activity.requestPermissions`: the user's answer is collected by Tauri's Activity, which
     * only knows how to hand it back to the plugin if the request left through its own plumbing.
     * Asked by hand, the system grants the permissions all the same but the `invoke` never
     * resolves and the PWA waits forever.
     *
     * The callback is `checkPermissions`, so the answer carries the REAL state of every
     * permission: the user may grant one and deny another. And a «no» RESOLVES, it does not
     * reject — without a printer the till has to keep selling.
     */
    @Command
    override fun requestPermissions(invoke: Invoke) {
        val scope = requestScope(PermissionPolicy.required(), requestedPermissions(invoke))
        val missing = pendingOf(scope, concedidos())
        if (missing.isEmpty()) {
            invoke.resolve(estadoActual())
            return
        }
        requestPermissionForAliases(missing.toTypedArray(), invoke, "checkPermissions")
    }

    /** The scope the caller sent, or `null` when it sent none (a web older than hub#758). */
    private fun requestedPermissions(invoke: Invoke): List<String>? {
        val requested = invoke.getArgs().optJSONArray("permissions") ?: return null
        return (0 until requested.length()).mapNotNull { requested.optString(it, null) }
    }

    /**
     * `save_to_downloads` — puts a file the shell already wrote where the USER can find it
     * (hub#499).
     *
     * The only way a business gets its own data off the tablet it keeps it on: the backup, an
     * invoice PDF, a document out of `/files`. Until this existed the shell refused on Android,
     * because the folder Tauri calls Downloads is app-scoped storage that Android 11 closed to
     * every file manager — a file written there exists and cannot be reached (hub#480, ADR-0259).
     *
     * The bytes arrive as a PATH, not as a payload: the shell has already staged them in its own
     * cache, so a multi-megabyte export crosses the JNI boundary once instead of being copied onto
     * the tablet's heap a second time.
     *
     * Both failures are answered, and they are not the same sentence. An Android too old to have
     * the collection is rejected with [DownloadPublisher.DOWNLOADS_UNREACHABLE], which the page
     * turns into *«open your business in a browser»* — the refusal the user can act on. Everything
     * else keeps its own words, because "this device cannot save files" would be the wrong advice
     * for a full disk.
     */
    @Command
    fun saveToDownloads(invoke: Invoke) {
        val args = invoke.getArgs()
        val sourcePath = args.getString("sourcePath", null)
        val name = args.getString("name", null)
        if (sourcePath.isNullOrBlank() || name.isNullOrBlank()) {
            invoke.reject("save_to_downloads needs both sourcePath and name")
            return
        }

        if (!DownloadPublisher.canPublish()) {
            invoke.reject(
                "Android ${Build.VERSION.SDK_INT} has no public Downloads collection",
                DownloadPublisher.DOWNLOADS_UNREACHABLE,
            )
            return
        }

        try {
            val location = DownloadPublisher.publish(
                activity.contentResolver,
                File(sourcePath),
                name,
            )
            invoke.resolve(JSObject().put("location", location))
        } catch (e: Exception) {
            // Resolved-with-nothing would read as a save that happened. The page has to be able to
            // say the file did NOT arrive — that is the whole lesson of hub#475.
            invoke.reject(e.message ?: e.toString(), e)
        }
    }

    /**
     * `print_html` — the system print screen with an A4 document, printer or «Save as PDF»
     * (hub#2008). The rendering and the `PrintManager` call are [HtmlPrinter]'s.
     *
     * Resolves once the print screen has been asked for — the same moment the desktop answers,
     * because what the user does in it (print, save, cancel) is the system's. Anything that stops
     * it from opening REJECTS: a resolve there would read as a document handed over (hub#475).
     */
    @Command
    fun printHtml(invoke: Invoke) {
        val html = invoke.getArgs().getString("html", null)
        if (html.isNullOrBlank()) {
            invoke.reject("print_html needs the html of the document")
            return
        }
        HtmlPrinter.print(
            activity,
            html,
            onOpened = { invoke.resolve() },
            onFailed = { e -> invoke.reject(e.message ?: e.toString(), e) },
        )
    }

    /**
     * `bluetooth_bonded_printers` — the bonded devices that look like printers (ADR-0204).
     *
     * The BONDED list, never a scan: pairing belongs to Android's own settings (its UI, its PIN
     * handling), and reading the list needs only `BLUETOOTH_CONNECT` — no `BLUETOOTH_SCAN`
     * dialog. Adapter absent or off answers an EMPTY list, not an error: a venue with no
     * bluetooth is a normal venue. A SecurityException (permission denied) rejects with
     * [BLUETOOTH_PERMISSION_DENIED], because "no permission" and "no printers" send the user in
     * opposite directions (the hub#338 lesson).
     */
    @Command
    fun bluetoothBondedPrinters(invoke: Invoke) {
        val printers = org.json.JSONArray()
        try {
            val manager =
                activity.getSystemService(android.content.Context.BLUETOOTH_SERVICE)
                    as? android.bluetooth.BluetoothManager
            val adapter = manager?.adapter
            @Suppress("MissingPermission")
            if (adapter != null && adapter.isEnabled) {
                @Suppress("MissingPermission")
                for (device in adapter.bondedDevices ?: emptySet()) {
                    @Suppress("MissingPermission")
                    val name = device.name
                    val majorClass = device.bluetoothClass?.majorDeviceClass ?: 0
                    if (!BluetoothSpp.looksLikePrinter(name, majorClass)) continue
                    val mac = device.address.uppercase()
                    printers.put(
                        JSObject()
                            .put("id", BluetoothSpp.printerId(mac))
                            .put("name", BluetoothSpp.displayName(name, mac))
                            .put("mac", mac),
                    )
                }
            }
        } catch (e: SecurityException) {
            invoke.reject("bluetooth permission denied: ${e.message}", BLUETOOTH_PERMISSION_DENIED)
            return
        }
        invoke.resolve(JSObject().put("printers", printers))
    }

    /**
     * `bluetooth_print` — sends already-rendered ESC/POS bytes to a bonded printer over RFCOMM
     * (ADR-0204). Transport only: the rendering happened in Rust, the payload arrives base64.
     *
     * The socket work runs off the main thread — RFCOMM connect blocks for seconds — and the
     * outcome is reported honestly: a ticket that did not come out REJECTS (hub#475), so the
     * print host reports `failed` instead of swallowing the job.
     */
    @Command
    fun bluetoothPrint(invoke: Invoke) {
        val args = invoke.getArgs()
        val mac = args.getString("mac", null)
        val payloadBase64 = args.getString("payloadBase64", null)
        if (mac.isNullOrBlank() || payloadBase64.isNullOrBlank()) {
            invoke.reject("bluetooth_print needs both mac and payloadBase64")
            return
        }
        val payload =
            try {
                android.util.Base64.decode(payloadBase64, android.util.Base64.DEFAULT)
            } catch (e: IllegalArgumentException) {
                invoke.reject("payloadBase64 is not base64: ${e.message}")
                return
            }

        val manager =
            activity.getSystemService(android.content.Context.BLUETOOTH_SERVICE)
                as? android.bluetooth.BluetoothManager
        val adapter = manager?.adapter
        @Suppress("MissingPermission")
        if (adapter == null || !adapter.isEnabled) {
            invoke.reject("bluetooth is off or unavailable on this device")
            return
        }

        Thread {
            try {
                BluetoothSpp.send(adapter, mac, payload)
                invoke.resolve()
            } catch (e: SecurityException) {
                invoke.reject("bluetooth permission denied: ${e.message}", BLUETOOTH_PERMISSION_DENIED)
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString(), e)
            }
        }.start()
    }

    /**
     * `nfcRead` — opens NFC reader mode and answers with the badge of the first card tapped
     * (hub#988).
     *
     * The counter reads badges through a USB reader that types the number like a keyboard; a
     * tablet has no such reader and, until this existed, could not enrol or read a card at all
     * even though the hardware was already inside it. What comes back is the same KIND of string
     * the wedge types (`NfcBadge.toBadge`), so the shell hands it to the very subscribers the
     * keyboard path feeds: one badge path, two origins.
     *
     * The window is bounded and the answer is always exactly one. Nothing tapped resolves with the
     * key ABSENT, because that is the ordinary outcome of a poll and not a failure — the shell
     * calls this in a loop while a screen waits for a card, and an error every fifteen seconds
     * would bury the real ones.
     *
     * Reader mode is closed on every exit. Left open it survives the screen that asked for it, and
     * the next tap is delivered to a callback whose `invoke` is long since resolved — the tap the
     * user makes and nothing answers.
     *
     * The three refusals stay apart on purpose: no reader, reader switched off, and a card that
     * randomises its id are three different things to do about it, and one shared "it did not
     * work" would send a user to a settings toggle their tablet does not have (hub#338).
     */
    @Command
    fun nfcRead(invoke: Invoke) {
        val adapter = android.nfc.NfcAdapter.getDefaultAdapter(activity)
        if (adapter == null) {
            invoke.reject("this device has no NFC reader", NfcBadge.NFC_UNAVAILABLE)
            return
        }
        if (!adapter.isEnabled) {
            invoke.reject("NFC is switched off on this device", NfcBadge.NFC_DISABLED)
            return
        }

        val requested = invoke.getArgs().let { if (it.has("timeoutMs")) it.optLong("timeoutMs") else null }
        val timeout = NfcBadge.clampTimeout(requested)

        // ONE answer, whoever gets there first: the tap and the deadline race, and resolving an
        // `invoke` twice throws on the second. `AtomicBoolean` and not a flag — the callback runs
        // on a binder thread and the deadline on the main looper.
        val answered = java.util.concurrent.atomic.AtomicBoolean(false)
        val main = android.os.Handler(android.os.Looper.getMainLooper())

        val close = { main.post { runCatching { adapter.disableReaderMode(activity) } } }

        val callback = android.nfc.NfcAdapter.ReaderCallback { tag ->
            if (!answered.compareAndSet(false, true)) return@ReaderCallback
            close()
            val badge = NfcBadge.badgeOf(tag?.id)
            if (badge == null) {
                invoke.reject(
                    "this card answers with a new id on every tap",
                    NfcBadge.NFC_RANDOM_UID,
                )
            } else {
                invoke.resolve(JSObject().put("badge", badge))
            }
        }

        main.post {
            try {
                adapter.enableReaderMode(activity, callback, NfcBadge.READER_FLAGS, null)
            } catch (e: Exception) {
                // Reader mode needs a RESUMED activity: asked for from the background it throws,
                // and swallowing that would leave the caller waiting out the whole window for a
                // reader that was never opened.
                if (answered.compareAndSet(false, true)) invoke.reject(e.message ?: e.toString(), e)
            }
        }

        main.postDelayed({
            if (answered.compareAndSet(false, true)) {
                close()
                // Nothing tapped. An EMPTY answer, not a refusal: `put(key, null)` on org.json
                // removes the key, so the absence is written by simply not putting it.
                invoke.resolve(JSObject())
            }
        }, timeout)
    }

    private fun concedidos(): Set<String> =
        PermissionPolicy.required()
            .filter { ContextCompat.checkSelfPermission(activity, it) == PackageManager.PERMISSION_GRANTED }
            .toSet()

    private fun estadoActual(): JSObject {
        val resultado = JSObject()
        for ((permiso, concedido) in statusOf(PermissionPolicy.required(), concedidos())) {
            resultado.put(permiso, concedido)
        }
        return resultado
    }

    companion object {
        /**
         * The code a bluetooth operation is refused under when `BLUETOOTH_CONNECT` is denied
         * (ADR-0204). Mirror of `BLUETOOTH_PERMISSION_DENIED` on the Rust side — same trip as
         * `downloads_unreachable`: Kotlin rejects with a code, Tauri renders `[code] - message`,
         * Rust recognises it out of the text.
         */
        const val BLUETOOTH_PERMISSION_DENIED = "bluetooth_permission_denied"

        /** Los que hay que pedir: los requeridos que aún no están concedidos. */
        @JvmStatic
        fun pendingOf(required: List<String>, granted: Set<String>): List<String> =
            required.filterNot { it in granted }

        /**
         * What a request may actually ask for: the caller's scope, clamped to what THIS Android
         * requires (hub#758).
         *
         * `null` (no scope sent — a web older than the scope) keeps the old behavior and asks for
         * everything. The intersection is not decoration: the policy stays the single authority
         * on what can be asked, so a scope cannot smuggle in a permission this API level does not
         * know — requesting one can hang the dialog on some vendors.
         */
        @JvmStatic
        fun requestScope(required: List<String>, requested: List<String>?): List<String> =
            if (requested == null) required else required.filter { it in requested }

        /**
         * Estado a devolver a la PWA. Solo incluye los permisos que la versión de Android conoce:
         * de los demás no se puede afirmar nada, así que informar de ellos sería mentir.
         */
        @JvmStatic
        fun statusOf(required: List<String>, granted: Set<String>): Map<String, Boolean> =
            required.associateWith { it in granted }
    }
}

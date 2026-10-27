package com.erplora.android

import android.graphics.Bitmap
import android.net.Uri
import android.os.Message
import android.util.Log
import android.view.View
import android.webkit.ConsoleMessage
import android.webkit.GeolocationPermissions
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import java.net.URI
import java.util.Locale

/**
 * Which pages may use the device's microphone and position (hub#247, hub#2552).
 *
 * wry's `RustWebChromeClient` answers `getUserMedia` and `navigator.geolocation` for WHATEVER page
 * asks: it asks Android once, for the whole app, and from then on hands the capture to every
 * origin. The window does load pages that are not ours — `navigation_verdict` allows every host
 * that is not the SaaS, and with no multi-window support a `target=_blank` or a `window.open` loads
 * in the same window — so once the person granted the microphone to dictate to the assistant, any
 * of those pages could record with no dialog at all.
 *
 * So the app decides by ORIGIN, with the rule its Rust half already uses for «one of ours»
 * (`trusted_hub_origin`, `is_hub_domain` in `src-tauri/src/lib.rs`): an https hub
 * `<label>(.<label>)*.erplora.com`, the SaaS at the apex, and the loopback for development. A
 * refused page gets its `NotAllowedError` / `PERMISSION_DENIED` and Android is never asked.
 */
object WebPermissionGate {

    /** The domain every hub and the SaaS live under (`HUB_DOMAIN_SUFFIX` on the Rust side). */
    const val DOMAIN = "erplora.com"

    private const val TAG = "ErploraWebPermissions"

    /** The development exemption: nothing outside this machine can serve it. */
    private val LOOPBACK = setOf("127.0.0.1", "localhost", "[::1]")

    /** Is the page at [origin] one of ours, and so allowed to ask for the microphone or position? */
    @JvmStatic
    fun isTrusted(origin: String?): Boolean {
        if (origin.isNullOrEmpty()) return false
        val uri = runCatching { URI(origin) }.getOrNull() ?: return false
        val host = uri.host?.lowercase(Locale.ROOT) ?: return false
        return when (uri.scheme?.lowercase(Locale.ROOT)) {
            "https" -> host == DOMAIN || isHubDomain(host) || host in LOOPBACK
            "http" -> host in LOOPBACK
            else -> false
        }
    }

    /**
     * The client to set in place of [current], or `null` when there is nothing to do: no client yet
     * (the WebView's default refuses every request), or the guard is already the one in place.
     */
    @JvmStatic
    fun guard(current: WebChromeClient?): WebChromeClient? = when (current) {
        null, is OriginGuardedChromeClient -> null
        else -> OriginGuardedChromeClient(current)
    }

    /**
     * The client to install, or `null`: [current] is only read when [canRead], because
     * `WebViewCompat.getWebChromeClient` THROWS `UnsupportedOperationException` on a WebView without
     * `GET_WEB_CHROME_CLIENT` (API 24-25 with the factory WebView; the platform getter is API 26),
     * and from `post` that would kill the app at start. Such a device keeps wry's client.
     */
    @JvmStatic
    fun clientFor(canRead: Boolean, current: () -> WebChromeClient?): WebChromeClient? =
        if (canRead) guard(current()) else null

    /**
     * Puts the guard in front of the client wry set on [webView].
     *
     * Called through `post` from the plugin's `load`. Tauri calls `load` from wry's
     * `on_webview_created` hook (tauri 2.11.5, wry 0.55.1 `main_pipe.rs`), which runs AFTER wry's
     * `setWebChromeClient` in the same turn of the main looper; `post` keeps the install on the next
     * turn so it finds the client whatever that order becomes — still before the page's first
     * callback, which Chromium queues behind it. Where the WebView cannot hand back its client
     * (see [clientFor]) nothing is installed and the device keeps wry's, instead of a crash at start.
     */
    @JvmStatic
    fun install(webView: WebView) {
        val canRead = WebViewFeature.isFeatureSupported(WebViewFeature.GET_WEB_CHROME_CLIENT)
        val guarded = clientFor(canRead) { WebViewCompat.getWebChromeClient(webView) }
        if (guarded == null) {
            Log.w(
                TAG,
                if (canRead) "no WebChromeClient to guard, or already guarded"
                else "this WebView cannot hand back its WebChromeClient (API 24-25, factory WebView): the microphone and the position stay wry's, for every page",
            )
            return
        }
        webView.webChromeClient = guarded
    }

    /** `<label>(.<label>)*.erplora.com`, every label a plain lowercase DNS label (`is_dns_label`). */
    private fun isHubDomain(host: String): Boolean {
        val subdomain = host.removeSuffix(".$DOMAIN")
        if (subdomain == host || subdomain.isEmpty()) return false
        return subdomain.split('.').all(::isDnsLabel)
    }

    private fun isDnsLabel(label: String): Boolean =
        label.isNotEmpty() &&
            label.length <= 63 &&
            !label.startsWith('-') &&
            !label.endsWith('-') &&
            label.all { it in 'a'..'z' || it in '0'..'9' || it == '-' }
}

/**
 * wry's client with the origin check in front of its two permission prompts; every other callback
 * goes to wry untouched (`WebPermissionGateTest` checks that none of the platform's is left out).
 *
 * [originOf] reads the origin off a request; it is a parameter only so the JVM tests can name one
 * without an `android.net.Uri`.
 */
class OriginGuardedChromeClient(
    private val inner: WebChromeClient,
    private val originOf: (PermissionRequest) -> String? = { it.origin?.toString() },
) : WebChromeClient() {

    // ── The two prompts that hand the page a piece of the device ───────────────────────────────

    override fun onPermissionRequest(request: PermissionRequest?) {
        if (request == null) return
        if (WebPermissionGate.isTrusted(originOf(request))) {
            inner.onPermissionRequest(request)
        } else {
            request.deny()
        }
    }

    override fun onGeolocationPermissionsShowPrompt(origin: String?, callback: GeolocationPermissions.Callback?) {
        if (WebPermissionGate.isTrusted(origin)) {
            inner.onGeolocationPermissionsShowPrompt(origin, callback)
        } else {
            callback?.invoke(origin, false, false)
        }
    }

    // ── Everything else: wry's ──────────────────────────────────────────────────────────────────

    override fun onPermissionRequestCanceled(request: PermissionRequest?) = inner.onPermissionRequestCanceled(request)

    override fun onGeolocationPermissionsHidePrompt() = inner.onGeolocationPermissionsHidePrompt()

    override fun onShowFileChooser(
        webView: WebView?,
        filePathCallback: ValueCallback<Array<Uri>>?,
        fileChooserParams: FileChooserParams?,
    ): Boolean = inner.onShowFileChooser(webView, filePathCallback, fileChooserParams)

    override fun onJsAlert(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean =
        inner.onJsAlert(view, url, message, result)

    override fun onJsConfirm(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean =
        inner.onJsConfirm(view, url, message, result)

    override fun onJsPrompt(
        view: WebView?,
        url: String?,
        message: String?,
        defaultValue: String?,
        result: JsPromptResult?,
    ): Boolean = inner.onJsPrompt(view, url, message, defaultValue, result)

    override fun onJsBeforeUnload(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean =
        inner.onJsBeforeUnload(view, url, message, result)

    override fun onConsoleMessage(consoleMessage: ConsoleMessage?): Boolean = inner.onConsoleMessage(consoleMessage)

    override fun onProgressChanged(view: WebView?, newProgress: Int) = inner.onProgressChanged(view, newProgress)

    override fun onReceivedTitle(view: WebView?, title: String?) = inner.onReceivedTitle(view, title)

    override fun onReceivedIcon(view: WebView?, icon: Bitmap?) = inner.onReceivedIcon(view, icon)

    override fun onReceivedTouchIconUrl(view: WebView?, url: String?, precomposed: Boolean) =
        inner.onReceivedTouchIconUrl(view, url, precomposed)

    override fun onShowCustomView(view: View?, callback: CustomViewCallback?) = inner.onShowCustomView(view, callback)

    override fun onHideCustomView() = inner.onHideCustomView()

    override fun onCreateWindow(view: WebView?, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message?): Boolean =
        inner.onCreateWindow(view, isDialog, isUserGesture, resultMsg)

    override fun onRequestFocus(view: WebView?) = inner.onRequestFocus(view)

    override fun onCloseWindow(window: WebView?) = inner.onCloseWindow(window)

    override fun getDefaultVideoPoster(): Bitmap? = inner.defaultVideoPoster

    override fun getVideoLoadingProgressView(): View? = inner.videoLoadingProgressView

    override fun getVisitedHistory(callback: ValueCallback<Array<String>>?) = inner.getVisitedHistory(callback)
}

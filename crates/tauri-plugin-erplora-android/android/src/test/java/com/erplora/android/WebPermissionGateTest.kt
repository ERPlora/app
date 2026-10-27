package com.erplora.android

import android.graphics.Bitmap
import android.net.Uri
import android.os.Message
import android.view.View
import android.webkit.ConsoleMessage
import android.webkit.GeolocationPermissions
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import java.lang.reflect.Modifier
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertSame
import kotlin.test.assertTrue
import kotlin.test.fail

/**
 * Which pages may use the device's microphone and position (hub#247, hub#2552).
 *
 * wry's `RustWebChromeClient` grants both to WHATEVER page asks: it asks Android once for the app
 * and, once granted, hands the capture to every origin. And the window does load pages that are not
 * ours — `navigation_verdict` allows every host that is not the SaaS, and with no multi-window
 * support a `target=_blank` or `window.open` loads in the same window. Once the person granted the
 * microphone to dictate to the assistant, a foreign page could record with no dialog at all.
 *
 * So the app decides by ORIGIN, with the rule the Rust side already uses for «is this one of ours»
 * (`trusted_hub_origin`, `is_hub_domain`): an https hub `<label>(.<label>)*.erplora.com`, the SaaS at
 * the apex, and the loopback for development. Everything else is refused without asking Android.
 */
class WebPermissionGateTest {

    // ── The rule ────────────────────────────────────────────────────────────────────────────────

    @Test
    fun `trusts the hubs, the SaaS and the loopback`() {
        for (origin in listOf(
            "https://panaderia.a.erplora.com",
            "https://panaderia.a.erplora.com/",
            "https://panaderia.3.erplora.com:8443",
            "https://erplora.com",
            "https://www.erplora.com/",
            "https://pre.erplora.com",
            "https://xn--panadera-i2a.a.erplora.com",
            "http://127.0.0.1:8000",
            "http://localhost:5173/",
            "https://localhost",
            "http://[::1]:8000",
        )) {
            assertTrue(WebPermissionGate.isTrusted(origin), "$origin is ours and must keep its microphone")
        }
    }

    @Test
    fun `refuses every other origin`() {
        for (origin in listOf(
            "https://accounts.google.com",
            "https://evil.example/",
            // Plain http is only the development exemption, on this machine.
            "http://panaderia.a.erplora.com",
            "http://erplora.com",
            // Look-alikes: a suffix is not a domain.
            "https://evilerplora.com",
            "https://erplora.com.evil.net",
            "https://panaderia.a.erplora.com.evil.net",
            "https://evil.com/?x=erplora.com",
            "https://evil.com/panaderia.a.erplora.com",
            "https://.erplora.com",
            "https://a..erplora.com",
            "https://-a.erplora.com",
            "https://a_b.erplora.com",
            // Not a web origin of ours: the bundled page, files, opaque origins, other machines.
            "http://tauri.localhost",
            "tauri://localhost",
            "file:///sdcard/index.html",
            "data:text/html,hi",
            "null",
            "",
            "http://10.0.2.2:8000",
            "http://192.168.1.20:8000",
            "not a url",
        )) {
            assertFalse(WebPermissionGate.isTrusted(origin), "$origin is not ours and must not get the microphone")
        }
        assertFalse(WebPermissionGate.isTrusted(null), "an origin Android could not name is not ours")
    }

    @Test
    fun `reads the host the way the browser reports it`() {
        // Chromium hands origins over lowercased; an upper-case spelling is the same site.
        assertTrue(WebPermissionGate.isTrusted("https://Panaderia.A.ERPLORA.com"))
    }

    // ── The microphone (and the camera): `getUserMedia` ────────────────────────────────────────

    @Test
    fun `a foreign page asking for the microphone is refused and wry never hears of it`() {
        val wry = RecordingClient()
        val request = FakeRequest()
        guarded(wry, origin = "https://evil.example/").onPermissionRequest(request)

        assertTrue(request.denied, "the page must get NotAllowedError")
        assertNull(wry.permissionRequest, "wry would ask Android and hand the capture over")
    }

    @Test
    fun `a request Android cannot attribute to an origin is refused`() {
        val wry = RecordingClient()
        val request = FakeRequest()
        guarded(wry, origin = null).onPermissionRequest(request)

        assertTrue(request.denied)
        assertNull(wry.permissionRequest)
    }

    @Test
    fun `a hub asking for the microphone goes on to wry, which asks Android`() {
        val wry = RecordingClient()
        val request = FakeRequest()
        guarded(wry, origin = "https://panaderia.a.erplora.com/").onPermissionRequest(request)

        assertSame(request, wry.permissionRequest)
        assertFalse(request.denied)
    }

    @Test
    fun `by default the origin is read off the request itself`() {
        // The seam the tests above use must default to the request's own origin, never to a
        // trusted one: this request names none (`getOrigin()` is null), so it is refused.
        val wry = RecordingClient()
        val request = FakeRequest()
        OriginGuardedChromeClient(wry).onPermissionRequest(request)

        assertEquals(1, request.originReads, "the guard must ask the request where it comes from")
        assertTrue(request.denied)
        assertNull(wry.permissionRequest)
    }

    // ── The position: `navigator.geolocation` ───────────────────────────────────────────────────

    @Test
    fun `a foreign page asking where the device is gets a no, and wry never hears of it`() {
        val wry = RecordingClient()
        val callback = FakeGeoCallback()
        guarded(wry).onGeolocationPermissionsShowPrompt("https://evil.example/", callback)

        assertEquals(Triple("https://evil.example/", false, false), callback.answer)
        assertNull(wry.geolocationOrigin)
    }

    @Test
    fun `a hub asking where the device is goes on to wry`() {
        val wry = RecordingClient()
        val callback = FakeGeoCallback()
        guarded(wry).onGeolocationPermissionsShowPrompt("https://panaderia.a.erplora.com/", callback)

        assertEquals("https://panaderia.a.erplora.com/", wry.geolocationOrigin)
        assertSame(callback, wry.geolocationCallback)
        assertNull(callback.answer, "wry answers it, after asking Android")
    }

    // ── Everything else is wry's, untouched ─────────────────────────────────────────────────────

    @Test
    fun `every other callback reaches wry with its answer`() {
        val wry = RecordingClient()
        val client = guarded(wry)

        assertTrue(client.onShowFileChooser(null, null, null), "attaching a file to the assistant (BASE-F94)")
        assertTrue(client.onJsAlert(null, "u", "m", null))
        assertTrue(client.onJsConfirm(null, "u", "m", null))
        assertTrue(client.onJsPrompt(null, "u", "m", "d", null))
        assertTrue(client.onConsoleMessage(null))
        client.onReceivedTitle(null, "Caja")
        client.onPermissionRequestCanceled(null)
        client.onGeolocationPermissionsHidePrompt()

        assertEquals(
            listOf(
                "onShowFileChooser", "onJsAlert", "onJsConfirm", "onJsPrompt", "onConsoleMessage",
                "onReceivedTitle", "onPermissionRequestCanceled", "onGeolocationPermissionsHidePrompt",
            ),
            wry.calls,
        )
    }

    @Test
    fun `no callback of WebChromeClient is left out of the delegation`() {
        // A callback the guard does not override answers with WebChromeClient's default instead of
        // wry's — a file chooser that never opens, an alert that never shows — with no error. So the
        // guard overrides every current callback of the platform class; only the deprecated ones,
        // which wry does not use either, are left to the default.
        val missing = WebChromeClient::class.java.declaredMethods
            .filter { Modifier.isPublic(it.modifiers) && !Modifier.isStatic(it.modifiers) }
            .filter { !it.isAnnotationPresent(java.lang.Deprecated::class.java) }
            .filter { method ->
                runCatching {
                    OriginGuardedChromeClient::class.java.getDeclaredMethod(method.name, *method.parameterTypes)
                }.isFailure
            }
            .map { "${it.name}(${it.parameterTypes.joinToString { type -> type.simpleName }})" }

        assertEquals(emptyList(), missing, "callbacks the guard would swallow")
    }

    // ── Installing it ───────────────────────────────────────────────────────────────────────────

    @Test
    fun `wraps the client wry set, once`() {
        val wry = RecordingClient()
        val installed = WebPermissionGate.guard(wry)
        assertTrue(installed is OriginGuardedChromeClient, "wry's client must be wrapped")

        assertNull(WebPermissionGate.guard(installed), "wrapping the guard again would only stack it")
        assertNull(WebPermissionGate.guard(null), "nothing set yet: nothing to wrap, and nothing grants")
    }

    @Test
    fun `where the WebView cannot hand back its client, nothing is read and nothing is installed`() {
        // `WebViewCompat.getWebChromeClient` THROWS on a WebView without `GET_WEB_CHROME_CLIENT` (API
        // 24-25 with the factory WebView: the platform getter is API 26); inside `post` that would kill
        // the app at start. Such a device keeps wry's client, so the till keeps selling.
        assertNull(
            WebPermissionGate.clientFor(canRead = false) { fail("the client must not be read: reading it throws") },
            "nothing to install where the client cannot be read",
        )
        assertTrue(WebPermissionGate.clientFor(canRead = true) { RecordingClient() } is OriginGuardedChromeClient)
    }

    // ── Fakes ───────────────────────────────────────────────────────────────────────────────────

    private fun guarded(wry: WebChromeClient, origin: String? = null) =
        OriginGuardedChromeClient(wry) { origin }

    private class FakeRequest : PermissionRequest() {
        var denied = false
        var granted: Array<String>? = null
        var originReads = 0
        override fun getOrigin(): Uri? { originReads++; return null }
        override fun getResources(): Array<String> = arrayOf(RESOURCE_AUDIO_CAPTURE)
        override fun grant(resources: Array<String>?) { granted = resources }
        override fun deny() { denied = true }
    }

    private class FakeGeoCallback : GeolocationPermissions.Callback {
        var answer: Triple<String?, Boolean, Boolean>? = null
        override fun invoke(origin: String?, allow: Boolean, retain: Boolean) {
            answer = Triple(origin, allow, retain)
        }
    }

    /** Stands in for wry's `RustWebChromeClient`: records what reached it and answers `true`. */
    private class RecordingClient : WebChromeClient() {
        val calls = mutableListOf<String>()
        var permissionRequest: PermissionRequest? = null
        var geolocationOrigin: String? = null
        var geolocationCallback: GeolocationPermissions.Callback? = null

        override fun onPermissionRequest(request: PermissionRequest?) { permissionRequest = request }
        override fun onGeolocationPermissionsShowPrompt(origin: String?, callback: GeolocationPermissions.Callback?) {
            geolocationOrigin = origin
            geolocationCallback = callback
        }
        override fun onShowFileChooser(
            webView: WebView?,
            filePathCallback: ValueCallback<Array<Uri>>?,
            fileChooserParams: FileChooserParams?,
        ): Boolean { calls += "onShowFileChooser"; return true }
        override fun onJsAlert(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean {
            calls += "onJsAlert"; return true
        }
        override fun onJsConfirm(view: WebView?, url: String?, message: String?, result: JsResult?): Boolean {
            calls += "onJsConfirm"; return true
        }
        override fun onJsPrompt(
            view: WebView?,
            url: String?,
            message: String?,
            defaultValue: String?,
            result: JsPromptResult?,
        ): Boolean { calls += "onJsPrompt"; return true }
        override fun onConsoleMessage(consoleMessage: ConsoleMessage?): Boolean {
            calls += "onConsoleMessage"; return true
        }
        override fun onReceivedTitle(view: WebView?, title: String?) { calls += "onReceivedTitle" }
        override fun onPermissionRequestCanceled(request: PermissionRequest?) { calls += "onPermissionRequestCanceled" }
        override fun onGeolocationPermissionsHidePrompt() { calls += "onGeolocationPermissionsHidePrompt" }
        override fun onCreateWindow(view: WebView?, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message?) = false
        override fun onShowCustomView(view: View?, callback: CustomViewCallback?) {}
        override fun getDefaultVideoPoster(): Bitmap? = null
    }
}

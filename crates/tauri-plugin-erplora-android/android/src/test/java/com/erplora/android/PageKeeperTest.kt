package com.erplora.android

import android.content.Context
import android.content.ContextWrapper
import android.view.View
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * hub#2307 — the glue that keeps the page running while the app listens with the screen off.
 *
 * The foreground service alone kept the process and lost the page: Chromium froze it a minute after
 * Android hid it (measured: 5 heartbeats and a `freeze` at 60 s, against 18/18 with this class).
 * None of that is seen by the compiler or by the pure decision's test, so without these the fix can
 * be unplugged — the keeper never told to listen, never installed, deaf to the lifecycle — with
 * every other test green and the notices lost again.
 *
 * The Android classes are the stubs of the unit-test jar (`isReturnDefaultValues`): the WebView
 * below only records what it is asked to do, and runs at once what is posted to it.
 */
class PageKeeperTest {

    private val context: Context = ContextWrapper(null)

    private inner class RecordingWebView : WebView(context) {
        val asked = mutableListOf<String>()
        val posted = mutableListOf<Runnable>()
        var runsPostsAtOnce = true

        override fun onResume() {
            asked += "resume"
        }

        override fun dispatchWindowVisibilityChanged(visibility: Int) {
            asked += "window:$visibility"
        }

        override fun post(action: Runnable): Boolean {
            if (runsPostsAtOnce) action.run() else posted += action
            return true
        }
    }

    private class Owner : LifecycleOwner {
        val registry = LifecycleRegistry.createUnsafe(this)
        override val lifecycle: Lifecycle get() = registry
    }

    private val shownAgain = listOf("resume", "window:${View.VISIBLE}")

    private fun installed(webView: WebView): Pair<PageKeeper, Owner> {
        val owner = Owner()
        val keeper = PageKeeper(webView)
        keeper.install(FrameLayout(context), owner)
        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        return keeper to owner
    }

    @Test
    fun `while listening the page is resumed and shown again when the app leaves the screen`() {
        val webView = RecordingWebView()
        val (keeper, owner) = installed(webView)
        keeper.setListening(true)

        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)

        assertEquals(shownAgain, webView.asked)
    }

    @Test
    fun `and again when the activity stops, which hides the window after the pause`() {
        val webView = RecordingWebView()
        val (keeper, owner) = installed(webView)
        keeper.setListening(true)
        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        webView.asked.clear()

        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_STOP)

        assertEquals(shownAgain, webView.asked)
    }

    @Test
    fun `while listening a window that stops being seen is shown again to the page`() {
        val webView = RecordingWebView()
        val (keeper, _) = installed(webView)
        keeper.setListening(true)

        keeper.onWindowVisibility(View.GONE)

        assertEquals(shownAgain, webView.asked)
    }

    @Test
    fun `a window coming back is left to Android`() {
        val webView = RecordingWebView()
        val (keeper, _) = installed(webView)
        keeper.setListening(true)

        keeper.onWindowVisibility(View.VISIBLE)

        assertEquals(emptyList(), webView.asked)
    }

    @Test
    fun `nobody listening, the page is paused and hidden as Android always did`() {
        val webView = RecordingWebView()
        val (keeper, owner) = installed(webView)

        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_STOP)
        keeper.onWindowVisibility(View.GONE)

        assertEquals(emptyList(), webView.asked)
    }

    @Test
    fun `once told to stop the page is no longer kept, not even by what was already on its way`() {
        val webView = RecordingWebView().apply { runsPostsAtOnce = false }
        val (keeper, owner) = installed(webView)
        keeper.setListening(true)
        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        keeper.onWindowVisibility(View.GONE)
        assertEquals(2, webView.posted.size)

        keeper.setListening(false)
        webView.posted.forEach { it.run() }

        assertEquals(emptyList(), webView.asked)
        owner.registry.handleLifecycleEvent(Lifecycle.Event.ON_STOP)
        assertEquals(2, webView.posted.size)
    }
}

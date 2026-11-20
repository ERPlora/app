package com.erplora.android

import androidx.activity.OnBackPressedCallback
import androidx.activity.OnBackPressedDispatcher
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import kotlin.test.Test
import kotlin.test.assertEquals

class BackToBackgroundTest {
    /** The activity as Back sees it: the dispatcher's fallback is Android finishing the activity. */
    private class Activity : LifecycleOwner {
        var finished = 0
        var movedToBack = 0
        private val registry = LifecycleRegistry.createUnsafe(this)
        override val lifecycle: Lifecycle get() = registry
        val dispatcher = OnBackPressedDispatcher { finished++ }

        fun moveTaskToBack() {
            movedToBack++
        }

        fun start() = registry.handleLifecycleEvent(Lifecycle.Event.ON_START)

        fun stop() = registry.handleLifecycleEvent(Lifecycle.Event.ON_STOP)
    }

    /**
     * Tauri's own handler (`app.tauri.AppPlugin`) as it ships, tied to the activity's lifecycle: a
     * page that listens gets the press, else the WebView goes back in its history, else it steps
     * aside and presses Back again.
     */
    private class TauriBack(activity: Activity) {
        var pageListens = false
        var canGoBack = false
        var pageTold = 0
        var wentBack = 0

        init {
            val callback =
                object : OnBackPressedCallback(true) {
                    override fun handleOnBackPressed() {
                        when {
                            pageListens -> pageTold++
                            canGoBack -> wentBack++
                            else -> {
                                isEnabled = false
                                activity.dispatcher.onBackPressed()
                                isEnabled = true
                            }
                        }
                    }
                }
            activity.dispatcher.addCallback(activity, callback)
        }
    }

    /** Tauri's handler can appear during `super.onCreate`, after it, or once the activity runs. */
    private fun everyOrderOfInstall(check: (Activity, TauriBack) -> Unit) {
        val beforeOurs = Activity()
        val tauriFirst = TauriBack(beforeOurs)
        BackToBackground.install(beforeOurs.dispatcher, beforeOurs::moveTaskToBack)
        beforeOurs.start()
        check(beforeOurs, tauriFirst)

        val afterOurs = Activity()
        BackToBackground.install(afterOurs.dispatcher, afterOurs::moveTaskToBack)
        val tauriSecond = TauriBack(afterOurs)
        afterOurs.start()
        check(afterOurs, tauriSecond)

        val onceStarted = Activity()
        BackToBackground.install(onceStarted.dispatcher, onceStarted::moveTaskToBack)
        onceStarted.start()
        check(onceStarted, TauriBack(onceStarted))
    }

    @Test
    fun `a Back that Tauri lets through sends the app to the background instead of finishing it`() {
        everyOrderOfInstall { activity, _ ->
            activity.dispatcher.onBackPressed()

            assertEquals(1, activity.movedToBack)
            assertEquals(0, activity.finished)
        }
    }

    @Test
    fun `Tauri still goes first - a page that listens gets the press and history goes back`() {
        everyOrderOfInstall { activity, tauri ->
            tauri.pageListens = true
            activity.dispatcher.onBackPressed()
            tauri.pageListens = false
            tauri.canGoBack = true
            activity.dispatcher.onBackPressed()

            assertEquals(1, tauri.pageTold)
            assertEquals(1, tauri.wentBack)
            assertEquals(0, activity.movedToBack)
            assertEquals(0, activity.finished)
        }
    }

    @Test
    fun `Tauri still goes first after the app comes back from the background`() {
        everyOrderOfInstall { activity, tauri ->
            activity.stop()
            activity.start()
            tauri.canGoBack = true
            activity.dispatcher.onBackPressed()

            assertEquals(1, tauri.wentBack)
            assertEquals(0, activity.movedToBack)
        }
    }
}

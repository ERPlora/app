package com.erplora.android

import androidx.activity.OnBackPressedCallback
import androidx.activity.OnBackPressedDispatcher

/**
 * Back with nowhere left to go sends the app to the background on every screen, as Android 12+
 * does with an app's root screen and as the business page already does through `leave_app`.
 *
 * Without it, a page that does not take Back (erplora.com's sign-in, Google's) lets Tauri hand the
 * press to Android, which finishes the activity, and the process aborts while the window is torn
 * down (`FORTIFY: pthread_mutex_lock called on a destroyed mutex`).
 *
 * Installed in `onCreate`, without a lifecycle owner, it stays below Tauri's handler: that one is
 * tied to the activity's lifecycle, so the dispatcher adds it back on top on every start, and it
 * still goes back in the WebView history and hands the press to a page that listens.
 */
object BackToBackground {
    fun install(dispatcher: OnBackPressedDispatcher, moveTaskToBack: () -> Unit) {
        val callback =
            object : OnBackPressedCallback(true) {
                override fun handleOnBackPressed() = moveTaskToBack()
            }
        dispatcher.addCallback(callback)
    }
}

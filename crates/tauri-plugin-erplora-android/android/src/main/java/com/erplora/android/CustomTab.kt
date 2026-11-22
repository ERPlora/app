package com.erplora.android

import android.app.Activity
import android.content.ActivityNotFoundException
import android.net.Uri
import android.util.Log
import androidx.browser.customtabs.CustomTabsIntent
import app.tauri.plugin.Invoke

/**
 * The browser half of «Continue with Google» on Android: a tab of the device's browser over the app.
 * It shares the browser's session, which already has the Google accounts of the device, so Google
 * offers them instead of asking for the address and the password. Only the app opens one
 * (`openCustomTab`), and only for a page of the web.
 */
object CustomTab {
    /** No browser on the device could show it. */
    const val FAILED = "custom_tab_failed"
    private const val TAG = "ErploraCustomTab"

    @JvmStatic
    fun isWebPage(url: String?): Boolean = url != null && (url.startsWith("https://") || url.startsWith("http://"))

    fun open(activity: Activity, invoke: Invoke) {
        val url = invoke.getArgs().getString("url", null)
        if (!isWebPage(url)) {
            invoke.reject(FAILED)
            return
        }
        activity.runOnUiThread {
            try {
                CustomTabsIntent.Builder().build().launchUrl(activity, Uri.parse(url))
                invoke.resolve()
            } catch (e: ActivityNotFoundException) {
                Log.w(TAG, "No browser on the device opens a tab", e)
                invoke.reject(FAILED)
            }
        }
    }
}

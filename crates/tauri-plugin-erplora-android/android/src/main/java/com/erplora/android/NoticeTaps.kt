package com.erplora.android

import android.content.Context
import android.content.Intent

/**
 * The process's one [NoticeTapBox] (hub#2360), fed from the two places a tap reaches the app before
 * the page listens — the plugin's `load` and `MainActivity.onNewIntent`. The tap the task was born
 * from is remembered in the app's preferences, so a process the system brings back does not keep
 * it again.
 */
object NoticeTaps {
    private const val PREFS = "com.erplora.notice_taps"
    private const val LAST_KEPT = "last_kept"

    @Volatile private var box: NoticeTapBox? = null

    private fun box(context: Context): NoticeTapBox =
        box ?: synchronized(this) {
            box ?: NoticeTapBox(PreferencesMemory(context.applicationContext)).also { box = it }
        }

    fun pageLoading(context: Context, intent: Intent?) {
        box(context).pageLoading(launchOf(intent))
    }

    fun newIntent(context: Context, intent: Intent?) {
        box(context).newIntent(launchOf(intent))
    }

    fun take(): NoticeLaunch.Tap? = box?.take()

    private fun launchOf(intent: Intent?): NoticeLaunch.Launch =
        if (intent == null) {
            NoticeLaunch.Launch(null, 0, NoticeLaunch.NO_ID, null, null)
        } else {
            NoticeLaunch.Launch(
                intent.action,
                intent.flags,
                intent.getIntExtra(NoticeLaunch.ID_KEY, NoticeLaunch.NO_ID),
                intent.getStringExtra(NoticeLaunch.USER_ACTION_KEY),
                intent.getStringExtra(NoticeLaunch.NOTIFICATION_KEY),
            )
        }

    private class PreferencesMemory(context: Context) : KeptTapMemory {
        private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        override fun last(): String? = prefs.getString(LAST_KEPT, null)
        override fun remember(key: String) {
            prefs.edit().putString(LAST_KEPT, key).apply()
        }
    }
}

package com.erplora.android

import android.content.Context
import android.content.Intent

/**
 * The process's one [NoticeTapBox] (hub#2360), fed from the two places a tap reaches the app — the
 * plugin's `load` and `MainActivity.onNewIntent`. The tap the task was born from is remembered in the
 * app's preferences, so a process the system brings back does not keep it again. So is the screen
 * each notice leads to ([NoticeTargets], hub#1039): the tap brings back the notice's number only.
 */
object NoticeTaps {
    private const val PREFS = "com.erplora.notice_taps"
    private const val LAST_KEPT = "last_kept"
    private const val TARGETS = "targets"

    @Volatile private var box: NoticeTapBox? = null
    @Volatile private var targets: NoticeTargets? = null

    private fun targets(context: Context): NoticeTargets =
        targets ?: synchronized(this) {
            targets ?: NoticeTargets(PreferencesTargets(context.applicationContext)).also { targets = it }
        }

    private fun box(context: Context): NoticeTapBox =
        box ?: synchronized(this) {
            box ?: NoticeTapBox(PreferencesMemory(context.applicationContext)) { targets(context).pathOf(it) }
                .also { box = it }
        }

    /** The screen notice `id` leads to, written down as it goes out (`rememberNotice`). */
    fun remember(context: Context, id: Int, path: String?) {
        targets(context).remember(id, path)
    }

    /** Who tells the page that a tap waits while it listens (`watchNoticeTaps`). */
    fun onTapWaiting(context: Context, listener: () -> Unit) {
        box(context).onTapWaiting(listener)
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

    private class PreferencesTargets(context: Context) : NoticeTargetsMemory {
        private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        override fun read(): String? = prefs.getString(TARGETS, null)
        override fun write(value: String) {
            prefs.edit().putString(TARGETS, value).apply()
        }
    }

    private class PreferencesMemory(context: Context) : KeptTapMemory {
        private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        override fun last(): String? = prefs.getString(LAST_KEPT, null)
        override fun remember(key: String) {
            prefs.edit().putString(LAST_KEPT, key).apply()
        }
    }
}

package com.erplora.android

import android.content.Intent

/**
 * The tap on a notice that STARTED the app (hub#2360).
 *
 * The notification plugin reports it as `actionPerformed` while it is still loading, before the page
 * has a listener, and Tauri drops an event nobody listens to. This plugin keeps the tap and the page
 * claims it once it is up (`takeNoticeTap`). Pure, so it is decided without a device: which launch
 * is such a tap, read from the extras the notification plugin puts on the intent.
 */
object NoticeLaunch {
    /** What the notification plugin writes on the intent (`TauriNotificationManager.buildIntent`). */
    const val ID_KEY = "NotificationId"
    const val USER_ACTION_KEY = "NotificationUserAction"
    const val NOTIFICATION_KEY = "LocalNotficationObject" // sic — the plugin's own spelling

    /** The plugin's «no id on this intent». */
    const val NO_ID = Int.MIN_VALUE

    /** The plugin's action for a tap on the notice itself (not a button, not a swipe). */
    private const val TAP = "tap"

    /** The notice's id and the JSON the plugin stored it as, whose `extra.path` is the screen. */
    data class Tap(val id: Int, val notification: String?) {
        /** What is remembered outside the process: the id alone is not enough, ids restart at every boot. */
        val key: String get() = "$id:${notification?.hashCode() ?: 0}"
    }

    /** What an intent says about a notice, read off it once so the decision needs no device. */
    data class Launch(val action: String?, val flags: Int, val id: Int, val userAction: String?, val notification: String?)

    fun tapOf(launch: Launch, lastKept: String?): Tap? {
        if (launch.action != Intent.ACTION_MAIN || launch.id == NO_ID || launch.userAction != TAP) return null
        // Relaunched from recents: Android hands back the ORIGINAL intent, extras and all.
        if (launch.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return null
        val tap = Tap(launch.id, launch.notification)
        // Back through the icon after the system killed the process, the task's intent is still the
        // tap it was born from, with none of the flags above: only what was remembered stops it.
        return if (tap.key == lastKept) null else tap
    }
}

/** Where the tap the task was born from is remembered, outside the process that kept it. */
interface KeptTapMemory {
    fun last(): String?
    fun remember(key: String)
}

/**
 * The tap the page was not there to hear (hub#2360), until it claims it. Once the page has claimed,
 * it is listening: a tap is the notification plugin's to deliver (hub#2305) and is not kept, or the
 * next boot of the page would open that screen a second time. A new page resets that.
 */
class NoticeTapBox(private val memory: KeptTapMemory) {
    private var kept: NoticeLaunch.Tap? = null
    private var claimed = false

    /**
     * A new page is loading, with the intent the activity was (re)created with. Android hands that
     * intent back every time the process returns, so its tap counts once: it is the one remembered.
     * A tap that reached this process before the plugin loaded is the later one, and stays.
     */
    @Synchronized
    fun pageLoading(launch: NoticeLaunch.Launch) {
        claimed = false
        val born = NoticeLaunch.tapOf(launch, memory.last()) ?: return
        memory.remember(born.key)
        if (kept == null) kept = born
    }

    /**
     * A tap delivered to a live activity — also when the system had killed its process. Android
     * delivers it once (rv-2411, measured), so it needs no memory; remembering it would forget the
     * tap the task was born from, which would then open again.
     */
    @Synchronized
    fun newIntent(launch: NoticeLaunch.Launch) {
        if (claimed) return
        kept = NoticeLaunch.tapOf(launch, lastKept = null) ?: return
    }

    /** Hands the tap over, once; from now on the page is listening. */
    @Synchronized
    fun take(): NoticeLaunch.Tap? {
        claimed = true
        return kept.also { kept = null }
    }
}

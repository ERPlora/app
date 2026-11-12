package com.erplora.android

import android.content.Intent

/**
 * The tap on a notice that STARTED the app.
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

    /**
     * The notice's id, the JSON the plugin stored it as (whose `extra.path` is the screen) and the
     * screen the app wrote down when it showed it ([NoticeTargets]). The plugin never stores that
     * JSON, so `path` is what brings the screen back.
     */
    data class Tap(val id: Int, val notification: String?, val path: String? = null) {
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
 * The tap the page has not heard yet, until it claims it. A tap that STARTS the app, or that
 * reaches it before the page loads, waits for the page's boot. Once the page has claimed, it is
 * listening: a tap whose intent carries its notice is the notification plugin's to deliver, and
 * kept it would open that screen a second time. The plugin never stores the notice on the intent,
 * though (`sourceJson`, tauri-apps/plugins-workspace#3550): its `actionPerformed` then says neither
 * the number nor the screen, and the page stays where it was. Such a tap is kept, with the screen
 * the app wrote down ([pathOf]), and the page is told it waits ([onTapWaiting]). A new page resets
 * that.
 */
class NoticeTapBox(private val memory: KeptTapMemory, private val pathOf: (Int) -> String? = { null }) {
    private var kept: NoticeLaunch.Tap? = null
    private var claimed = false
    private var waiting: (() -> Unit)? = null

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
        if (kept == null) kept = withScreen(born)
    }

    /**
     * A tap delivered to a live activity — also when the system had killed its process. Android
     * delivers it once (measured), so it needs no memory; remembering it would forget the
     * tap the task was born from, which would then open again.
     */
    fun newIntent(launch: NoticeLaunch.Launch) {
        val tell = synchronized(this) {
            val tap = NoticeLaunch.tapOf(launch, lastKept = null) ?: return
            if (claimed && tap.notification != null) return
            kept = withScreen(tap)
            // Told outside the lock: the page answers by claiming it.
            if (claimed) waiting else null
        }
        tell?.invoke()
    }

    /** Who tells the page that a tap waits while it listens (the shell, through Rust). */
    @Synchronized
    fun onTapWaiting(listener: () -> Unit) {
        waiting = listener
    }

    /** Hands the tap over, once; from now on the page is listening. */
    @Synchronized
    fun take(): NoticeLaunch.Tap? {
        claimed = true
        return kept.also { kept = null }
    }

    private fun withScreen(tap: NoticeLaunch.Tap) = tap.copy(path = pathOf(tap.id))
}

/** Where [NoticeTargets] keeps what it wrote down, outside the process. */
interface NoticeTargetsMemory {
    fun read(): String?
    fun write(value: String)
}

/**
 * Which screen each notice the app showed leads to, by the number it went out with: the
 * notification plugin brings that number back on the tap and nothing else. The most recent
 * [REMEMBERED] notices, a number per line followed by its screen; a screen that does not fit on its
 * line is not written down, and a memory that does not read starts again.
 */
class NoticeTargets(private val memory: NoticeTargetsMemory) {
    companion object {
        /** As many as the page remembers for its own taps (`REMEMBERED_TAPS`): a tap is on a recent notice. */
        const val REMEMBERED = 50
        private const val FIELD = '\t'
        private const val LINE = '\n'
    }

    @Synchronized
    fun remember(id: Int, path: String?) {
        val others = read().filterKeys { it != id }
        val all = if (path == null || FIELD in path || LINE in path) others else others + (id to path)
        val kept = all.entries.toList().takeLast(REMEMBERED)
        memory.write(kept.joinToString(LINE.toString()) { (number, screen) -> "$number$FIELD$screen" })
    }

    @Synchronized
    fun pathOf(id: Int): String? = read()[id]

    /** In the order they went out, the oldest first. */
    private fun read(): Map<Int, String> {
        val text = memory.read().orEmpty()
        val targets = LinkedHashMap<Int, String>()
        for (line in text.split(LINE)) {
            if (line.isEmpty()) continue
            val fields = line.split(FIELD)
            val id = fields.first().toIntOrNull()
            if (fields.size != 2 || id == null) return emptyMap()
            targets[id] = fields[1]
        }
        return targets
    }
}

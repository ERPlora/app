package com.erplora.android

import android.content.Intent
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * The tap on a notice that STARTED the app (hub#2360).
 *
 * The notification plugin reports it as `actionPerformed` while it is still loading, before the page
 * has a listener, and Tauri drops an event nobody listens to — so the tap opened the app on its first
 * screen instead of the booking it announced. This plugin keeps it for the page to claim; what is
 * decided here is WHICH launches are such a tap, from the extras the notification plugin puts on it.
 */
class NoticeLaunchTest {

    private val json = """{"id":7,"title":"New booking","extra":{"path":"/m/appointments"}}"""

    private fun launch(
        action: String? = Intent.ACTION_MAIN,
        flags: Int = 0,
        id: Int = 7,
        userAction: String? = "tap",
        notification: String? = json,
    ) = NoticeLaunch.Launch(action, flags, id, userAction, notification)

    @Test
    fun `a tap on a notice that starts the app is kept with its notice`() {
        assertEquals(NoticeLaunch.Tap(7, json), NoticeLaunch.tapOf(launch(), lastKept = null))
    }

    @Test
    fun `a tap whose notice was not stored still opens the app`() {
        assertEquals(NoticeLaunch.Tap(7, null), NoticeLaunch.tapOf(launch(notification = null), lastKept = null))
    }

    @Test
    fun `an ordinary launch is no tap`() {
        assertNull(NoticeLaunch.tapOf(launch(id = NoticeLaunch.NO_ID, userAction = null, notification = null), lastKept = null))
        // No notice id, no notice to open — whatever else the intent carries.
        assertNull(NoticeLaunch.tapOf(launch(id = NoticeLaunch.NO_ID), lastKept = null))
        assertNull(NoticeLaunch.tapOf(launch(action = Intent.ACTION_VIEW), lastKept = null))
    }

    @Test
    fun `a button or a swipe is not a tap on the notice`() {
        assertNull(NoticeLaunch.tapOf(launch(userAction = "dismiss"), lastKept = null))
        assertNull(NoticeLaunch.tapOf(launch(userAction = "reply"), lastKept = null))
        assertNull(NoticeLaunch.tapOf(launch(userAction = null), lastKept = null))
    }

    @Test
    fun `reopening the app from recents does not replay an old tap`() {
        // Android hands the ORIGINAL intent back when the app is relaunched from the recents list,
        // extras and all: without this the booking of this morning would open again every time.
        assertNull(NoticeLaunch.tapOf(launch(flags = Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY), lastKept = null))
    }

    @Test
    fun `the tap the task was born from is not kept again when the process comes back`() {
        // The system killed the process and the person came back through the icon: the activity is
        // restored with the intent the task started with — the old tap, extras and all, and none of
        // the flags that say «from history». Only what was remembered outside the process stops it.
        val first = NoticeLaunch.tapOf(launch(), lastKept = null)!!
        assertNull(NoticeLaunch.tapOf(launch(flags = 0x34000000), lastKept = first.key))
    }

    @Test
    fun `another notice that happens to reuse the id is still a tap`() {
        // Ids restart from the clock at every boot, so two notices of two sessions may share one.
        val old = NoticeLaunch.Tap(7, """{"id":7,"title":"Old order"}""")
        assertEquals(NoticeLaunch.Tap(7, json), NoticeLaunch.tapOf(launch(), lastKept = old.key))
    }
}

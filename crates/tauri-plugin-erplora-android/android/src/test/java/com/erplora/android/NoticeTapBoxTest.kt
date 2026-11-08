package com.erplora.android

import android.content.Intent
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * Where the taps the page was not there to hear wait for it (hub#2360), across the two ways a tap
 * reaches a dead app: the intent the activity is created with, and `onNewIntent` when the system
 * killed the process but kept the task — the activity comes back with its old launcher intent and
 * the tap arrives afterwards, before any plugin is loaded.
 */
class NoticeTapBoxTest {

    private class Memory(var last: String? = null) : KeptTapMemory {
        override fun last(): String? = last
        override fun remember(key: String) {
            last = key
        }
    }

    private fun tap(id: Int, path: String) =
        NoticeLaunch.Launch(Intent.ACTION_MAIN, 0x24000000, id, "tap", """{"id":$id,"extra":{"path":"$path"}}""")

    private val launcher = NoticeLaunch.Launch(Intent.ACTION_MAIN, 0x10000000, NoticeLaunch.NO_ID, null, null)

    /**
     * What the notification plugin really puts on the intent of a tap (hub#1039): the notice's
     * number, and no notice — it never fills `sourceJson` (tauri-apps/plugins-workspace#3550).
     */
    private fun bare(id: Int) = NoticeLaunch.Launch(Intent.ACTION_MAIN, 0x24000000, id, "tap", null)

    /** The screens the app wrote down when it showed notices 7 and 8 ([NoticeTargets]). */
    private val shown = mapOf(7 to "/appointments/calendar", 8 to "/kitchen/display")

    private fun boxOf(memory: KeptTapMemory = Memory(), told: () -> Unit = {}) =
        NoticeTapBox(memory) { shown[it] }.also { it.onTapWaiting(told) }

    @Test
    fun `a tap that starts the app is handed over once`() {
        val box = NoticeTapBox(Memory())
        box.pageLoading(tap(7, "/m/appointments"))
        assertEquals(7, box.take()?.id)
        assertNull(box.take())
    }

    @Test
    fun `a tap that reaches a dead process through onNewIntent is kept for the page`() {
        val box = NoticeTapBox(Memory())
        box.newIntent(tap(7, "/m/appointments"))
        // The plugin loads after it, with the launcher intent the activity was restored with.
        box.pageLoading(launcher)
        assertEquals(7, box.take()?.id)
    }

    @Test
    fun `once the page has claimed, a tap that carries its notice is the notification plugin's to deliver`() {
        var told = 0
        val box = boxOf { told += 1 }
        box.pageLoading(launcher)
        assertNull(box.take())
        box.newIntent(tap(8, "/m/kitchen"))
        // Kept, it would open that screen a second time: the plugin hands this one to the page.
        assertNull(box.take())
        assertEquals(0, told)
    }

    @Test
    fun `a tap the plugin hands over without its notice opens the screen it went out with while the page listens`() {
        var told = 0
        val box = boxOf { told += 1 }
        box.pageLoading(launcher)
        assertNull(box.take())
        // The kitchen's notice, tapped with the app open: the plugin's own `actionPerformed` says
        // neither its number nor its screen, so the page would stay where it was.
        box.newIntent(bare(8))
        assertEquals(1, told, "the page was not told a tap is waiting")
        assertEquals(NoticeLaunch.Tap(8, null, "/kitchen/display"), box.take())
        assertNull(box.take())
    }

    @Test
    fun `a tap without its notice that starts the app opens the screen it went out with`() {
        val box = boxOf()
        box.pageLoading(bare(7))
        assertEquals(NoticeLaunch.Tap(7, null, "/appointments/calendar"), box.take())
    }

    @Test
    fun `a tap without its notice that reaches a dead process waits for the page with its screen`() {
        var told = 0
        val box = boxOf { told += 1 }
        box.newIntent(bare(8))
        box.pageLoading(launcher)
        // Nobody listens yet: the page claims it when it boots.
        assertEquals(0, told)
        assertEquals("/kitchen/display", box.take()?.path)
    }

    @Test
    fun `a tap on a notice the app did not write down still reaches the page`() {
        val box = boxOf()
        box.pageLoading(launcher)
        box.take()
        box.newIntent(bare(9))
        assertEquals(NoticeLaunch.Tap(9, null, null), box.take())
    }

    @Test
    fun `a new page is not there to hear a tap until it claims`() {
        val box = NoticeTapBox(Memory())
        box.pageLoading(launcher)
        box.take()
        box.pageLoading(launcher)
        box.newIntent(tap(9, "/m/kitchen"))
        assertEquals(9, box.take()?.id)
    }

    @Test
    fun `the last tap wins`() {
        val box = NoticeTapBox(Memory())
        box.newIntent(tap(7, "/m/appointments"))
        box.newIntent(tap(8, "/m/kitchen"))
        assertEquals(8, box.take()?.id)
    }

    @Test
    fun `a new process does not keep the tap the last one already kept`() {
        val memory = Memory()
        NoticeTapBox(memory).pageLoading(tap(7, "/m/appointments"))
        // The system killed the process; back through the icon, the task's intent is the old tap.
        val next = NoticeTapBox(memory)
        next.pageLoading(tap(7, "/m/appointments"))
        assertNull(next.take())
    }

    @Test
    fun `a tap through onNewIntent does not take the place of the tap the task was born from`() {
        // Android delivers a new intent once, and hands the intent the task was born from back at
        // every return of the process (rv-2411, measured): only the latter has to be remembered.
        val memory = Memory()
        NoticeTapBox(memory).pageLoading(tap(7, "/m/appointments"))
        NoticeTapBox(memory).newIntent(tap(8, "/m/kitchen"))
        assertEquals(NoticeLaunch.Tap(7, """{"id":7,"extra":{"path":"/m/appointments"}}""").key, memory.last)
    }

    @Test
    fun `a tap that reaches a dead process is not lost to the tap the task was born from`() {
        val memory = Memory()
        // The task was born from a tap on notice 7; that process kept it.
        NoticeTapBox(memory).pageLoading(tap(7, "/m/appointments"))
        // The system killed the process and the person taps notice 8, still in the tray.
        val next = NoticeTapBox(memory)
        next.newIntent(tap(8, "/m/kitchen"))
        // The plugin loads after it, with the intent the task started with: the old tap.
        next.pageLoading(tap(7, "/m/appointments"))
        assertEquals(8, next.take()?.id)
    }

    @Test
    fun `a tap that came in before the plugin loaded is the later one and wins`() {
        val box = NoticeTapBox(Memory())
        box.newIntent(tap(8, "/m/kitchen"))
        box.pageLoading(tap(7, "/m/appointments"))
        assertEquals(8, box.take()?.id)
    }

    @Test
    fun `a notice still in the tray opens its screen when it is tapped again`() {
        val memory = Memory()
        NoticeTapBox(memory).pageLoading(tap(7, "/m/appointments"))
        val next = NoticeTapBox(memory)
        next.newIntent(tap(7, "/m/appointments"))
        next.pageLoading(tap(7, "/m/appointments"))
        assertEquals(7, next.take()?.id)
    }

    @Test
    fun `the tap the task was born from stays old after later taps were kept`() {
        val memory = Memory()
        NoticeTapBox(memory).pageLoading(tap(7, "/m/appointments"))
        val second = NoticeTapBox(memory)
        second.newIntent(tap(8, "/m/kitchen"))
        second.pageLoading(tap(7, "/m/appointments"))
        second.take()
        // Killed again, back through the icon: the task's intent is still the tap on notice 7.
        val third = NoticeTapBox(memory)
        third.pageLoading(tap(7, "/m/appointments"))
        assertNull(third.take())
    }
}

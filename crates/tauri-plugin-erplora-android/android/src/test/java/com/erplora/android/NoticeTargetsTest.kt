package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * Which screen each notice the app showed leads to (hub#1039).
 *
 * The notification plugin never stores the notice on the intent of its tap (`sourceJson` is never
 * set, tauri-apps/plugins-workspace#3550): the tap comes back with the notice's number and nothing
 * else. So the app writes the screen down when it shows the notice, in the device, where a process
 * started by the tap still finds it.
 */
class NoticeTargetsTest {

    private class Memory(var value: String? = null) : NoticeTargetsMemory {
        override fun read(): String? = value
        override fun write(value: String) {
            this.value = value
        }
    }

    @Test
    fun `each notice leads to the screen it went out with`() {
        val targets = NoticeTargets(Memory())
        targets.remember(7, "/kitchen/display")
        targets.remember(8, "/appointments/calendar?date=2026-11-08")
        assertEquals("/kitchen/display", targets.pathOf(7))
        assertEquals("/appointments/calendar?date=2026-11-08", targets.pathOf(8))
        assertNull(targets.pathOf(9))
    }

    @Test
    fun `a process started by the tap reads what the last one wrote down`() {
        val memory = Memory()
        NoticeTargets(memory).remember(7, "/kitchen/display")
        assertEquals("/kitchen/display", NoticeTargets(memory).pathOf(7))
    }

    @Test
    fun `a number that goes out again leads to its new screen`() {
        val targets = NoticeTargets(Memory())
        targets.remember(7, "/kitchen/display")
        targets.remember(7, "/appointments/calendar")
        assertEquals("/appointments/calendar", targets.pathOf(7))
    }

    @Test
    fun `a notice without a screen forgets the one its number led to before`() {
        val targets = NoticeTargets(Memory())
        targets.remember(7, "/kitchen/display")
        targets.remember(7, null)
        assertNull(targets.pathOf(7))
    }

    @Test
    fun `only the most recent notices are remembered`() {
        val targets = NoticeTargets(Memory())
        for (id in 1..NoticeTargets.REMEMBERED + 1) targets.remember(id, "/kitchen/display")
        assertNull(targets.pathOf(1), "the oldest notice should have been forgotten")
        assertEquals("/kitchen/display", targets.pathOf(2))
        assertEquals("/kitchen/display", targets.pathOf(NoticeTargets.REMEMBERED + 1))
    }

    @Test
    fun `a memory that does not read is started again`() {
        val memory = Memory("not\twhat\nwe\twrote\tdown")
        val targets = NoticeTargets(memory)
        assertNull(targets.pathOf(7))
        targets.remember(7, "/kitchen/display")
        assertEquals("/kitchen/display", NoticeTargets(memory).pathOf(7))
    }

    @Test
    fun `a screen that cannot be written down is not remembered`() {
        val targets = NoticeTargets(Memory())
        targets.remember(7, "/kitchen/display\n8\t/elsewhere")
        assertNull(targets.pathOf(7))
        assertNull(targets.pathOf(8))
    }
}

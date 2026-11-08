package com.erplora.android

import android.view.WindowManager
import kotlin.test.Test
import kotlin.test.assertEquals

class ScreenOnTest {
    @Test
    fun `applies Android's keep-screen-on flag to the visible window`() {
        var appliedFlags = 0

        ScreenOn.apply { flags -> appliedFlags = appliedFlags or flags }

        assertEquals(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON, appliedFlags)
    }
}

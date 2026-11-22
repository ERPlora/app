package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * What can be decided without a device: which addresses a tab of the browser opens. Showing the tab
 * takes a real Android with a browser.
 */
class CustomTabTest {

    @Test
    fun `a page of the web opens in a tab`() {
        assertTrue(CustomTab.isWebPage("https://erplora.com/auth/app/google/?challenge=C"))
        // The SaaS of a development machine.
        assertTrue(CustomTab.isWebPage("http://127.0.0.1:8001/auth/app/google/"))
    }

    @Test
    fun `nothing but a page of the web opens in a tab`() {
        for (address in listOf(null, "", "erplora://signed-in?code=x", "intent://scan/#Intent;end", "file:///sdcard/x", "javascript:alert(1)")) {
            assertFalse(CustomTab.isWebPage(address), "$address")
        }
    }
}

package com.erplora.android

import android.content.Context
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.fail

/**
 * The app's window hands nothing to Android's autofill service (app#15).
 *
 * QA, 2026-10-30, tablet with Google Password Manager: saving a reservation in the hub popped
 * «Save password to Google Password Manager?» with the CUSTOMER's e-mail as the user and their
 * notes («gluten free») as the password. The pages of a business carry customers' data, never the
 * credentials of whoever holds the tablet, so none of it may reach the service.
 *
 * The page cannot stop it (the e-mail already said `autocomplete="off"`), and neither can
 * `importantForAutofill` on the WebView: Chromium's `AutofillProvider` never reads it, and
 * Android 14+ puts a WebView in the autofill structure whatever its importance
 * (`always_include_webview_in_assist_structure`, on by default). What Chromium does read is the
 * `AutofillManager` its context hands out — none, and it treats autofill as off for good.
 */
class AutofillOptOutTest {

    /**
     * `"autofill"` is the name `getSystemServiceName(AutofillManager::class.java)` answers and the one
     * `getSystemService(AutofillManager::class.java)` asks for, which is how Chromium gets it. The SDK
     * hides the constant (`Context.AUTOFILL_MANAGER_SERVICE` is `@hide`), so the name is spelled out.
     */
    @Test
    fun `the window has no autofill manager`() {
        assertNull(
            AutofillOptOut.systemService("autofill") {
                fail("the platform's AutofillManager was created: the WebView would hand it the hub's forms")
            },
        )
    }

    @Test
    fun `it refuses the name every caller asks for`() {
        assertEquals("autofill", AutofillOptOut.AUTOFILL_SERVICE)
    }

    @Test
    fun `every other service is the platform's`() {
        val keyboard = Any()
        assertEquals(
            keyboard,
            AutofillOptOut.systemService(Context.INPUT_METHOD_SERVICE) { name ->
                assertEquals(Context.INPUT_METHOD_SERVICE, name)
                keyboard
            },
            "without the keyboard nobody can type in the hub",
        )
        val windows = Any()
        assertEquals(windows, AutofillOptOut.systemService(Context.WINDOW_SERVICE) { windows })
    }

    @Test
    fun `a service the platform does not have stays missing`() {
        assertNull(AutofillOptOut.systemService("not-a-service") { null })
    }
}

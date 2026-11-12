package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Which permissions have to be ASKED for at runtime, by Android version.
 *
 * Declaring them in the manifest is not enough, and the two the till needs fail **silently**:
 *
 *  - Without `POST_NOTIFICATIONS` (API 33+) the notification does not show. No exception, no error:
 *    the order reaches the kitchen and nobody notices.
 *  - Without `ACCESS_LOCAL_NETWORK` (API 37+) ALL traffic to the LAN is blocked, below the API. The
 *    sweep of 9100 is 254 **timeouts**, not errors — impossible to tell from «this venue has no
 *    printers». Verified on the API 37 emulator: `discover_printers` returned `[]` without the
 *    slightest complaint until the permission was granted by hand with `adb`.
 *
 * And asking for a permission the version does not know is worse than not asking: on some
 * manufacturers it leaves the dialog hanging. That is why the list is COMPUTED, not written out.
 */
class PermissionPolicyTest {

    @Test
    fun `Android 17 needs both`() {
        val permisos = PermissionPolicy.required(sdkInt = 37)
        assertTrue(PermissionPolicy.POST_NOTIFICATIONS in permisos)
        assertTrue(PermissionPolicy.ACCESS_LOCAL_NETWORK in permisos)
    }

    @Test
    fun `on Android 16 the local network comes with INTERNET and is NOT asked for`() {
        val permisos = PermissionPolicy.required(sdkInt = 36)
        assertTrue(PermissionPolicy.POST_NOTIFICATIONS in permisos)
        assertFalse(
            PermissionPolicy.ACCESS_LOCAL_NETWORK in permisos,
            "asking for a permission the version does not know can hang the dialog",
        )
    }

    @Test
    fun `before Android 13 notifications are granted and not asked for`() {
        val permisos = PermissionPolicy.required(sdkInt = 32)
        assertFalse(PermissionPolicy.POST_NOTIFICATIONS in permisos)
    }

    @Test
    fun `only what the app will use can be asked for`() {
        // A screen-only deployment (a KDS with no printer) has no reason to ask for the local
        // network, and a permission asked for without being used is an easy «no» from the user.
        assertEquals(
            listOf(PermissionPolicy.POST_NOTIFICATIONS),
            PermissionPolicy.required(sdkInt = 37, localNetwork = false, bluetooth = false),
        )
        assertEquals(
            listOf(PermissionPolicy.ACCESS_LOCAL_NETWORK),
            PermissionPolicy.required(sdkInt = 37, notifications = false, bluetooth = false),
        )
    }

    @Test
    fun `with nothing to use the user is not bothered`() {
        assertEquals(
            emptyList(),
            PermissionPolicy.required(
                sdkInt = 37,
                notifications = false,
                localNetwork = false,
                bluetooth = false,
            ),
        )
    }

    @Test
    fun `the order is stable — notifications, local network, bluetooth`() {
        // The user sees the dialogs in this order; having it change between versions is confusing.
        assertEquals(
            listOf(
                PermissionPolicy.POST_NOTIFICATIONS,
                PermissionPolicy.ACCESS_LOCAL_NETWORK,
                PermissionPolicy.BLUETOOTH_CONNECT,
            ),
            PermissionPolicy.required(sdkInt = 37),
        )
    }

    // ── The SPP transport needs BLUETOOTH_CONNECT on Android 12+ ──────────────────────────────

    @Test
    fun `bluetooth connect is required from Android 12 on`() {
        // Talking to a bonded RFCOMM device — listing it, connecting to it — is gated by
        // BLUETOOTH_CONNECT since API 31, and like the other two it fails SILENTLY without it:
        // the bonded list comes back empty and the till reports no bluetooth printers.
        assertTrue(PermissionPolicy.BLUETOOTH_CONNECT in PermissionPolicy.required(sdkInt = 31))
        assertTrue(PermissionPolicy.BLUETOOTH_CONNECT in PermissionPolicy.required(sdkInt = 37))
    }

    @Test
    fun `before Android 12 bluetooth rides the install-time permission and is not asked`() {
        // Pre-31 the legacy BLUETOOTH permission is granted at install; requesting a runtime
        // permission the version does not know can hang the dialog on some vendors.
        assertFalse(PermissionPolicy.BLUETOOTH_CONNECT in PermissionPolicy.required(sdkInt = 30))
    }

    @Test
    fun `a bluetooth-only scope does not drag the other dialogs in`() {
        // Permissions are asked in context: printing to a bluetooth printer has nothing to do with
        // the LAN or with notifications.
        assertEquals(
            listOf(PermissionPolicy.BLUETOOTH_CONNECT),
            PermissionPolicy.required(sdkInt = 37, notifications = false, localNetwork = false),
        )
    }
}

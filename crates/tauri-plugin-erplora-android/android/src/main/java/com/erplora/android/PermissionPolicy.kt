package com.erplora.android

import android.os.Build

/**
 * Which permissions have to be ASKED for at runtime, by Android version.
 *
 * Declaring them in the manifest is not enough: since Android 6 the dangerous ones are granted at
 * runtime, and the two the till needs came late and fail **silently**:
 *
 *  - `POST_NOTIFICATIONS` (API 33): without it the notification does not show. No exception, no
 *    error — the order reaches the kitchen and nobody notices.
 *  - `ACCESS_LOCAL_NETWORK` (API 37): blocks **all** traffic to the LAN, below the API (it reaches
 *    raw sockets, not only mDNS). Without it, the sweep of port 9100 is 254 **timeouts**, not
 *    errors: impossible to tell from «this venue has no printers». Verified on the API 37 emulator
 *    — `discover_printers` returned `[]` without the slightest complaint.
 *
 * Asking for a permission the version does not know is worse than not asking: on some
 * manufacturers it leaves the dialog hanging. That is why the list is COMPUTED, not written out.
 *
 * Pure logic except for the version constant, so it can be tested on the JVM without an emulator.
 */
object PermissionPolicy {

    const val POST_NOTIFICATIONS = "android.permission.POST_NOTIFICATIONS"
    const val ACCESS_LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
    const val BLUETOOTH_CONNECT = "android.permission.BLUETOOTH_CONNECT"

    /** Android 13 — before it, notifications are granted out of the box. */
    const val SDK_NOTIFICATIONS = 33

    /** Android 17 — before it, the local network comes with `INTERNET`. */
    const val SDK_LOCAL_NETWORK = 37

    /**
     * Android 12 — before, talking to bonded devices rides the install-time `BLUETOOTH`
     * permission. From API 31 on, listing or connecting to a bonded RFCOMM printer without
     * `BLUETOOTH_CONNECT` fails the same silent way as the other two: an empty bonded list and a
     * till that reports no bluetooth printers.
     */
    const val SDK_BLUETOOTH_CONNECT = 31

    /**
     * Permissions to ask for on a device at API level [sdkInt].
     *
     * The order is stable — notifications, local network, bluetooth — because it is the order in
     * which the user sees the dialogs, and having it change between versions is confusing.
     *
     * @param notifications whether the app will show system notifications.
     * @param localNetwork whether the app will talk to devices on the local network (printers).
     * @param bluetooth whether the app will talk to bonded Bluetooth SPP printers.
     */
    @JvmStatic
    @JvmOverloads
    fun required(
        sdkInt: Int = Build.VERSION.SDK_INT,
        notifications: Boolean = true,
        localNetwork: Boolean = true,
        bluetooth: Boolean = true,
    ): List<String> {
        val permisos = mutableListOf<String>()
        if (notifications && sdkInt >= SDK_NOTIFICATIONS) permisos += POST_NOTIFICATIONS
        if (localNetwork && sdkInt >= SDK_LOCAL_NETWORK) permisos += ACCESS_LOCAL_NETWORK
        if (bluetooth && sdkInt >= SDK_BLUETOOTH_CONNECT) permisos += BLUETOOTH_CONNECT
        return permisos
    }
}

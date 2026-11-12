package com.erplora.android

import app.tauri.annotation.TauriPlugin
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * The plugin is glue: it takes the `invoke`, asks the system and answers. Every decision lives in
 * pure functions of its companion, and those are what is tested here — without an emulator.
 *
 * What is pinned:
 *  - what is really asked for (what is missing, not everything: a dialog per scan would be
 *    unbearable);
 *  - what is answered (the REAL state of each permission, not what the user just tapped);
 *  - that a «no» from the user is NOT an error: the app has to be able to go on without a printer;
 *  - that the annotation declares EVERYTHING the policy can ask for (see the test, it is a trap).
 */
class ErploraAndroidPluginTest {

    private val NOTIF = PermissionPolicy.POST_NOTIFICATIONS
    private val RED = PermissionPolicy.ACCESS_LOCAL_NETWORK
    private val BT = PermissionPolicy.BLUETOOTH_CONNECT

    @Test
    fun `only what is missing is asked for`() {
        val faltan = ErploraAndroidPlugin.pendingOf(
            required = listOf(NOTIF, RED),
            granted = setOf(NOTIF),
        )
        assertEquals(listOf(RED), faltan)
    }

    @Test
    fun `when everything is already granted nothing is asked for`() {
        // The web app calls this BEFORE every discovery. If it always asked, the user would see a
        // dialog per scan.
        val faltan = ErploraAndroidPlugin.pendingOf(
            required = listOf(NOTIF, RED),
            granted = setOf(NOTIF, RED),
        )
        assertTrue(faltan.isEmpty())
    }

    @Test
    fun `the result reports the REAL state of every permission`() {
        // The user can grant one and deny another. The web app needs to know exactly what it can
        // do: notify yes, look for printers no.
        val estado = ErploraAndroidPlugin.statusOf(
            required = listOf(NOTIF, RED),
            granted = setOf(NOTIF),
        )
        assertEquals(mapOf(NOTIF to true, RED to false), estado)
    }

    @Test
    fun `a permission this Android version does not know is left out of the result`() {
        // It can be neither granted nor denied: reporting it would be a lie.
        //
        // What is pinned is the FILTERING by API level, not how many permissions there are: on 36
        // the policy knows notifications (33) and Bluetooth (31), but NOT the local network, which
        // arrives in 37. The expected map is written out in full on purpose — so adding a
        // permission to the policy forces saying in which version it exists instead of slipping in
        // here unnoticed.
        val estado = ErploraAndroidPlugin.statusOf(
            required = PermissionPolicy.required(sdkInt = 36),
            granted = setOf(NOTIF),
        )
        assertEquals(mapOf(NOTIF to true, BT to false), estado)
        assertTrue(RED !in estado, "ACCESS_LOCAL_NETWORK arrives in API 37: nothing can be said about it on 36")
    }

    @Test
    fun `the annotation declares everything the policy can ask for`() {
        // Tauri asks for permissions by the ALIAS declared in `@TauriPlugin`: it resolves the alias
        // to its strings and, if it finds none, `requestPermissionForAliases` **does nothing** — no
        // dialog, no exception, and the `invoke` hangs FOREVER: the system grants the permissions
        // and the web app's promise never resolves. Adding a permission to the policy and
        // forgetting it here would bring that back silently, so a machine checks it.
        val declarados = ErploraAndroidPlugin::class.java
            .getAnnotation(TauriPlugin::class.java)!!
            .permissions
            .flatMap { it.strings.toList() }
            .toSet()

        // Every API level the policy tells apart, not only the emulator's at hand. The list grows
        // with every new threshold: if one is missing, a permission that only shows up below the
        // next threshold would go unchecked and the trap would be back.
        val posibles = listOf(
            1,
            PermissionPolicy.SDK_BLUETOOTH_CONNECT,
            PermissionPolicy.SDK_NOTIFICATIONS,
            PermissionPolicy.SDK_LOCAL_NETWORK,
        )
            .flatMap { PermissionPolicy.required(sdkInt = it) }
            .toSet()

        assertEquals(emptySet(), posibles - declarados, "permissions the policy asks for and the annotation does NOT declare")
    }

    @Test
    fun `every permission is declared with its alias equal to its string`() {
        // The alias IS the permission string: so what `check_permissions` returns reads the same as
        // what `dumpsys` says, and there is no second naming to keep in step.
        val perms = ErploraAndroidPlugin::class.java.getAnnotation(TauriPlugin::class.java)!!.permissions
        assertTrue(perms.isNotEmpty(), "with no declared permission nothing can be asked for")
        for (p in perms) {
            assertEquals(listOf(p.alias), p.strings.toList(), "the alias and the string must match")
        }
    }

    // ── A request carries the SCOPE of the operation that makes it ───────────────────────────
    //
    // Asked without one, the plugin requests its whole batch: tapping «Re-scan» would pop the
    // local-network dialog and then, with no visible relation to anything the user just did, the
    // notifications one. An out-of-context permission reads as opportunistic and gets denied —
    // and a denied POST_NOTIFICATIONS is a kitchen that stops hearing orders.

    @Test
    fun `a scoped request asks only for what the operation needs`() {
        assertEquals(
            listOf(RED),
            ErploraAndroidPlugin.requestScope(
                required = listOf(NOTIF, RED),
                requested = listOf(RED),
            ),
        )
    }

    @Test
    fun `a request without a scope keeps asking for everything — an older web must not break`() {
        // The web app is served by the hub and the plugin ships inside the installed binary: they
        // CAN be out of step. A web that predates the scope sends none, and gets the whole batch.
        assertEquals(
            listOf(NOTIF, RED),
            ErploraAndroidPlugin.requestScope(required = listOf(NOTIF, RED), requested = null),
        )
    }

    @Test
    fun `the scope cannot smuggle in a permission this Android does not require`() {
        // On API 36 the local network permission does not exist; requesting it can hang the
        // dialog on some vendors. The policy stays the single authority on what CAN be asked.
        assertEquals(
            emptyList(),
            ErploraAndroidPlugin.requestScope(
                required = PermissionPolicy.required(sdkInt = 36, notifications = false),
                requested = listOf(RED),
            ),
        )
    }
}

package com.erplora.android

import app.tauri.annotation.TauriPlugin
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * El plugin es pegamento: recibe el `invoke`, pregunta al sistema y responde. Toda decisión vive
 * en funciones puras de su companion, que son las que se prueban aquí — sin emulador.
 *
 * Lo que se fija:
 *  - qué se pide de verdad (lo que falta, no todo: un diálogo por escaneo sería insufrible);
 *  - qué se responde (el estado REAL de cada permiso, no lo que el usuario acaba de tocar);
 *  - que un «no» del usuario NO es un error: la app tiene que poder seguir sin impresora;
 *  - que la anotación declara TODO lo que la política puede pedir (ver el test, es una trampa).
 */
class ErploraAndroidPluginTest {

    private val NOTIF = PermissionPolicy.POST_NOTIFICATIONS
    private val RED = PermissionPolicy.ACCESS_LOCAL_NETWORK
    private val BT = PermissionPolicy.BLUETOOTH_CONNECT

    @Test
    fun `solo se pide lo que falta`() {
        val faltan = ErploraAndroidPlugin.pendingOf(
            required = listOf(NOTIF, RED),
            granted = setOf(NOTIF),
        )
        assertEquals(listOf(RED), faltan)
    }

    @Test
    fun `si ya esta todo concedido no se pide nada`() {
        // La PWA llama a esto ANTES de cada descubrimiento. Si pidiera siempre, el usuario vería
        // un diálogo por escaneo.
        val faltan = ErploraAndroidPlugin.pendingOf(
            required = listOf(NOTIF, RED),
            granted = setOf(NOTIF, RED),
        )
        assertTrue(faltan.isEmpty())
    }

    @Test
    fun `el resultado informa del estado REAL de cada permiso`() {
        // El usuario puede conceder uno y denegar otro. La PWA necesita saber exactamente qué
        // puede hacer: avisar sí, buscar impresoras no.
        val estado = ErploraAndroidPlugin.statusOf(
            required = listOf(NOTIF, RED),
            granted = setOf(NOTIF),
        )
        assertEquals(mapOf(NOTIF to true, RED to false), estado)
    }

    @Test
    fun `un permiso que la version de Android no conoce no aparece en el resultado`() {
        // No se puede conceder ni denegar: informar de él sería mentir.
        //
        // Lo que se fija es el FILTRADO por nivel de API, no cuántos permisos hay: en la 36 la
        // política conoce las notificaciones (33) y el Bluetooth (31), pero NO la red local, que
        // llega en la 37. El esperado se escribe entero a propósito — así, añadir un permiso a la
        // política obliga a decir en qué versión existe en vez de colarse aquí sin mirar.
        val estado = ErploraAndroidPlugin.statusOf(
            required = PermissionPolicy.required(sdkInt = 36),
            granted = setOf(NOTIF),
        )
        assertEquals(mapOf(NOTIF to true, BT to false), estado)
        assertTrue(RED !in estado, "ACCESS_LOCAL_NETWORK llega en la API 37: en la 36 no se puede afirmar nada de él")
    }

    @Test
    fun `la anotacion declara todo lo que la politica puede pedir`() {
        // Tauri pide los permisos por ALIAS declarado en `@TauriPlugin`: resuelve el alias a sus
        // strings y, si no encuentra ninguno, `requestPermissionForAliases` **no hace nada** — ni
        // diálogo ni excepción, y el `invoke` queda colgado PARA SIEMPRE.
        //
        // Es exactamente el fallo que costó un ciclo entero de emulador: el sistema concedía los
        // dos permisos y la promesa de la PWA no resolvía jamás. Añadir un permiso a la política y
        // olvidarlo aquí lo repetiría en silencio, así que se comprueba a máquina.
        val declarados = ErploraAndroidPlugin::class.java
            .getAnnotation(TauriPlugin::class.java)!!
            .permissions
            .flatMap { it.strings.toList() }
            .toSet()

        // Todos los niveles de API que la política distingue, no solo el del emulador de turno.
        // La lista se amplía con cada umbral nuevo: si falta uno, el permiso que solo aparece por
        // debajo del siguiente umbral se quedaría sin comprobar y la trampa volvería (hub#933).
        val posibles = listOf(
            1,
            PermissionPolicy.SDK_BLUETOOTH_CONNECT,
            PermissionPolicy.SDK_NOTIFICATIONS,
            PermissionPolicy.SDK_LOCAL_NETWORK,
        )
            .flatMap { PermissionPolicy.required(sdkInt = it) }
            .toSet()

        assertEquals(emptySet(), posibles - declarados, "permisos que la política pide y la anotación NO declara")
    }

    @Test
    fun `cada permiso se declara con su alias igual al string`() {
        // El alias ES el string del permiso: así lo que devuelve `check_permissions` se lee igual
        // que lo que dice `dumpsys`, y no hay una segunda nomenclatura que mantener.
        val perms = ErploraAndroidPlugin::class.java.getAnnotation(TauriPlugin::class.java)!!.permissions
        assertTrue(perms.isNotEmpty(), "sin permisos declarados no se puede pedir nada")
        for (p in perms) {
            assertEquals(listOf(p.alias), p.strings.toList(), "alias y string deben coincidir")
        }
    }

    // ── hub#758: a request carries the SCOPE of the operation that makes it ──────────────────
    //
    // Asked without one, the plugin requested its whole batch: tapping «Re-scan» popped the
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
        // CAN be out of step. A web that predates the scope sends none, and gets the old batch.
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

    // Hubo aquí un test del `REQUEST_CODE` propio del plugin. Se retiró con el mecanismo que
    // fijaba: pedir por `activity.requestPermissions` deja la respuesta en la Activity de Tauri,
    // que no sabe devolvérsela al plugin — el sistema concedía los permisos y el `invoke` no
    // resolvía JAMÁS. Ahora se pide por `requestPermissionForAliases` y el código de solicitud es
    // de Tauri, no nuestro. Lo que protege de la recaída es el test de la anotación, de arriba.
}

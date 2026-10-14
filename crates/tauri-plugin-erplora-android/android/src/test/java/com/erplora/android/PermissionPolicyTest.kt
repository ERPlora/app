package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Qué permisos hay que PEDIR en runtime, por versión de Android.
 *
 * Declararlos en el manifest no basta, y los dos que necesita el TPV fallan **en silencio**:
 *
 *  - Sin `POST_NOTIFICATIONS` (API 33+) la notificación no aparece. Sin excepción ni error: la
 *    comanda entra en cocina y nadie se entera.
 *  - Sin `ACCESS_LOCAL_NETWORK` (API 37+) se bloquea TODO el tráfico a la LAN, por debajo de la
 *    API. El barrido del 9100 son 254 **timeouts**, no errores — indistinguible de «este local no
 *    tiene impresoras». Verificado en el emulador API 37: `discover_printers` devolvía `[]` sin la
 *    menor queja hasta que se concedió el permiso a mano con `adb`.
 *
 * Y pedir un permiso que la versión no conoce es peor que no pedirlo: en algunos fabricantes deja
 * el diálogo colgado. Por eso la lista se CALCULA y no se escribe fija.
 */
class PermissionPolicyTest {

    @Test
    fun `en Android 17 hacen falta los dos`() {
        val permisos = PermissionPolicy.required(sdkInt = 37)
        assertTrue(PermissionPolicy.POST_NOTIFICATIONS in permisos)
        assertTrue(PermissionPolicy.ACCESS_LOCAL_NETWORK in permisos)
    }

    @Test
    fun `en Android 16 la red local va implicita en INTERNET y NO se pide`() {
        val permisos = PermissionPolicy.required(sdkInt = 36)
        assertTrue(PermissionPolicy.POST_NOTIFICATIONS in permisos)
        assertFalse(
            PermissionPolicy.ACCESS_LOCAL_NETWORK in permisos,
            "pedir un permiso que la versión no conoce puede colgar el diálogo",
        )
    }

    @Test
    fun `antes de Android 13 las notificaciones van concedidas y no se piden`() {
        val permisos = PermissionPolicy.required(sdkInt = 32)
        assertFalse(PermissionPolicy.POST_NOTIFICATIONS in permisos)
    }

    @Test
    fun `se puede pedir solo lo que la app va a usar`() {
        // Un despliegue solo-pantalla (KDS sin impresora) no tiene por qué pedir la red local, y
        // un permiso que se pide sin usarlo es un «no» fácil del usuario.
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
    fun `sin nada que usar no se molesta al usuario`() {
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
    fun `el orden es estable — notificaciones, red local, bluetooth`() {
        // El usuario ve los diálogos en este orden; que cambie entre versiones desconcierta.
        assertEquals(
            listOf(
                PermissionPolicy.POST_NOTIFICATIONS,
                PermissionPolicy.ACCESS_LOCAL_NETWORK,
                PermissionPolicy.BLUETOOTH_CONNECT,
            ),
            PermissionPolicy.required(sdkInt = 37),
        )
    }

    // ── ADR-0204 / hub#388: the SPP transport needs BLUETOOTH_CONNECT on Android 12+ ─────────

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
        // ADR-0204 point 4: permissions in context. Printing to a bluetooth printer has nothing
        // to do with the LAN or with notifications.
        assertEquals(
            listOf(PermissionPolicy.BLUETOOTH_CONNECT),
            PermissionPolicy.required(sdkInt = 37, notifications = false, localNetwork = false),
        )
    }
}

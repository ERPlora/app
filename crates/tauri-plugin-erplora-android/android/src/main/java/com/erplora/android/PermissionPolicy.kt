package com.erplora.android

import android.os.Build

/**
 * Qué permisos hay que PEDIR en runtime según la versión de Android.
 *
 * Declararlos en el manifest no basta: desde Android 6 los peligrosos se conceden en runtime, y
 * los dos que necesita el TPV llegaron tarde y fallan **en silencio**:
 *
 *  - `POST_NOTIFICATIONS` (API 33): sin él la notificación no aparece. No hay excepción ni error —
 *    la comanda entra en cocina y nadie se entera.
 *  - `ACCESS_LOCAL_NETWORK` (API 37): bloquea **todo** el tráfico a la LAN, y por debajo de la API
 *    (alcanza a los sockets crudos, no solo a mDNS). Sin él, el barrido del puerto 9100 son 254
 *    **timeouts**, no errores: indistinguible de «este local no tiene impresoras». Verificado en el
 *    emulador API 37 — `discover_printers` devolvía `[]` sin la menor queja.
 *
 * Pedir un permiso que la versión no conoce es peor que no pedirlo: en algunos fabricantes deja el
 * diálogo colgado. Por eso la lista se CALCULA y no se escribe fija.
 *
 * Lógica pura salvo la constante de versión, para poder probarla en la JVM sin emulador.
 */
object PermissionPolicy {

    const val POST_NOTIFICATIONS = "android.permission.POST_NOTIFICATIONS"
    const val ACCESS_LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
    const val BLUETOOTH_CONNECT = "android.permission.BLUETOOTH_CONNECT"

    /** Android 13 — antes, las notificaciones van concedidas de fábrica. */
    const val SDK_NOTIFICATIONS = 33

    /** Android 17 — antes, la red local va implícita en `INTERNET`. */
    const val SDK_LOCAL_NETWORK = 37

    /**
     * Android 12 — before, talking to bonded devices rides the install-time `BLUETOOTH`
     * permission. From API 31 on, listing or connecting to a bonded RFCOMM printer without
     * `BLUETOOTH_CONNECT` fails the same silent way as the other two: an empty bonded list and a
     * till that reports no bluetooth printers (ADR-0204, hub#388).
     */
    const val SDK_BLUETOOTH_CONNECT = 31

    /**
     * Permisos a solicitar en un dispositivo con nivel de API [sdkInt].
     *
     * El orden es estable —notificaciones, red local, bluetooth— porque es el orden en que el
     * usuario ve los diálogos, y que cambie entre versiones desconcierta.
     *
     * @param notifications si la app va a mostrar notificaciones del sistema.
     * @param localNetwork si la app va a hablar con dispositivos de la red local (impresoras).
     * @param bluetooth si la app va a hablar con impresoras Bluetooth SPP emparejadas (ADR-0204).
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

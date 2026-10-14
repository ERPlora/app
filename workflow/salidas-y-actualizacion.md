# WORKFLOW — La aplicación instalada · Salidas y actualización

Prefijo: HUB_APP

## Flujos

### HUB_APP-F27 Arrancar con el ordenador
Estado: hecho
Actor: administrador, responsable
Pantalla: HUB_SHELL: Ajustes
Pasos:
1. En un ordenador (no en Android), en **Ajustes › Hub** aparece «Arrancar al iniciar sesión»; está
   **apagado** de fábrica.
2. Al activarlo, la aplicación lo registra en el sistema (macOS, agente de arranque; Windows, clave de
   registro; Linux, carpeta de autoarranque) y vuelve a leer qué dice el sistema.
3. La pantalla enseña lo que el sistema respondió, no lo que se pidió.
Entra: activar o desactivar.
Sale: la entrada del sistema; la aplicación no guarda estado propio.
Si falla: un fallo al registrar se propaga y la pantalla dice «No se pudo cambiar el ajuste de arranque al
iniciar sesión». En Android la orden rechaza a propósito y el control no se pinta. Arrancar con el
ordenador es para que siempre haya un puesto que saque la cola; abrir el equipo no garantiza que haya
sesión iniciada en el hub.
Implicados: HUB_SHELL-F163
QA: ninguno

### HUB_APP-F28 Botón Atrás de Android
Estado: hecho
Actor: empleado
Pantalla: Ventana de ERPlora
Pasos:
1. La página toma el botón Atrás del sistema.
2. Cierra primero lo de encima: un diálogo, una hoja, el menú lateral o lo que el módulo abierto declare;
   uno que no se puede cerrar retiene la pulsación.
3. Sin nada abierto y con historial, retrocede; sin historial, manda la aplicación al fondo (no la cierra
   ni la mata).
Entra: la pulsación.
Sale: la aplicación en segundo plano.
Si falla: una aplicación más antigua que la orden devuelve el botón a Tauri, que sale por sí misma. Una página
que no es el negocio enlazado no puede mandar la aplicación al fondo: recibe `not_the_linked_hub`
(HUB_APP-F10, ERPlora/hub#2642).
Implicados: HUB_SHELL-F15
QA: ninguno

### HUB_APP-F29 Abrir un enlace fuera de la aplicación
Estado: hecho
Actor: administrador, responsable
Pantalla: ninguna
Pasos:
1. Un botón que cobra o lleva a la cuenta (comprar un módulo, planes, portal de facturación, la descarga
   de la actualización) pide abrir una dirección. En la copia de Play los controles de comprar o cambiar de plan
   no se pintan (`upgrade-plan-link.ts:74-76`); la salida al navegador desde Play no la vigila la aplicación
   (hub#1918).
2. La aplicación comprueba la dirección y la entrega al **navegador del sistema** (no a una pestaña
   propia): la caja se queda como estaba mientras se paga.
3. La persona vuelve a la aplicación, que sigue donde estaba.
Entra: una dirección https de erplora.com o de un negocio, la de pago `checkout.stripe.com` (solo ese host),
o http a un bucle local de desarrollo.
Sale: el navegador abierto; nada guardado.
Si falla: una dirección que no es de las anteriores, con usuario o contraseña, o de otro esquema
(`file:`, `javascript:`) se rechaza (`external_url_refused`); sin navegador instalado, o que dice que no
(`external_url_unavailable`). La página convierte ambos en un aviso: un botón que no hace nada es el
defecto que esto evita. Desde un navegador normal es una pestaña nueva.
Implicados: HUB_SHELL-F16, HUB_SHELL-F48, HUB_SHELL-F129
QA: qa-hub-android Fase 4

### HUB_APP-F30 Guardar una descarga
Estado: parcial — iOS la rechaza y Android 9 o anterior no tiene dónde guardar (la frase de la pantalla habla de «un móvil o una tablet» en general)
Actor: administrador, responsable
Pantalla: ninguna
Pasos:
1. Desde Archivos, una copia de seguridad o una factura en PDF, la persona pulsa descargar.
2. La página tiene los bytes y se los da a la aplicación con un nombre.
3. **Ordenador**: el nombre lo pone la página (saneado) y la carpeta Descargas del usuario la decide la aplicación; si ya existe, el nombre pasa a
   `nombre (2).ext` (hasta 999; nunca pisa un fichero). **Android 10 o posterior**: se publica en la
   colección pública de Descargas (si el nombre está ocupado, Android añade su propio número); se enseña
   «Download/<nombre>».
4. La pantalla enseña «Guardado en {ruta}»: dentro de la aplicación no hay barra de descargas ni aviso.
Entra: nombre y contenido (en base64).
Sale: el fichero; la ruta que se enseña.
Si falla: un nombre con separadores, `:` (flujos alternativos de Windows), caracteres de control, solo
puntos o de más de 255 bytes se rechaza (`download_refused`) en vez de recortarse. Sin carpeta de
descargas alcanzable: «Esta app no puede guardar archivos en un móvil o una tablet. Abre tu negocio en un
navegador para descargarlo.»; cualquier otro fallo, «No se ha podido descargar el archivo.».
Implicados: HUB_SHELL-F126, HUB_SHELL-F132, HUB_SHELL-F173
QA: ninguno

### HUB_APP-F31 Saber que hay una versión nueva y actualizar
Estado: parcial — la aplicación nunca se actualiza sola: abre la descarga en el navegador; macOS no tiene destino; la copia de la Store y la de Play no avisan (las actualiza la tienda)
Actor: administrador, responsable
Pantalla: HUB_SHELL: Sistema
Pasos:
1. Al arrancar y cada 6 horas, la página pregunta a la aplicación qué versión es y al hub cuál es la última
   publicada. Primero por la orden estándar de Tauri, que contestan las aplicaciones anteriores a
   ERPlora/hub#2658; en las nuevas, por `erplora_bridge_status`, que solo contesta al negocio enlazado (si la
   página aún está cargando, lo reintenta hasta unos 8 s después de su `load`).
2. Solo si hay sesión iniciada (la consulta la exige), la publicada es **estrictamente mayor** (comparando número a
   número) y quien está conectado administra el negocio, el menú lateral enseña «Actualizar ERPlora ({versión})»
   mientras la haya, y un aviso emergente lo dice una vez por versión.
3. Al pulsarlo, confirma: en ordenador, «Se abre tu navegador para descargar la versión… No se instala
   nada solo»; en Android, «Se abre la ficha de ERPlora en Google Play…».
4. La descarga o la ficha se abre en el navegador del sistema (HUB_APP-F29). Nada recarga ni cierra la
   ventana: el momento de instalar lo elige la persona.
Entra: la versión instalada, la publicada (`latest.json`, que escribe cada publicación), el sistema y el
canal.
Sale: el navegador abierto en `erplora.com/app/download/<sistema>/` (el Cloud decide el instalador o la
tienda). **La aplicación no comprueba nada de lo que se descarga**: no hay actualizador de Tauri ni firma
(`.exe`/`.msi` sin firmar, `.dmg` sin notarizar).
Si falla: sin red, versión ilegible o sin respuesta (también una página que no es el negocio enlazado):
silencio, ni alarma ni «estás al día». Para las
copias de Play y de Microsoft Store no hay destino y no se ofrece (Google prohíbe descargar un APK fuera de
Play). macOS no tiene descarga. Una versión con sufijo (`1.2.3-beta`) se ignora. «No hemos podido abrir
tu navegador…» si no se pudo abrir.
Implicados: HUB_SHELL-F20, SAAS_PUBLIC-F33, SAAS_PUBLIC-F34
QA: qa-hub-android Fase 4

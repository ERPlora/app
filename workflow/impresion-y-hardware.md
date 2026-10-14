# WORKFLOW — La aplicación instalada · Impresión y hardware

Prefijo: HUB_APP

## Flujos

### HUB_APP-F13 Buscar impresoras
Estado: parcial — con el permiso de Bluetooth negado o el Bluetooth apagado las impresoras Bluetooth no salen y nada dice por qué (`lib.rs:1514-1520`); en macOS 15 una red local negada se ve como «no hay impresoras»
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. Abre **Impresión › Impresoras**: la búsqueda arranca sola; «Re-escanear» la repite.
2. En Android, antes se pide el permiso de red local y el de Bluetooth juntos (HUB_APP-F07).
3. La aplicación junta **una sola lista** de cuatro orígenes: la red (mDNS y barrido del 9100), las
   Bluetooth ya emparejadas (solo Android), las colas USB del sistema (solo ordenador) y las que la
   persona escribió a mano (HUB_APP-F14), sin repetir ninguna.
4. Cada una queda en el registro de esta aplicación para poder darle función.
Entra: la red del dispositivo; en Android, la lista de emparejados de Android.
Sale: la lista de impresoras (red `network:<ip>:<puerto>`, Bluetooth `bluetooth:<mac>`, USB `usb:<cola>`).
Las Bluetooth salen siempre sin clasificar: se ofrecen las emparejadas cuyo nombre o clase parecen
impresora. Un fallo de Bluetooth o de CUPS **no estropea** la mitad de red, pero tampoco se dice: la lista sale sin ellas.
Si falla: sin permiso de red local la búsqueda **no se hace** y devuelve «bloqueada» en vez de una lista
vacía: «ERPlora no ha podido buscar en esta red: el sistema no le ha dado permiso…». Con permiso y sin
nada: el módulo Impresión dice «No se encontraron impresoras de red (puerto 9100) en esta subred.» (la frase
«No se ha encontrado ninguna impresora en esta red…» del hub no la llama nadie; lo dicho en HUB_APP-F09 para macOS). En el
emulador de Android el barrido recorre la red del emulador y no encuentra nada: no es un defecto (QA).
Desde un navegador: «Desde el navegador, este dispositivo no puede llegar a las impresoras…».
Implicados: HUB-F196, HUB_PERIPHERALS-F01, HUB_PERIPHERALS-F04, PRINTING-F02, REC_ALTA-F17
QA: qa-hub §8, qa-hub-android Fase 3

### HUB_APP-F14 Añadir una impresora por su IP
Estado: hecho
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. Pulsa «Añadir impresora por IP» (solo se ofrece en la aplicación instalada).
2. Escribe la dirección IPv4 de la hoja de configuración de la impresora y el puerto (9100).
3. Pulsa «Añadir y probar»: la aplicación se conecta (3 s de plazo) y **solo si contesta** la guarda.
4. Sale en la lista y la hoja de prueba sale por ella (HUB_APP-F20).
Entra: `host` y `puerto`; en Android antes se asegura el permiso de red local.
Sale: la impresora marcada «escrita a mano»: las siguientes búsquedas la siguen listando aunque el barrido
no la vea (otra subred, wifi aislada, mDNS bloqueado).
Si falla: la orden contesta un **código estable** además del texto para que la pantalla distinga lo que
escribió mal (`invalid_printer_address`: «La dirección no es válida…») de una impresora que no contestó
(`printer_unreachable`: «Ninguna impresora ha respondido en {dirección}…»); una aplicación más antigua
que la orden da «No se pudo añadir la impresora. Actualiza la app…». Contestar en el 9100 no prueba que
sea térmica: una láser de oficina también contesta.
Implicados: HUB_PERIPHERALS-F02, PRINTING-F03, REC_ALTA-F17
QA: qa-hub-android Fase 3

### HUB_APP-F15 Impresora Bluetooth (solo Android)
Estado: parcial — no hay forma de emparejar desde ERPlora (se empareja en los ajustes de Android); el nombre y la función de una Bluetooth no se vigilan y, apagada, solo se sabe al imprimir
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. Empareja la impresora en los **ajustes de Bluetooth de Android** (la aplicación no escanea ni empareja,
   para no pedir el permiso de escaneo ni tocar el PIN).
2. En **Impresión › Impresoras** aparece en la lista (HUB_APP-F13) con el nombre del emparejamiento o
   «Bluetooth (MAC)». Dale función y pruébala.
3. Imprimir abre la conexión **en cada trabajo** y la cierra al acabar (no la mantiene).
Entra: el emparejamiento de Android y el permiso de Bluetooth.
Sale: los bytes por el canal serie (SPP). A diferencia de la red, **el resultado vuelve de verdad**: la
orden no contesta hasta que los bytes se han escrito o el intento ha fallado.
Si falla: Bluetooth apagado o sin permiso, o impresora fuera de alcance/apagada: la orden rechaza con el
motivo («impresora inalcanzable: …» o `bluetooth_permission_denied`) y la persona lo ve en la hoja de
prueba y el hub marca el trabajo como fallido para reintentar. El conectar puede tardar segundos; la
orden no deja la pantalla colgada porque corre fuera del hilo principal.
Implicados: HUB_PERIPHERALS-F09, HUB_PERIPHERALS-F14, PRINTING-F02, PRINTING-F05, REC_ALTA-F17
QA: qa-hub-android Pendiente (Bluetooth SPP, fase C)

### HUB_APP-F16 Impresora USB (solo ordenador)
Estado: parcial — una USB se ve, recibe función y se prueba, pero no recibe tiques, comandas, trabajos de la cola ni el cajón al cobrar; en Windows no existe (ERPlora/hub#1269)
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. En macOS o Linux con la impresora puesta y su controlador instalado, **Impresión › Impresoras**
   la muestra con el nombre de la cola del sistema.
2. Dale función; «Probar» manda la hoja por la cola RAW del sistema.
3. Un tique, una comanda o un trabajo de la cola **no llegan a ella**: la puerta de impresión y el
   alta del puesto solo reconocen impresoras con dirección de red o Bluetooth con MAC
   (`apps/web/src/lib/print.ts:199-202`); una USB entra en el registro con dirección vacía
   (`crates/peripherals/src/registry.rs:259-273`).
4. En Android no hay cola del sistema: un identificador `usb:` falla con «impresora inalcanzable… usa una
   de red o una Bluetooth emparejada».
Entra: el identificador `usb:<cola>`.
Sale: solo la hoja de prueba y la orden directa; con las colas CUPS la aplicación comprueba el estado
antes de enviar y cancela a los 15 s lo que no sale (`HUB_PERIPHERALS-F07`).
Si falla: en Windows no se listan colas (no hay `lp`): una impresora USB de Windows se usa por red.
Implicados: HUB_PERIPHERALS-F03, PRINTING-F02, REC_ALTA-F17
QA: qa-hub §8

### HUB_APP-F17 Dar nombre y función a una impresora, y quitarla
Estado: parcial — un nombre puesto a mano lo pisa la siguiente búsqueda; quitar la función o borrar la impresora aquí no da de baja al dispositivo en el hub; no hay ninguna pantalla que llame a renombrar ni a quitar
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. En la tarjeta de la impresora elige una función en el desplegable «Rol» (Recibo, Cocina, Barra,
   Etiqueta): la aplicación la guarda en el registro del dispositivo (`devices.json`, en su carpeta de
   datos) y devuelve el registro entero.
2. La orden de **renombrar** cambia el nombre y la de **quitar** borra la entrada; ambas devuelven el
   registro actualizado.
3. En el siguiente arranque, si el dispositivo tiene una única impresora **de red** (con dirección) y ninguna
   función en todo el registro, la página le pone «Recibo» sola (`print-host.ts:66-68`); una Bluetooth o una USB
   sola no la recibe.
Entra: la clave de la impresora (MAC o identificador) y el valor; la función `receipt`, `kitchen`, `bar` o
`label`.
Sale: el registro de **este dispositivo**; el hub lo sabe solo cuando el puesto se da de alta (HUB_APP-F18).
Si falla: una impresora que el registro no tiene («escanea otra vez»). **Un nombre puesto a mano lo
sobrescribe la siguiente búsqueda** del mismo equipo (`crates/peripherals/src/registry.rs:300-310`,
`existing.name = name`). Quitar la entrada del registro no retira al dispositivo del hub ni la función
queda sin dar de baja: el dispositivo se vuelve a dar de alta solo mientras tenga otra función, y una
función ya anunciada no se retira (`HUB-F197`). No existe la acción «dejar sin función». La orden de
renombrar y la de quitar existen y están concedidas, pero **ninguna pantalla las llama** (el SDK no tiene
`setDeviceName` ni `removeDevice`; el módulo Impresión solo usa `setDeviceRole`, `addNetworkPrinter` y
`testPrint`) y, con una clave que no existe, contestan «correcto» (`registry.rs:431-434,476-479`).
Implicados: HUB-F196, HUB-F197, HUB_PERIPHERALS-F04, PRINTING-F04
QA: qa-hub §8

### HUB_APP-F18 Ser el puesto que imprime
Estado: parcial — el puesto solo imprime mientras la ventana está viva (no hay icono de bandeja ni nada que la mantenga al cerrarla) y, en Android, mientras el sistema no la congele (HUB_APP-F26 solo lo evita si hay algo que avisar y los avisos no están negados); una función quitada en el hub no la retira este puesto
Actor: sistema
Pantalla: ninguna
Pasos:
1. Al abrir el hub dentro de la aplicación, la página pregunta a la aplicación si hay hardware
   (`erplora_bridge_status`: contesta con la versión de la aplicación, la de `tauri.conf.json`).
2. Si lo hay, la página lee el registro de impresoras de la aplicación y, si hay una sola impresora
   **de red** y ninguna función, le pone «Recibo» (una Bluetooth sola no la recibe, `print-host.ts:66-68`).
3. La página da de alta este equipo en el hub como puesto de las funciones de sus impresoras alcanzables
   y abre el canal en vivo de la cola; cada trabajo que el hub le entrega se lo pasa a la aplicación
   (HUB_APP-F19).
4. Si la ventana se cierra, el canal se cae con ella; el hub da el puesto por muerto a los 90 s.
Entra: el registro de la aplicación y la sesión del hub.
Sale: la aplicación aporta la lista, la versión y la orden de imprimir; la cola, las funciones y la
reconexión son del hub y de la página.
Si falla: un navegador sin aplicación no es puesto. Una USB no cuenta como alcanzable (HUB_APP-F16). Lo
que la aplicación no hace es avisar al hub de que una impresora se apagó: el vigilante marca en línea o fuera de
línea cada 30 s en `devices.json` y sus eventos van a un `eprintln!` que nadie lee (HUB_PERIPHERALS-F05).
Implicados: HUB-F196, HUB-F197, HUB-F198, HUB_PERIPHERALS-F04, HUB_SHELL-F73, HUB_SHELL-F74, HUB_SHELL-F162, REC_ALTA-F17
QA: qa-hub §8

### HUB_APP-F19 Imprimir un tique, una factura o una comanda
Estado: parcial — desde hub#2494 la impresora de red contesta lo que pasó con el papel, pero una impresora encendida sin papel o con la tapa abierta sigue contestando «correcto» (no se consulta su estado), la cola interna vive en memoria y su trabajador único retiene ~13 s a las demás impresoras del equipo por cada trabajo a una apagada
Actor: sistema, empleado
Pantalla: ninguna
Pasos:
1. Una venta cobrada, una comanda disparada o un trabajo de la cola llegan a la aplicación con la
   impresora de su función, el tipo de documento y el documento estructurado.
2. La aplicación comprueba el identificador, que el tipo es uno de los ocho conocidos y que el documento
   se puede componer; si no, **rechaza ya** y el error vuelve a quien pidió el papel.
3. **Red**: pone los bytes en una cola interna en memoria y **espera** a que salgan (`erplora_print` →
   `print_to_network`, hub#2494). Un único trabajador la vacía: 3 intentos, 3 s para conectar, 10 s para
   escribir, 2 s entre intentos. Contesta «correcto» cuando la impresora se queda los bytes y el error
   cuando se agotan los intentos.
4. **Bluetooth**: manda los bytes por el canal serie y contesta cuando se han escrito; el fallo vuelve.
5. **USB**: la orden existe pero la puerta no se la da (HUB_APP-F16).
Entra: el identificador de la impresora, el tipo de documento y el documento (nunca HTML).
Sale: lo que **se le devuelve a la pantalla** por caso:

   | Caso | Qué recibe quien pidió el papel |
   |---|---|
   | Identificador mal formado, tipo desconocido, documento que no compone | Error al instante |
   | Red, impresora apagada, IP equivocada, sin permiso de red local en Android 17 o en macOS 15 | Error «impresora inalcanzable» con el motivo, tras los 3 intentos (unos 13 s) (hub#2494) |
   | Red, impresora encendida pero sin papel o con la tapa abierta | **«Correcto» y nada más**: no se consulta el estado de la impresora; escribir en el socket funciona |
   | Bluetooth fuera de alcance, apagado o sin permiso | Error con el motivo |
   | USB (solo la hoja de prueba) | Error con el motivo si la cola no está lista o no saca el trabajo en 15 s |

   Quien lo pidió trata el «correcto» como entregado y el error como no impreso: la puerta directa de
   la caja devuelve `printerFailed` y la caja avisa con «Reintentar» (HUB_SHELL-F70, F72, F78); el
   puesto que vacía la cola del hub manda `done` o `failed` según la respuesta (HUB-F199).
   La cola interna vive en memoria: lo que espera se pierde al cerrar la aplicación; no se deduplica el
   `jobId`; el trabajador único detiene ~13 s a las **demás** impresoras del mismo equipo por cada
   trabajo a una impresora apagada.
Si falla: lo anterior. Sin papel o con la tapa abierta la respuesta sigue siendo «correcto»: ahí la
salida de verdad es mirar el papel (qa-hub §8: «mira si sale papel, no el valor de retorno»). Ver
también `HUB_PERIPHERALS-F06` y `HUB-F199`.
Implicados: HUB-F199, HUB_PERIPHERALS-F06, KITCHEN-F08, PRINTING-F07, PRINTING-F10, SALES-F01
QA: qa-hub §8, qa-hub-restaurant §16, qa-hub-android Fase 3

### HUB_APP-F20 Hacer una hoja de prueba
Estado: hecho
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. Pulsa «Probar» en la tarjeta de la impresora (o se envía sola al añadir por IP).
2. La aplicación compone la hoja (idioma y nombre del negocio, si llegan; si no, «ERPlora» en español) y
   la manda: por la cola interna si es de red, directa si es USB o Bluetooth.
Entra: el identificador de la impresora y, opcionalmente, el nombre del negocio y el idioma.
Sale: la hoja; nada guardado; no pasa por la cola del hub. Un módulo más antiguo que la aplicación
no manda datos y la hoja sale igual con «ERPlora» en español.
Si falla: de red, tras los intentos (unos 13 s) vuelve «impresora inalcanzable» (HUB_APP-F19,
hub#2494). De USB o Bluetooth, el error vuelve en el momento. En los dos casos sale en rojo en la
pantalla de Impresión.
Implicados: HUB_PERIPHERALS-F14, PRINTING-F05
QA: qa-hub §8, qa-hub-android Fase 3

### HUB_APP-F21 Abrir el cajón
Estado: parcial — el cajón se abre por la impresora, pero no sin venta, una USB nunca lo recibe y el error se descarta antes de llegar a la persona; con una IP que no contesta la orden no tiene plazo propio
Actor: sistema, empleado
Pantalla: ninguna
Pasos:
1. Con «Abrir cajón al cobrar» activo, el dispositivo que cobró le manda a la aplicación el pulso para la
   impresora de Recibo.
2. **Red**: abre una conexión directa (sin cola, sin reintentos) y manda cinco bytes; **Bluetooth**: por
   el canal serie, esperando el resultado; **USB**: la orden existe pero la puerta no se la da al cobrar.
3. El pulso va por el pin 2; la orden trata cualquier otro valor como pin 5, pero ninguna pantalla manda otro
   (el SDK pone 2 y `print-on-sale.ts:195` no lo pasa): el pin 5 no se puede elegir.
Entra: la impresora y el pin.
Sale: el pulso; nada guardado ni avisado al hub.
Si falla: la orden **sí devuelve el error** (a diferencia de imprimir): conexión rechazada, impresora
inalcanzable. Pero la pantalla lo descarta (`apps/web/src/lib/print-on-sale.ts:195`, `.catch(() =>
undefined)`). La conexión de red no lleva plazo propio (`crates/peripherals/src/drawer.rs`, `open_drawer`):
una IP que no responde cuelga la orden lo que dure el plazo del sistema, y el error es el de E/S, no
«impresora inalcanzable». Un pulso por el pin equivocado no da error: el cajón no se abre.
Implicados: HUB-F207, HUB_PERIPHERALS-F15, PRINTING-F13, SALES-F01
QA: qa-hub §8, qa-hub-restaurant §16

### HUB_APP-F22 Imprimir un documento A4 con el diálogo del sistema
Estado: hecho
Actor: empleado, administrador, responsable
Pantalla: Ventana de impresión
Pasos:
1. Desde un documento A4 que **manda su HTML** a la puerta de impresión, la persona pulsa imprimir. Facturas
   no lo manda: su factura va a la térmica de «Recibo» (PRINTING-F08).
2. **Ordenador**: se abre una ventana que enseña el documento y, al cargar, el diálogo de impresión del
   sistema con su lista de impresoras y «Guardar como PDF»; la ventana se queda detrás como vista previa
   hasta que se cierra.
3. **Android**: se abre la pantalla de impresión de Android con el documento a A4.
Entra: el HTML del documento (no vacío, con tope de tamaño).
Sale: nada guardado. Se contesta cuando el diálogo se ha pedido; lo que la persona haga en él (imprimir,
guardar, cancelar) es del sistema.
Si falla: documento vacío o demasiado grande: `print_document_refused`; sistema sin diálogo (iOS):
`native_print_unsupported`; ventana que no se abre: `native_print_failed`. Si el diálogo falla y
hay una impresora de «Recibo» alcanzable, **la puerta sigue**: el documento sale por la térmica y vuelve
`via: bridge`, dado por impreso (`print.ts:375-411`); solo sin impresora devuelve el motivo. El documento corre sin scripts (política
que no permite ninguno), sin permisos de ninguna orden y sin poder navegar a otro sitio.
Implicados: HUB_SHELL-F77
QA: qa-hub-android Fase 3

### HUB_APP-F23 Leer una tarjeta NFC para entrar
Estado: parcial — la lectura con tarjeta y tableta reales no está validada (pm#145); en ordenador no existe y el lector USB sigue siendo la vía
Actor: responsable, empleado
Pantalla: HUB_SHELL: Acceso
Pasos:
1. En Android, en una pantalla que espera una tarjeta (acceso con placa, aprobación, ficha de personal),
   la persona acerca la tarjeta a la tableta.
2. La página abre una ventana de lectura de 15 s (acotada entre 1 y 60 s) y la repite mientras alguien
   la espera; la tableta pita al leer.
3. La aplicación contesta el número de la tarjeta (el UID en mayúsculas hexadecimal, sin separadores),
   que entra por **la misma puerta** que el lector USB: ninguna pantalla sabe de dónde vino.
Entra: la tarjeta (se leen NFC-A, B, F y V; no se busca mensaje).
Sale: la placa, que viaja en claro a la página y de la página al hub; el hub guarda un índice HMAC-SHA256 y un
hash argon2, nunca el número (`identity.rs:240-251`); nada guardado aquí.
Si falla: tres rechazos distintos: sin lector (`nfc_unavailable`: la página deja de preguntar toda la
sesión), lector apagado (`nfc_disabled`: «El NFC está apagado en este aparato. Enciéndelo para leer las
tarjetas acercándolas.», una vez por pantalla) y tarjeta que da un número nuevo en cada lectura
(`nfc_random_uid`: «Esta tarjeta da un número distinto cada vez que se lee, así que no puede usarse como
placa. Prueba con otra.»). Un UID de ceros o de menos de 4 bytes también se rechaza. Pasada la ventana sin
tarjeta no es un error. En ordenador la orden contesta siempre `nfc_unavailable`. El NFC no es requisito
de instalación: la ficha de Play sigue abierta a tabletas sin chip.
Implicados: HUB-F134, HUB_SHELL-F05, HUB_SHELL-F87
QA: ninguno

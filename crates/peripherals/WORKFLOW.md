# WORKFLOW — Periféricos: impresoras y cajón

Prefijo: HUB_PERIPHERALS
Alcance MVP: transversal

> Contrato de comportamiento del crate `erplora-peripherals` (pm#620, pm#621). Se lee antes de tocar
> `crates/peripherals`. Contrastado contra `origin/develop` del hub el 05/10/2026. Lo técnico vive en
> `architecture/hub/crates/peripherals.md` y `architecture/hub/print-queue.md`. La mitad que decide
> **qué** se imprime y **a quién** se le da (la cola del hub, los dispositivos que imprimen, la
> cobertura) es del `WORKFLOW.md` de la raíz del hub, área «Impresión» (`HUB-F190` a `HUB-F207`).

## Para qué sirve y para quién

Es lo que **saca el papel**: el trozo de la aplicación instalada que habla con la impresora y el
cajón. Busca impresoras en la red y las cables por USB del ordenador, recuerda cuáles hay y para qué
sirve cada una, convierte un documento (tique, factura, comanda, cuenta, etiqueta, cierre de caja) en
los bytes que entiende una térmica (ESC/POS), los manda por red (puerto 9100), por la cola del
sistema (USB, solo ordenador) o por Bluetooth (solo Android, que lo hace el complemento de Android),
y manda el pulso que abre el cajón. No tiene pantallas ni decide qué se imprime: lo usan el
**administrador** y el **responsable** (montan las impresoras desde la pantalla «Impresoras» del
módulo Impresión) y, sin saberlo, el **cajero** y la **camarera** cada vez que cobran o disparan una
ronda. Sirve a los dos negocios: la peluquería saca tique y etiquetas; el restaurante, además,
comanda de cocina, de barra y cuenta de la mesa.

**Dónde corre.** Solo dentro de la aplicación instalada (Windows, macOS, Android): el hub no linka
este crate, y un navegador no llega a las impresoras (por eso la pantalla de impresoras dice que
desde el navegador no se puede). Qué dispositivo saca el papel de qué trabajo lo decide el hub (la
cola y sus «hosts»); este crate solo obedece cuando el dispositivo le dice «imprime esto en esa
impresora».

## Referencia adoptada

Ya contrastada en `.claude/agents/qa-hub-restaurant.md` §2 y su sección de hardware (§16) y en
`.claude/qa/qa-hub.md` §8; no se rehace. Se adopta solo esto:

- **ESC/POS por el puerto 9100** ([Epson ESC/POS](https://download4.epson.biz/sec_pubs/pos/reference_en/escpos/)):
  es el lenguaje que hablan las térmicas de Epson, Star en modo emulación, Bixolon y compatibles;
  el puerto 9100 es el «raw printing» estándar. Square y Toast hablan igual con sus térmicas de red.
- **Cola RAW del sistema para USB** (CUPS en macOS y Linux): el fabricante pone el controlador, el
  hub sigue mandando los mismos bytes. Se descartó escribir un controlador por sistema.
- **Impresora = destino, función = para qué sirve** (Toast, Square, Odoo): la impresora no sabe si
  es la de cocina; eso lo dice la función que se le pone (Recibo, Cocina, Barra, Etiqueta). El mapa
  documento → función es del hub.
- **Cajón por la impresora** (RJ11/DK, pulso ESC/POS `ESC p`): lo abre la impresora del tique, no
  hay otra conexión. Square y Toast lo abren con cada pago en efectivo; ERPlora hoy lo abre con
  cualquier pago (dudas abiertas de Impresión).
- **El papel que sale mal es peor que el que no sale** (regla de QA del hub): un documento que no se
  puede componer se rechaza, no se corta en blanco.

## Antes de empezar

- La **aplicación de ERPlora instalada** en el dispositivo que está en la red de la impresora (o con
  el cable USB puesto), abierta en ese puesto. Es lo único que enlaza este crate.
- Una **impresora térmica ESC/POS** encendida: de red (puerto 9100) y en la misma red que el
  dispositivo, por USB en un ordenador con la cola del sistema creada (la instala el controlador
  del fabricante), o Bluetooth emparejada en Android. Una impresora de oficina (A4) no entiende
  ESC/POS: el descubrimiento la marca «a4» cuando la reconoce y la pantalla avisa al darle función.
- En **macOS y Linux** para USB, las herramientas de cliente de CUPS (`lp`, `lpstat`, `lpoptions`,
  `cancel`): macOS las trae; en Linux es el paquete `cups-client`.
- En **Android 17 (API 37) y en macOS 15**, el permiso del sistema de «red local» concedido a la
  aplicación; sin él la búsqueda no se hace y la pantalla lo dice (HUB_APP).
- Configuración paso a paso: la de Impresión (`PRINTING`, «Configuración inicial»); aquí solo cambia
  lo que pasa por debajo.

## Pantallas

Este crate **no tiene pantallas propias**. Lo que hace se ve en la pantalla «Impresoras» del módulo
Impresión (lista de impresoras de la red, desplegable «Rol», «Probar», «Añadir impresora por IP»,
«Re-escanear») y en el papel. Cada flujo de abajo dice en `Pantalla:` cuál es la del módulo, o
`ninguna` cuando es algo que ocurre sin que nadie pulse nada.

## Flujos

### HUB_PERIPHERALS-F01 Buscar impresoras en la red
Estado: hecho
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. En la aplicación instalada, abre **Impresión → Impresoras**: la búsqueda arranca sola
   («Escaneando…»). «Re-escanear» la repite.
2. La aplicación mira dos cosas a la vez: quién se anuncia en la red como impresora (mDNS) y quién
   contesta en el puerto 9100 en las 254 direcciones de la red de este dispositivo (los tres
   primeros números de su dirección, de .1 a .254).
3. Aparece cada impresora una sola vez, con un nombre («Network Printer (192.168.1.50)» si solo
   contestó al puerto) y su estado «Lista». Las USB y las Bluetooth ya emparejadas salen en la misma
   lista (HUB_PERIPHERALS-F03; el Bluetooth es del complemento de Android).
Entra: la red del dispositivo (anuncios mDNS `_pdl-datastream._tcp` e `_ipp._tcp`, barrido TCP al
9100) y el permiso de red local que concede el sistema.
Sale: la lista de impresoras; cada una de red queda en el registro de la aplicación con su MAC si
el sistema la resuelve por ARP (en Android nunca; en un ordenador con VPN o cortafuegos, a veces
no): sin MAC igualmente se registra, identificada por su dirección. Una que anuncia IPP (las de
oficina y las AirPrint) sale con la categoría «a4»; las que solo contestan al 9100 salen «sin
clasificar»: no se adivina que sea térmica. El barrido sondea de 64 en 64 con 1 s por dirección
(unos 4 s si no hay nadie) porque con menos margen una impresora encendida se perdía sin error.
Si falla: sin permiso de red local la búsqueda no se hace y la aplicación lo dice, en vez de devolver
una lista vacía que se confunde con «no hay impresoras» (HUB_APP pinta el aviso); con cero
resultados no se sabe si la impresora está apagada, en otra red o tras un cortafuegos: se prueba
con la dirección que sale en la hoja de la impresora (HUB_PERIPHERALS-F02). Si el mDNS no arranca,
queda el barrido. Una impresora que solo se anuncia por IPP aparece con el puerto del anuncio (631),
no el 9100.
Implicados: HUB_APP-F07, HUB_APP-F09, HUB_APP-F13, HUB_SHELL-F140, PRINTING-F02, REC_ALTA-F17
QA: qa-hub §8, qa-hub-android §15

### HUB_PERIPHERALS-F02 Añadir una impresora de red por su IP
Estado: hecho
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. Pulsa «Añadir impresora por IP».
2. Escribe la dirección que sale en la hoja de configuración de la impresora y el puerto (9100).
3. Pulsa «Añadir y probar»: la aplicación llama a esa dirección y espera hasta 3 s.
4. Si contesta, la impresora queda guardada y sale en la lista; si no, no se guarda nada.
Entra: una dirección IPv4 (cuatro números separados por puntos; IPv6 o un nombre de equipo no valen)
y un puerto de 1 a 65535.
Sale: la impresora, marcada como «escrita a mano»: la lista la sigue enseñando en cada búsqueda
aunque el barrido no la vea (suele estar en otra subred o en una wifi aislada), con el estado que le
dé el vigilante (HUB_PERIPHERALS-F05). Nunca se guarda una que no contestó.
Si falla: dirección que no es IPv4 o puerto 0, se rechaza sin llamar a nadie («La dirección no es
válida…» en la pantalla de Impresión); si nadie contesta en 3 s, «Ninguna impresora ha respondido en
{dirección}…» y no se guarda. Contestar en el 9100 no prueba que sea una térmica: una láser de oficina
también contesta, y la impresora se guarda como «sin clasificar».
Implicados: HUB_APP-F14, PRINTING-F03, REC_ALTA-F17
QA: qa-hub-android §15

### HUB_PERIPHERALS-F03 Encontrar las impresoras USB del ordenador
Estado: hecho
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. En un ordenador (macOS o Linux) con la impresora enchufada y su controlador instalado, abre
   **Impresión → Impresoras**.
2. En la misma búsqueda de HUB_PERIPHERALS-F01 aparece también la impresora USB, con el nombre que
   le da el cable (por ejemplo «Star TSP143 (STR_T-001)»).
3. Su estado dice si el sistema la ve preparada («Lista»), parada («Parada»: sin papel, tapa
   abierta, cable fuera o en pausa) o no lo sabe («Desconocido»).
Entra: las colas de impresión del sistema y su estado.
Sale: cada cola USB entra en la lista y en el registro de la aplicación identificada por su nombre
(`usb:<cola>`), así que admite función y nombre como cualquier otra. Solo salen las colas que van
por cable USB: las de red no se duplican. Se pregunta por el estado en el idioma del sistema sin
depender de él (se leen los atributos IPP, no las frases traducidas).
Si falla: si el sistema no tiene las herramientas de CUPS o no responde en 20 s, no salen impresoras
USB y la búsqueda de red sigue funcionando; solo queda un aviso en el registro de la aplicación, no
en pantalla. En Android no hay cola del sistema que consultar, y en Windows no se sabe hacer
(HUB_PERIPHERALS-F08).
Implicados: HUB_APP-F16, PRINTING-F02
QA: qa-hub §8

### HUB_PERIPHERALS-F04 Recordar cada impresora y su función
Estado: parcial — no hay forma de quitarle la función a una impresora ya asignada (solo cambiarla por otra o borrar la impresora)
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. En la tarjeta de la impresora abre el desplegable «Rol» y elige una función (Recibo, Cocina,
   Barra, Etiqueta).
2. La aplicación recuerda la función aunque se cierre y se vuelva a abrir.
3. Con el tiempo se puede cambiarle el nombre o quitarla de la lista; pero el nombre que se le ponga dura hasta la siguiente búsqueda, que vuelve a poner el del anuncio, «Network Printer (IP)» o el de la cola.
Entra: la impresora elegida y la función.
Sale: una entrada por impresora en el fichero de dispositivos de la aplicación (`devices.json`, en
su carpeta de datos), con su identificador (la MAC si se conoce y, si no, `network:<ip>:<puerto>` o
`usb:<cola>`), su dirección, su nombre, su función, cuándo se vio por primera y última vez y si
está en línea. Las impresoras Bluetooth entran con su MAC y sin dirección. Una impresora que se
vuelve a encontrar conserva su función y la fecha de alta; una MAC que aparece después enriquece la
entrada, nunca la borra. Un fichero de una versión anterior se completa al cargarlo, sin perder las
funciones ya puestas. La función que elige la persona es **solo de este dispositivo**: que el hub
sepa que este dispositivo imprime esa función es otro paso (HUB-F196).
Si falla: asignar función a una impresora que el registro no tiene («escanea otra vez e inténtalo»);
no pasa si la impresora salió de una búsqueda de esta aplicación. Si el fichero está corrupto o no se
puede leer, la aplicación arranca con el registro vacío y avisa solo en el registro técnico: las
funciones se pierden y hay que volver a ponerlas. No existe la acción de dejar una impresora
sin función. Quitarle la función a una impresora o borrarla del registro **no da de baja al dispositivo en el hub** (HUB-F197): el hub sigue dando la función por cubierta.
Implicados: HUB-F196, HUB_APP-F13, HUB_APP-F17, HUB_APP-F18, HUB_SHELL-F73, PRINTING-F02, PRINTING-F04

Pendiente de enlazar: hub — HUB_APP, el registro de dispositivos de la aplicación instalada y sus órdenes
QA: qa-hub §8

### HUB_PERIPHERALS-F05 Seguir la pista de una impresora que se apaga o cambia de dirección
Estado: parcial — el vigilante solo anota «perdida» o «recuperada» en el registro técnico de la aplicación: ninguna pantalla ni aviso lo cuenta; sin MAC (siempre en Android) no se recupera una impresora que cambió de dirección; ni Bluetooth ni USB se vigilan
Actor: sistema
Pantalla: ninguna
Pasos:
1. La aplicación arranca un vigilante en segundo plano (espera 5 s al empezar).
2. Cada 30 s prueba el puerto de cada impresora de red conocida: si no contesta pasa a «fuera de
   línea», y si vuelve a contestar, a «en línea».
3. Cada 120 s, a las que siguen fuera de línea y de las que se conoce la MAC, las busca en las 254
   direcciones de la red: si aparece una con esa MAC en otra dirección (porque el router le dio una
   nueva), actualiza la dirección y la marca «en línea».
Entra: el registro de impresoras y la red.
Sale: el estado y la dirección actualizados en el registro; un registro técnico de cada cambio. El
estado «fuera de línea» solo lo ve la persona en las impresoras escritas a mano por IP (las de la
búsqueda salen siempre «Lista» porque se acaban de ver contestar).
Si falla: si la aplicación no puede leer su red local, no recupera nada y no lo dice; con la
impresora apagada el vigilante la marca fuera de línea y ningún trabajo de la cola lo sabe (los
trabajos siguen yendo a esa dirección: HUB_PERIPHERALS-F06).
Implicados: PRINTING-F02
QA: ninguno

### HUB_PERIPHERALS-F06 Sacar un documento por una impresora de red
Estado: parcial — desde hub#2494 la aplicación contesta lo que pasó con el papel (correcto cuando llegó a la impresora, «impresora inalcanzable» cuando se agotaron los intentos), y la caja que lo pidió directo avisa con «Reintentar» (HUB_SHELL-F70); pero la cola interna sigue en memoria (se pierde al cerrar la aplicación), tiene un solo trabajador (una impresora apagada retiene ~13 s los trabajos de otras impresoras del mismo dispositivo) y un trabajo de la cola del hub que falla vuelve «fallido» sin que nadie lo vea (HUB_SHELL-F74); y una impresora encendida sin papel se da por impresa (hub#2716)
Actor: sistema
Pantalla: ninguna
Pasos:
1. La caja que cobra o dispara la comanda, si tiene una impresora con esa función, la manda **directamente**
   sin pasar por la cola del hub (`print.ts:381-404`): en ese caso un papel perdido no deja ninguna fila
   en el hub. Si no, el dispositivo que imprime (el que tiene la función en el hub) recibe un trabajo de
   la cola. En los dos casos se lo pasa a la aplicación con la impresora de esa función, el tipo de documento y el documento.
2. La aplicación comprueba que el tipo de documento es uno conocido y compone los bytes ESC/POS
   (HUB_PERIPHERALS-F09 a F13).
3. Pone los bytes en su cola interna y **espera** a que salgan.
4. Una cola con un solo trabajador los manda a `<ip>:<puerto>`: hasta 3 intentos seguidos, 2 s entre
   intento e intento, 3 s para conectar y 10 s para escribir en cada uno. Cuando la impresora se los
   queda contesta «correcto» al dispositivo; si los 3 intentos fallan, contesta el error (con la
   impresora apagada, unos 13 s después de pedirlo).
Entra: el identificador de la impresora (`network:<ip>:<puerto>`), el tipo de documento (uno de
ocho: tique, factura, comanda, albarán, etiqueta, cierre de caja, cuenta, genérico) y el documento ya
estructurado (nunca HTML).
Sale: los bytes en el papel y la respuesta a quien lo pidió: «correcto» solo cuando el papel llegó a la
impresora, el error cuando no (hub#2494). Si venía de la cola del hub, el dispositivo la confirma
«hecho» o «fallido» según esa respuesta; si salió directo de la caja, la caja avisa (HUB_SHELL-F70,
F72, F78). El resultado de cada intento se sigue escribiendo también en el registro técnico de la
aplicación. La cola interna vive en memoria: lo que está esperando se pierde si se cierra la
aplicación. El trabajo no se deduplica aquí (la clave `jobId` viaja pero no se
comprueba): un mismo trabajo mandado dos veces saca dos papeles.
Si falla: antes de ponerlo en la cola sí hay error y vuelve a quien lo pidió: identificador de
impresora mal formado («printer id inválido»), tipo de documento que no se conoce («tipo de
documento desconocido»), documento que no es un objeto, una cuenta sin líneas o una factura completa
sin los datos que exige (emisor, número, cliente, NIF del cliente, desglose de IVA). Después, con la
impresora apagada o fuera de la red, tras los 3 intentos vuelve «impresora inalcanzable» a quien lo
pidió: la caja que lo mandó directo lo avisa y deja reintentar (HUB_SHELL-F70); un trabajo de la cola
del hub vuelve «fallido» y hoy nadie lo ve (HUB_SHELL-F74). Una impresora sin papel que sigue
aceptando la conexión se queda los bytes: el puerto 9100 no cuenta si hay papel, así que eso sigue sin
detectarse (hub#2716). Mientras se agotan los intentos, la cola de un solo trabajador retiene los trabajos de las
otras impresoras de red del mismo dispositivo.
Implicados: HUB-F199, HUB_APP-F19, HUB_SHELL-F70, HUB_SHELL-F72, HUB_SHELL-F74, HUB_SHELL-F77, HUB_SHELL-F78, PRINTING-F05, PRINTING-F07, PRINTING-F09, PRINTING-F10, PRINTING-F12, REC_ALTA-F17

QA: qa-hub-restaurant §16, qa-hub §8

### HUB_PERIPHERALS-F07 Sacar un documento por una impresora USB
Estado: parcial — la aplicación sabe imprimir por USB, pero el dispositivo no se da de alta en el hub como quien imprime una función cuando su impresora es USB (la puerta de impresión y el alta solo aceptan red y Bluetooth), así que ningún trabajo de la cola llega a una USB: solo sale la hoja de prueba
Actor: sistema
Pantalla: ninguna
Pasos:
1. La aplicación recibe «imprime esto en `usb:<cola>`» con el documento.
2. Antes de mandar nada pregunta al sistema cómo está la cola: si está parada, sin aceptar trabajos
   o con un error (sin papel, cable fuera, en pausa), no manda nada y devuelve el motivo.
3. Manda los bytes a la cola con el título «ERPlora» (para reconocerlo en la ventana de impresión del
   sistema) y sin pasar por los filtros de impresión.
4. Sigue el trabajo hasta 15 s: si sale de la cola, correcto; si sigue allí, lo cancela y devuelve el
   motivo que da el sistema en ese momento.
Entra: el identificador `usb:<cola>` y los bytes ya compuestos; el nombre de la cola se valida (sin
espacios, `/` ni `#`, sin empezar por `-`, hasta 127 caracteres).
Sale: el papel, o un error que vuelve a «Probar» y sale en rojo en la pantalla de Impresión (ningún
trabajo de la cola llega a una USB, así que hoy es el único llamante). Cancelar al vencer los 15 s existe para que un
tique que no salió no aparezca horas después, con un número que la caja ya dio por fallido. Sin
cola interna ni reintentos propios: es directo.
Si falla: «la impresora de la cola `…` no está lista (…); el trabajo NO se mandó», «la cola `…`
rechazó el trabajo: …» o «la impresora aún tenía el tique 15 s después de mandarlo (…); se canceló, así
que NO salió y no aparecerá después». Si las herramientas de CUPS no están, «No se pudo ejecutar
`lp`…». Si el estado no se pudo leer se manda igualmente y se deja al sistema decidir. Si `lp` no
devuelve el identificador del trabajo, se da por mandado sin comprobar que salió.
Implicados: HUB-F199, PRINTING-F02
QA: qa-hub §8

### HUB_PERIPHERALS-F08 Impresora USB en Windows
Estado: no hecho — Windows no tiene cola RAW (el sistema de colas de Windows no se maneja desde este crate); hub#1269
Actor: sistema
Pantalla: ninguna
Pasos:
1. En un ordenador con Windows, con la impresora USB enchufada, abre **Impresión → Impresoras**.
2. La impresora USB aparece en la lista y admite función.
3. Un trabajo de la cola sale por ella.
Entra: la cola de impresión de Windows y su nombre.
Sale: el papel, con la misma garantía de aviso que HUB_PERIPHERALS-F07 (no sale = se dice).
Si falla: hoy, en Windows no se listan colas USB (el comando de CUPS no existe) y una impresora
asignada a una cola USB falla con «No se pudo ejecutar `lp`». Una impresora térmica de Windows se
usa hoy por red (IP).
Implicados: PRINTING-F02
QA: ninguno

### HUB_PERIPHERALS-F09 Sacar el tique o la factura
Estado: parcial — la fecha y la hora del papel son las del momento de imprimir (reloj del dispositivo), no las de la venta: un tique que esperó en la cola, o una reimpresión, sale con otra hora; con una impresora de red apagada, la caja que lo mandó avisa y deja reintentar (hub#2494), pero un tique que iba por la cola del hub vuelve «fallido» sin aviso (HUB_PERIPHERALS-F06)
Actor: sistema
Pantalla: ninguna
Pasos:
1. Un cobro, o una reimpresión desde Ventas o Facturas, manda el documento compuesto por Ventas.
2. El papel sale así, de arriba abajo: si lleva QR fiscal, el texto «QR tributario:» y el QR, y
   debajo la leyenda «VERI*FACTU»; el nombre del negocio, su dirección, su NIF y su teléfono; el
   número del tique y la fecha; el cajero y el cliente si vienen; las líneas con sus suplementos (el
   suplemento sin importe, bajo la línea); el subtotal, el IVA, el descuento y el total; el pago, lo
   entregado y el cambio; el QR «pide tu factura» si el documento lo trae; la cabecera y el pie del negocio; «Gracias»; y, si el negocio puso un
   enlace propio, su nota y un QR más pequeño.
3. Una **factura completa** lleva además el título de factura (o factura rectificativa), el
   desglose de IVA por tipo (porcentaje, base y cuota), el NIF y el nombre del cliente.
4. La marca «DUPLICADO» sale solo con un `true` explícito, debajo de los datos del negocio. «QR tributario:» y «VERI*FACTU» son textos que manda el productor, no fijos.
5. Se corta el papel.
Entra: el documento estructurado que compone `sales` (o `invoice`); la moneda llega con su escala de
decimales (euro 2, yen 0, dinar 3) y el idioma con `locale`.
Sale: el papel. La aplicación no inventa nada fiscal: imprime lo que trae el documento, y rechaza
una factura completa que no trae emisor, número, cliente, NIF del cliente y desglose de IVA, en vez de
cortar un papel que parece factura y no lo es.
Si falla: un documento que no es un objeto se rechaza («un documento es un objeto JSON de campos que
imprimir»). Un campo que falta sale en blanco o con su valor por omisión (nombre «ERPlora» si el
negocio no trae el suyo): el papel no avisa. Un tique sin QR fiscal sale igual, sin marca: que el QR
llegue o no es de Ventas (SALES) y de la cola (el tique puede salir antes de que esté listo, y la
pantalla avisa «El tique salió antes de que estuviera listo su QR de VeriFactu»).
Implicados: HUB_APP-F15, HUB_SHELL-F76, PRINTING-F07, PRINTING-F08, SALES-F01
QA: R-09, qa-hub §8

### HUB_PERIPHERALS-F10 Sacar la comanda de cocina o barra
Estado: parcial — la hora que sale en la comanda es la de imprimir, no la de pedir; con una impresora de red apagada, la caja que la disparó avisa y deja reintentar esa estación (hub#2494), pero una comanda que iba por la cola del hub vuelve «fallida» sin aviso (HUB_PERIPHERALS-F06)
Actor: sistema
Pantalla: ninguna
Pasos:
1. Una ronda disparada, un aviso de urgencia o el pase marcado como lista manda la comanda.
2. El papel sale con la palabra «COCINA» grande, el número de la comanda, la etiqueta de la mesa en
   doble tamaño (en doble ancho si cabe en una línea), el camarero, la ronda (solo a partir de la
   segunda) y la hora.
3. Cada línea sale en negrita y doble altura, «2x Bacalao»; los suplementos, en negrita y sangrados
   debajo; la nota del camarero con «>>» y sin realce; las líneas de un menú, agrupadas bajo su nombre
   y sangradas.
4. Una comanda marcada urgente cierra con «URGENTE» en grande. Se corta el papel.
5. El vale de anulación de una ronda cancelada o de un plato anulado en el TPV (HUB_SHELL-F78) es esta misma comanda: «ANULADA · Mesa 4» (o «PLATO ANULADO · Mesa 4» para un solo plato) donde va la mesa y cada plato con la cantidad en negativo («-2x Croquetas», «-0.5x Flan»); el signo sale tal cual, nunca se redondea a positivo.
Entra: el documento que compone `kitchen` (o el shell): número, etiqueta de sala, camarero, ronda,
prioridad y líneas con cantidad, nombre, suplementos, nota y menú.
Sale: el papel. La función (Cocina o Barra) la decide el hub; esta parte pinta lo mismo en las dos.
Si falla: un documento que no es objeto se rechaza antes de imprimir. Con un campo sin rellenar sale
con su valor por omisión (cantidad 1). Sin comprobar que sea una comanda: una comanda sin líneas sale
con la cabecera y nada más.
Implicados: HUB_SHELL-F72, HUB_SHELL-F78, KITCHEN-F08, KITCHEN-F17, PRINTING-F10
QA: qa-hub-restaurant §08, qa-hub-restaurant §16

### HUB_PERIPHERALS-F11 Sacar la cuenta de la mesa
Estado: parcial — con una impresora de red apagada el error ya vuelve a quien la pidió (HUB_PERIPHERALS-F06, hub#2494), y que la persona lo vea depende de Ventas, que es quien la pide (el aviso con «Reintentar» del shell cubre el tique, la comanda y el vale, no la cuenta); por la cola del hub vuelve «fallida» sin aviso
Actor: sistema
Pantalla: ninguna
Pasos:
1. El camarero pide la cuenta (o el cajero la del cliente) antes de cobrar.
2. El papel sale con el título «CUENTA», el negocio, la mesa (o el cliente), las líneas con sus
   suplementos, los totales y un aviso de que **no es una factura**, partido por palabras al ancho del
   papel. Se corta.
Entra: el documento «cuenta» que compone `sales`, con sus líneas.
Sale: el papel, sin serie, sin forma de pago y sin QR fiscal: la numeración se gasta al cobrar y aún no
hay registro de facturación. Un papel que pasa por factura sería un problema legal, no estético.
Si falla: una cuenta **sin líneas** se rechaza («una cuenta tiene líneas que cobrar; este documento no
trae ninguna (¿es la forma de la pantalla, con `lines`?)») en vez de cortar un papel en blanco con
total 0,00.
Implicados: PRINTING-F09
QA: R-08

### HUB_PERIPHERALS-F12 Sacar la etiqueta de un código de barras
Estado: parcial — sale en la impresora térmica de tiques con corte, no en una impresora de etiquetas (no habla ZPL, TSPL ni EPL), y Inventario manda el SKU, no el EAN
Actor: sistema
Pantalla: ninguna
Pasos:
1. Desde la ficha de un producto, Inventario manda imprimir su etiqueta.
2. El papel sale con el nombre del producto, el código de barras con sus cifras debajo y el precio
   en grande. Se corta.
Entra: nombre del producto, código y precio (Inventario, INVENTORY-F25).
Sale: el papel. El código sale como **EAN-13** si son 12 o 13 cifras (con 12, la impresora calcula la
última) y como **CODE128** si no (solo los caracteres imprimibles; el resto sale como `?`); vacío o de
más de 250 caracteres, se imprime el texto entre corchetes. Se configura la altura (80 puntos) y el
ancho del módulo.
Si falla: sin código, sale el nombre y el precio sin código; sin precio, sin precio. El cajón de la
etiqueta es el de un tique: una impresora de etiquetas de rollo no entiende estos bytes.
Implicados: INVENTORY-F25, PRINTING-F12
QA: qa-hub §8

### HUB_PERIPHERALS-F13 Sacar el cierre de caja, el albarán o un documento genérico
Estado: parcial — la aplicación sabe sacarlos, pero ningún módulo los pide (cierre de caja: PRINTING-F17 no hecho; albarán y genérico sin productor), así que hoy no salen
Actor: sistema
Pantalla: ninguna
Pasos:
1. Un productor manda un documento de cierre de caja, de albarán o genérico.
2. El **cierre de caja** sale con título, sesión, fecha, cajero, saldo de apertura, de cierre, la
   diferencia (cierre menos apertura) y una línea por movimiento.
3. El **albarán** sale con número, fecha, cliente, dirección de entrega, líneas («2x Bacalao») y una
   línea para la firma.
4. El **genérico** sale con título y una línea «clave: valor» por campo, sin formato.
Entra: el documento estructurado de quien lo pida.
Sale: el papel; fechas del momento de imprimir, como en HUB_PERIPHERALS-F09.
Si falla: como el resto: un documento que no es objeto se rechaza; un campo que falta, en blanco.
Implicados: PRINTING-F16, PRINTING-F17
QA: ninguno

### HUB_PERIPHERALS-F14 Hacer una hoja de prueba
Estado: hecho
Actor: administrador, responsable, empleado
Pantalla: printing: Impresoras
Pasos:
1. Pulsa «Probar» en la tarjeta de la impresora.
2. Sale una hoja corta: una raya, el nombre del negocio en grande (o «ERPlora» si no llega), el
   aviso de que la impresora responde, la fecha y la hora y el identificador de la impresora. Se corta.
Entra: el identificador de la impresora, y opcionalmente el nombre del negocio y el idioma (si no
llegan, sale «ERPlora» y en español).
Sale: la hoja, por la cola interna si es de red (F06) y directa si es USB o Bluetooth. Va en el idioma
del hub, firmada con el nombre del negocio, y no pasa por la cola del hub.
Si falla: con una impresora de red que no contesta, tras los intentos de HUB_PERIPHERALS-F06 (unos
13 s) vuelve «impresora inalcanzable» (hub#2494). Con USB o Bluetooth, el error de la impresora vuelve
en el momento. En los tres casos sale en rojo en la pantalla.
Implicados: HUB_APP-F15, HUB_APP-F20, PRINTING-F05
QA: qa-hub §8

### HUB_PERIPHERALS-F15 Abrir el cajón
Estado: parcial — al cobrar solo se abre con una impresora de «Recibo» de red o Bluetooth del dispositivo que cobró (una USB nunca); si no se abre nadie lo sabe (el error se descarta en `print-on-sale.ts:195`); con la impresora de red apagada el intento no tiene plazo propio
Actor: sistema
Pantalla: ninguna
Pasos:
1. Con «Abrir cajón al cobrar» activado, un cobro hecho en este dispositivo abre el cajón (HUB-F207).
2. La aplicación manda a la impresora del tique el pulso que abre el cajón: por el pin 2 del conector
   (lo normal) o por el pin 5 si quien lo pide lo indica.
3. El cajón se abre.
Entra: la impresora (red o Bluetooth al cobrar; USB solo si alguien llamara a la orden directamente, y hoy nadie lo hace) y el pin (2 por omisión; cualquier valor distinto de 2 se
trata como pin 5).
Sale: cinco bytes en la impresora; nada guardado y nada avisado al hub. No pasa por ninguna cola: tiene
que hacerse aquí y ahora, desde el dispositivo que está junto a la impresora.
Si falla: una impresora de red que no contesta devuelve error a quien lo pidió («impresora
inalcanzable»); una USB o Bluetooth, igual. Ese error se pierde antes de llegar a la persona (HUB-F207).
Un pulso por el pin equivocado no da error: el cajón simplemente no se abre.
Implicados: HUB-F207, HUB_APP-F21, HUB_SHELL-F71, PRINTING-F13, SALES-F01

QA: qa-hub §8, qa-hub-restaurant §16

### HUB_PERIPHERALS-F16 Ajustar el papel: ancho, caracteres, idioma y corte
Estado: parcial — el ancho es siempre de 32 columnas: el ancho de papel (80 o 58 mm) que se guarda en Impresión no llega aquí, y el logo del negocio no se imprime
Actor: sistema
Pantalla: ninguna
Pasos:
1. Cualquier documento sale compuesto a 32 columnas, sea el rollo de 58 o de 80 mm.
2. Los textos salen en el idioma del documento (español o inglés; otro idioma sale en español) y con los
   caracteres de la tabla clásica de las térmicas (cp437): las vocales con acento y la eñe salen bien; el
   euro sale como «EUR», las comillas tipográficas, las rayas largas y los puntos suspensivos se
   cambian por sus equivalentes simples.
3. Los textos largos (dirección, cabecera, pie, notas) se parten por palabras al ancho, sin cortar una
   palabra por la mitad.
4. Al final se corta el papel: tres saltos de línea y el corte parcial.
Entra: el documento y su `locale`; el ancho es fijo.
Sale: bytes ESC/POS con alineación, negrita, doble alto y doble ancho; códigos de barras y QR nativos
de la impresora (el QR en modelo 2, módulo de 4 puntos, el promocional de 3, corrección M, hasta
7 KB). No hay imágenes: ni el logo ni gráficos de ningún tipo.
Si falla: un carácter que no está en la tabla sale como `?`; un texto de una sola palabra más ancho que
la línea se deja entero y la impresora lo dobla; en un rollo de 80 mm el papel queda estrecho (32 de
las 48 columnas que cabrían).
Implicados: PRINTING-F06
QA: ninguno

## Cobertura contra la referencia

| Elemento | Estado | Flujo |
|---|---|---|
| Buscar impresoras de red (mDNS y barrido al 9100) | hecho | F01 |
| Añadir una impresora por IP | hecho | F02 |
| Impresora USB (macOS, Linux) | parcial: se lista y se prueba; no recibe nada de la caja, de la cola ni del cajón | F03, F07 |
| Impresora USB en Windows | no hecho | F08 |
| Impresora Bluetooth (Android) | complemento de Android (HUB_APP); aquí solo se valida el identificador | F06 |
| Función por impresora recordada | parcial: no se puede quitar | F04 |
| Vigilar la impresora y recuperarla tras un cambio de IP | parcial: sin aviso, sin MAC no hay recuperación | F05 |
| Cola con reintentos hacia la impresora de red | parcial: en memoria, un solo trabajador | F06 |
| Aviso cuando el papel no sale | parcial: la red devuelve el error a quien lo pidió (la caja avisa con «Reintentar»; la cola del hub no avisa); sin papel no se detecta por red; USB comprueba y cancela | F06, F07 |
| Tique con QR fiscal, «DUPLICADO» y QR promocional | parcial: la hora es la de imprimir | F09 |
| Factura completa con desglose de IVA | hecho | F09 |
| Comanda con etiqueta de sala, ronda, suplementos y urgente | parcial: la hora es la de imprimir | F10 |
| Cuenta no fiscal | parcial: con la red apagada el error vuelve a Ventas; el aviso con «Reintentar» del shell no la cubre | F11 |
| Etiqueta con código de barras nativo | parcial: térmica de tiques, no de etiquetas | F12 |
| Cierre de caja, albarán, genérico | parcial: sin productor | F13 |
| Hoja de prueba | hecho | F14 |
| Cajón (pin 2 y pin 5) | parcial: nunca por USB, error descartado, sin plazo | F15 |
| Ancho 58 y 80 mm | no hecho: siempre 32 columnas | F16 |
| Logo e imágenes en el papel | no hecho | F16 |
| Báscula | no existe (dudas abiertas) | — |
| Lector de códigos de barras | no aplica: es un teclado para el sistema | — |

## Datos: de quién es cada dato

- **Propios del crate** (en el dispositivo, no en el hub): el registro de dispositivos
  (`devices.json`): impresoras de red, USB y Bluetooth conocidas, su función, su nombre, su MAC (cuando
  se resuelve), su dirección y si están en línea. Una fila por impresora y dispositivo.
- **Del hub**: la cola de trabajos, las funciones, el mapa documento → función y los «hosts» de
  impresión (HUB-F190 a HUB-F207). Este crate no los lee ni los escribe: recibe un trabajo ya
  entregado.
- **De Ventas, Cocina, Inventario**: el contenido del documento. El crate no guarda ninguno: lo compone
  en bytes y se olvida.
- **Datos personales** (inventario RGPD): ninguno persistente. El documento que se imprime (nombre,
  NIF y dirección del cliente en una factura; mesa y camarero en una comanda) pasa por la memoria de la
  aplicación y por el registro técnico solo en su forma de error (identificador de trabajo y motivo, no
  el documento). Lo único guardado que identifica algo es la MAC y la dirección de las impresoras (de
  la empresa, no de personas) y el nombre que se le puso a cada una.

## Reglas que no se rompen

- **Un tipo de documento desconocido se rechaza**, nunca sale como «genérico»: una comanda que sale
  mal se descubre cuando falta el plato. El vocabulario es el mismo, de ocho, que el de la cola del hub.
- **Un documento que no es un objeto, una cuenta sin líneas y una factura completa sin los datos que la
  hacen factura se rechazan antes de imprimir**: el papel que sale mal es peor que el que no sale.
- **La cuenta de la mesa no lleva serie, ni forma de pago ni QR fiscal, y lleva el aviso de que no es
  una factura.** El tique solo lleva «DUPLICADO» si se pide con un `true` explícito.
- **Un USB que no está listo no recibe el trabajo y, si lo acepta y no lo saca en 15 s, se cancela**:
  salvo que CUPS no deje leer la cola o no devuelva el identificador del trabajo (entonces se da por mandado sin seguirlo), no queda un tique detenido que salga horas después.
- **La impresora se identifica por su transporte**: `network:<ip>:<puerto>`, `bluetooth:<mac>` o
  `usb:<cola>`; cualquier otro identificador se rechaza al entrar, no dentro de una conexión.
- **Una impresora que no contestó no se guarda al añadirla por IP**, y una búsqueda sin permiso de red
  local no devuelve una lista vacía: dice que no pudo mirar.
- **El registro conserva lo que la persona decidió**: volver a encontrar una impresora conserva su
  función y su fecha de alta (el nombre no: lo repone la búsqueda).
- **No se adivina que una impresora sea térmica.** Solo el anuncio IPP la marca «a4»; el resto queda
  «sin clasificar».

## Lo que NO hace, a propósito

- No decide qué documento sale por qué impresora, ni guarda cola de trabajos que sobreviva a cerrar la
  aplicación: eso es del hub (cola, mapa y hosts).
- No tiene controlador por sistema operativo: USB va por la cola RAW del sistema; no usa `libusb`, ni
  WebUSB, ni OPOS.
- No maneja el escáner de códigos: lo trata el sistema como un teclado.
- No habla lenguajes de impresoras de etiquetas (ZPL, TSPL, EPL) ni de oficina (PCL, PostScript).
- No imprime imágenes ni el logo del negocio.
- No guarda historial de lo que imprimió.
- No lee ni maneja básculas.
- No abre el cajón desde el hub ni de forma manual («sin venta»): solo con un cobro, desde el
  dispositivo que cobró (HUB-F207).

## Dudas abiertas

Se resuelven con `market-decision`; no las decide el worker.

1. **Báscula** (SALES-F10, INVENTORY-F26 dicen «no existe»): la mitad de software está en `sales` y la
   mitad de hardware (leer la báscula y emitir `erplora:scale-weight`) es hub#1217, abierta con
   `prio:P1` y `area:hardware`; `TODO-MVP.md` la sitúa en «Profundidad de vertical», que se vende
   después del primer cliente. ¿Es un flujo `no hecho` de este documento ahora, o una línea de «Lo que
   NO hace» hasta el cliente que la pida?
2. ¿Entra el **logo del negocio** en el tique de la térmica dentro del MVP? Hoy no se imprime ninguna
   imagen y ningún módulo ni issue lo pide.
3. ¿Entra **Windows USB** (hub#1269) en el MVP, o basta la IP en esa plataforma?
4. ¿Debe la hora del papel ser la del documento (la venta) y no la de imprimir? Hoy es la del
   dispositivo; una reimpresión o un tique que espera en cola sale con otra.
5. ¿Debe cada impresora saber el ancho de su papel (58 o 80 mm) y componer el documento a él?

## Fuentes contrastadas

Contra `origin/develop` del hub (05/10/2026). Una línea por discrepancia; manda el código.

- **`architecture/hub/print-queue.md` («Tarde, no perdido»)** y el guion `qa-hub-restaurant` §10/§16
  («impresora sin papel/offline: el trabajo queda pendiente y la UI informa»): con una impresora de red
  del propio dispositivo apagada, la caja avisa y deja reintentar (hub#2494), pero el trabajo no
  queda pendiente en la cola y uno que venía de la cola del hub vuelve «fallido» sin aviso (F06).
- **`README.md` del crate y `usb.rs`** hablan de «un recibo con logo»: no hay imágenes en el
  renderizador (F16).
- **Comentario de `escpos.rs` («80 mm ≈ 32 caracteres»)**: en una térmica de 80 mm con la fuente A caben
  48 columnas; el crate compone a 32 siempre (F16).
- **`architecture/hub/crates/peripherals.md`** presenta el crate como «el camino vivo» de la cola; hoy
  lo recorren la impresión directa de la caja y los trabajos que el hub entrega a un dispositivo con impresora de red o Bluetooth, la hoja de prueba y el cajón; una USB solo recibe la hoja de prueba («Probar»): ni tiques, ni comandas, ni trabajos de la cola, ni el cajón al cobrar (F07, F15).
- **`hand-book/hub/11-app-instalada-y-hardware.md`** no nombra USB, Bluetooth, ni el cajón: la
  documentación para la persona sobre hardware es el capítulo del módulo Impresión.
- **Pantalla Impresoras** (`status`): el crate devuelve estados que la pantalla no traduce (`busy`,
  `error`, `offline` salen sin traducir; la pantalla solo traduce «ready», «stopped» y «unknown»).

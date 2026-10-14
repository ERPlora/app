# WORKFLOW — La aplicación instalada · Permisos y avisos

Prefijo: HUB_APP

## Flujos

### HUB_APP-F07 Android pide los permisos en su momento
Estado: parcial — en Android 12 a 16 el permiso de Bluetooth se pide en frío, sin frase previa, en cada búsqueda e impresión Bluetooth hasta que Android deja de mostrarlo (`bridge-transport.ts:100-108`); solo en Android 17 viaja con la red local detrás de la frase
Actor: administrador, responsable, empleado
Pantalla: Pedir permiso
Pasos:
1. Solo en Android. La persona hace algo que necesita un permiso: buscar impresoras, imprimir, añadir
   una impresora por IP (red local y Bluetooth), o dar de alta el puesto de impresión, abrir la primera
   comanda o cita, o **iniciar sesión** en un hub con cocina, citas o contador en la campana (notificaciones,
   `main.ts:489-498`).
2. El hub enseña **antes** una frase suya: «Vamos a buscar tu impresora» / «Deja que te avisemos», con
   «Ahora no» y el botón de permitir.
3. Si acepta, Android enseña su diálogo, **uno solo** aunque la operación necesite varios permisos
   (red local y Bluetooth viajan juntos, solo en Android 17); si dice «ahora no» a la frase, no se vuelve a pedir sola.
   En Android 12 a 16 no hay frase para Bluetooth.
4. La aplicación solo pide lo que la operación va a usar (el cajón y el tique piden red local o
   Bluetooth según el identificador de la impresora; los avisos, notificaciones).
Entra: la operación y la lista de permisos que Android conoce en esta versión: notificaciones desde la 13
(API 33), Bluetooth desde la 12 (API 31), red local desde la 17 (API 37). En versiones anteriores no se
pide lo que no existe (en algunos fabricantes colgaría el diálogo).
Sale: el estado real de cada permiso; un «no» **se contesta, no se rechaza**: sin impresora el TPV tiene
que seguir vendiendo. El NFC y el servicio en primer plano son permisos de instalación: no hay diálogo.
Si falla: sin el permiso de red local el barrido y el envío a la red son tiempos de espera, no errores
(Android los bloquea por debajo de la API): sin protección parecería «no hay impresoras» (HUB_APP-F13).
Después de dos «no» Android deja de mostrar el diálogo para siempre: la salida es HUB_APP-F08. Una página que
no es el negocio enlazado (otro negocio, la web pública, el entorno de pruebas) no puede preguntar ni pedir
ningún permiso: recibe `not_the_linked_hub` y Android no enseña nada (HUB_APP-F10, ERPlora/hub#2642).
Implicados: HUB_PERIPHERALS-F01, HUB_SHELL-F68, HUB_SHELL-F139, PRINTING-F02, REC_ALTA-F17
QA: qa-hub-android Fase 2, qa-hub-android Fase 3

### HUB_APP-F08 Volver a activar un permiso negado
Estado: hecho
Actor: administrador, responsable
Pantalla: HUB_SHELL: Sistema
Pasos:
1. Con la red local o los avisos negados, la pantalla lo dice («La búsqueda de impresoras está
   bloqueada» / «Los avisos están desactivados») y ofrece «Permitir la búsqueda» / «Activar los avisos».
2. Si Android aún deja preguntar, vuelve a salir el diálogo.
3. Si ya no, la pantalla ofrece «Abrir los ajustes»: la aplicación abre **su** página en los ajustes del
   sistema; la persona activa el permiso y vuelve.
Entra: el estado de los permisos (`check_permissions`, sin molestar).
Sale: la aplicación abierta en los ajustes; nada guardado.
Si falla: un dispositivo sin esa página (modo quiosco) o una aplicación más antigua que la orden rechaza,
y la pantalla dice dónde ir en vez de quedarse muda («Tu dispositivo no ha vuelto a preguntar. Abre sus
ajustes, busca ERPlora y concédele el acceso a la red local.»). En ordenador no hay estado: la ausencia
del permiso se lee como «no aplica», nunca como «negado». Una página que no es el negocio enlazado no puede
abrir los ajustes de la aplicación: recibe `not_the_linked_hub` (HUB_APP-F10).
Implicados: HUB_SHELL-F68, HUB_SHELL-F139, HUB_SHELL-F140
QA: qa-hub-android Fase 2

### HUB_APP-F09 Red local en ordenador (macOS y Windows)
Estado: parcial — en macOS 15 la aplicación no puede saber si el permiso se negó: la búsqueda vuelve vacía y la pantalla dice «No se encontraron impresoras de red…» en vez de «dale permiso a la app»
Actor: administrador, responsable
Pantalla: printing: Impresoras
Pasos:
1. **macOS 15 o posterior**: la primera vez que la aplicación habla con una impresora de la red local,
   el sistema pregunta con el motivo que declara la aplicación («ERPlora needs access to your local
   network to find and print to the receipt and kitchen printers in your business.»). La aplicación declara
   los dos servicios que busca (`_pdl-datastream._tcp`, `_ipp._tcp`).
2. Sin respuesta afirmativa, las conexiones expiran en silencio.
3. **Windows**: el paquete de la Store declara acceso a internet y a la red privada; el instalador
   normal no pide nada.
Entra: la respuesta de la persona al sistema.
Sale: nada guardado por la aplicación; el sistema recuerda la respuesta.
Si falla: negado en macOS, la búsqueda no encuentra nada y no hay señal que lo distinga de «no hay
impresoras»; y **imprimir por red también contesta «correcto» y el papel se pierde** (`lib.rs:1370-1394`, la
comprobación solo mira Android). Se arregla en Ajustes del sistema › Privacidad › Red local.
Implicados: HUB_PERIPHERALS-F01, REC_ALTA-F17
QA: ninguno

### HUB_APP-F24 Avisar con el sistema aunque nadie mire la pantalla
Estado: hecho
Actor: sistema
Pantalla: Aviso del sistema
Pasos:
1. Llega una comanda a la cocina, una cita que no salió de este TPV o sube un contador de la campana.
   La página (no la aplicación) decide avisar y manda título, cuerpo, un número de aviso y la pantalla a
   la que lleva.
2. En Android, la página pide antes el permiso de notificaciones (HUB_APP-F07); sin él **no manda el
   aviso** (para no abrir el diálogo de Android a pelo en pleno servicio).
3. La aplicación muestra el aviso del sistema: Windows, con un enlace propio para poder abrirlo desde el
   Centro de actividades; macOS y Linux, esperando el clic; Android, por el plugin de notificaciones.
Entra: título, cuerpo, número (si cabe en 32 bits) y ruta (solo pantallas del propio hub).
Sale: la notificación. **Nunca falla hacia arriba**: permiso negado o plataforma que no puede mostrarla se pierde sin rastro y la comanda o la cita siguen. Un número fuera de rango no impide el aviso, solo le quita su destino.
Si falla: un navegador sin aplicación no tiene aviso del sistema. Sin permiso negado no hay otra señal que
el que el aviso no sale (la fila «Los avisos están desactivados» de Sistema lo cuenta).
Implicados: HUB-F60, HUB_SHELL-F64, HUB_SHELL-F65, HUB_SHELL-F66, KITCHEN-F05
QA: qa-hub-restaurant §7.08

### HUB_APP-F25 Tocar un aviso
Estado: hecho
Actor: administrador, responsable, empleado
Pantalla: Aviso del sistema
Pasos:
1. La persona toca (o hace clic en) el aviso.
2. La aplicación se trae la ventana delante y la página abre **la pantalla de la que avisaba** (la
   conversación, la agenda, la cocina).
3. **Android con la aplicación abierta**: el plugin entrega el toque a la página. **Android con la
   aplicación cerrada o matada por el sistema**: el aviso guarda su destino, la aplicación arranca, y la
   página lo reclama al arrancar. **Ordenador**: el clic se guarda y la página lo reclama (la aplicación le
   avisa de que hay uno esperando). **Windows**: cada clic, también desde el Centro de actividades y con la
   aplicación cerrada, lanza la aplicación con un enlace propio (`erplora://notice?...`) que la ya abierta
   recibe.
Entra: el número del aviso y la ruta con que salió.
Sale: la pantalla abierta; el toque se entrega **una vez**. Si la página es nueva y no recuerda el número,
usa la ruta que el aviso trae, tras comprobar de nuevo que es una pantalla del hub.
Si falla: un aviso sin número solo trae la ventana al frente. Una ruta que no es de una pantalla del hub
(otra dirección, `//…`) se descarta. Un aviso que pasó sin clic (macOS, Linux) no abre nada. Solo oye los
toques la página del negocio enlazado: cualquier otra página de erplora.com que enseñe la ventana recibe
`not_the_linked_hub` al suscribirse (ERPlora/hub#2658); la del negocio que se suscribe al arrancar, antes de
terminar de cargar, lo reintenta hasta unos 8 s después de su `load`.
Implicados: HUB_SHELL-F64, HUB_SHELL-F67, KITCHEN-F05
QA: ninguno

### HUB_APP-F26 Seguir a la escucha con la pantalla apagada (Android)
Estado: parcial — solo se mantiene a la escucha si hay sesión, el hub tiene cocina, citas o un contador en la campana y los avisos no están negados: la tablet de cocina sí escucha, pero un puesto de bar o de caja en un hub sin cocina, sin citas y sin contador se congela con la pantalla apagada; negar los avisos después en los ajustes de Android no apaga la escucha; y si el sistema mata el proceso no vuelve sola
Actor: sistema
Pantalla: Aviso del sistema
Pasos:
1. Decisión vigente (ERPlora/hub#2307): **no hay push**; los avisos con la pantalla apagada van por tener la
   aplicación escuchando durante el turno.
2. Con sesión iniciada, algo que avisar (cocina, citas o un módulo con contador en la campana) y los avisos
   no negados, la página pide mantener la aplicación a la escucha con las palabras de la notificación en
   el idioma del hub.
3. Android arranca un servicio en primer plano y muestra la notificación permanente «ERPlora está a la
   escucha». Mientras dura, la aplicación mantiene viva la página: la desbloquea cuando la pantalla se
   apaga o la aplicación pasa al fondo (sin ello Chromium la congela al minuto) y los avisos de la
   comanda y la cita siguen llegando.
4. Al cerrar la sesión (`isAuthed=false`, `stopListeningWhenSignedOut` en `main.ts`), al cambiar de negocio o al
   olvidar uno borrado (antes de olvidarlo, HUB_APP-F04 y F05), al destruirse la actividad o al cerrar la aplicación
   desde recientes, se para. **No** se para al negar los avisos después.
Entra: la petición de la página (activar con textos o desactivar).
Sale: el servicio y su notificación silenciosa, sin sonido ni insignia. No se reinicia solo
(`START_NOT_STICKY`): lo reabre la página al abrir la aplicación.
Si falla: Android 12 o posterior no deja arrancar un servicio en primer plano desde el segundo plano: la
orden rechaza y los avisos siguen funcionando con la aplicación en pantalla. Sin los permisos de servicio
(Android 14) lanza error. En ordenador no se necesita: la aplicación no se duerme mientras esté abierta.
Una página que no es el negocio enlazado no puede encender ni apagar la escucha: recibe `not_the_linked_hub`
(HUB_APP-F10, ERPlora/hub#2642).
Implicados: HUB-F60, HUB_SHELL-F69, HUB_SHELL-F139
QA: qa-hub-android Pendiente (pantalla apagada y segundo plano)

# cac — lo que hay que saber antes de tocar

Monorepo: `app/` (Tauri 2 + React 19, el cliente de escritorio), `core/` (Rust,
la lógica que comparten todas las apps, sin interfaz), `mcp/` (Rust, el servidor
MCP y su binario `cac-mcp`, sin interfaz), `backend/` (Go +
chi + GORM + Postgres, la API en `cac.guz-studio.dev`), `swarm-manage/` (agente
por host), `infra/`.

Los comentarios del código de este repo son buenos y explican el *por qué*. Aquí
sólo va lo que no cabe en un comentario porque no tiene un sitio único donde
vivir. Lo demás, léelo donde está.

## El método de la casa: mutar toda prueba nueva

Una prueba que no se ha mutado no se sabe si prueba algo. Escribe la prueba,
rómpele el código a propósito, y comprueba que la prueba se queja. Es lo que ha
encontrado los fallos de verdad en este repo — hay comentarios que lo dicen por
su nombre (`meeting_rule_test.go`, `report_project_patch_test.go`,
`useEncogerEnLlamada.test.tsx`, `locale-sync.test.ts`).

Hay una skill para el ritual: `/mutar`. **Un mutante vivo se investiga, no se
ignora.**

## Las trampas que ya han mordido

**Hay dos máquinas de estados, no una.** `ReportStatus.CanTransitionTo` sólo
conoce el estado, así que sólo puede contestar por la del **cliente**, que es
estricta a propósito. Lo que hay que preguntar es
`domain.CanTransition(ficha.Flow(), from, to)`. Aplicarle la estricta a una tarea
interna deja botones que no hacen nada y no dicen por qué — así se murió el check
de las subtareas. Ver `backend/internal/core/domain/report.go` (sección «Dos
máquinas, y por qué»).

**En el flujo del cliente, `pending → resolved` no existe**: hay que pasar por
`in_progress`. Mover directo devuelve 409. En el interno se puede ir de cualquier
sitio a cualquier sitio, y **cerrado no es terminal**.

**Una subtarea siempre es interna**, porque `CreateTask` le borra el `ProjectID`
al crearla (no gastar un folio del cliente en una línea de checklist). De eso
depende que el check de «hecho» funcione: salta de Open a Done, que sólo es legal
en el flujo interno.
→ Guardián: `domain.TestUnaSubtareaEsInterna` y `service/subtarea_interna_test.go`.

**Un padre no se termina ni se cierra con subtareas abiertas**
(command-and-control-387). «Abierta» es la que no está ni hecha ni cerrada;
vale en las dos máquinas y por las dos puertas que cambian un estado
(`TaskService.MoveTask` y `ReportService.Update`), que contestan 409
`subtasks-open` con el motivo y la cuenta. Reabrir se puede siempre. Hasta el
17-nov-2026 era una opción de la organización, sólo para «hecho», y cerrar era
la salida: el campo `DoneNeedsSubtasksDone` sigue en la base y ya no decide
nada. Y un «no» nunca es mudo: el core da el motivo (`status_refusal_phrase`)
y quien pinta lo enseña.
→ Guardianes: `domain.TestOpenSubtasksBlockDoneAndClosed`,
`service/subtasks_open_test.go`, y en el core `open_subtasks_block_what_the_server_blocks`
(la misma tabla, escrita por Go en `state_machines.json`).

**`dueAt` es una fecha guardada como instante.** Léela con `dueDay`
(`app/src/lib/month.ts`). Con los captadores locales (`getDate()`, etc.) el día se
corre un día al oeste de Greenwich. `dayKey` **también** es local: no la
uses sobre un instante crudo. Así pintó el calendario de «Mi trabajo» en el 29
lo que se soltó en el 30. Por eso `CalendarItem` obliga a elegir entre `day`
(una fecha) y `at` (un instante): un vencimiento va como `day`; pasarlo como
`at` compila y vuelve a fallar.
→ Guardianes: `app/src/lib/month.test.ts` y `ItemCalendar.test.tsx` («en qué
día cae»).

**Las pruebas de la app corren en `America/Mexico_City`**, fijado en el script
`test` (no en `vitest.config.ts`: Node lee la zona al arrancar y `test.env`
llega tarde). Sin eso corrían en la zona de la máquina, y en UTC un fallo de
zona es invisible. Si tocas fechas, pasa `bun run test:timezones`: la suite bajo
UTC, UTC−6 y UTC+13. Con una sola zona, un fallo simétrico al revés sigue
escondido — el primer barrido encontró uno en `meeting-time.test.ts`.

**Un token personal es la persona para los scopes, pero no para firmar.** Sus
claims copian `Superadmin` del usuario, así que «¿es superadmin?» no distingue
a la persona de un agente con su token. Lo que tiene que hacer una persona en
persona —hoy, firmar la revisión de un doc— se decide con `ViaToken`, que sólo
pone la rama PAT de `AuthMiddleware` y nunca viaja en JSON. Hasta el 27-sep-2026
un comentario aseguraba que ningún token podía firmar, y el de un superadmin
firmaba (#84).
→ Guardianes: `domain.TestDocReviewSigner`, `TestViaTokenNeverTravelsInJSON` y
`middleware.TestAPersonalTokenIsMarkedAsOne`.

**La prioridad media se guarda `medium` y se contesta `normal`.** `normal` es
sólo como la pide y la devuelve la API de tareas (`TaskWire`); en la base es
`medium`. Una consulta que ordene o filtre por prioridad pregunta por el nombre
**guardado**. El orden de «mi trabajo» preguntaba por `normal` y ponía casi todas
las tareas por debajo de las de prioridad baja; y editar guardaba la entrada
cruda, así que convivían los dos nombres (#83). Todo lo que escribe prioridad
pasa por `Canonical()`.
→ Guardianes: `service.TestMyWorkPutsMediumBetweenHighAndLow`,
`TestEditingAPriorityStoresWhatCreatingStores` y
`repository.TestNormalPrioritiesBecomeMediumAndNothingElseMoves`.

**Un nombre para enseñar sale de `nombreVisible`**
(`backend/internal/core/repository/nombres.go`), nunca de `username` a mano: un
identificador de acceso no es cómo se llama a una persona. Buscar, mencionar e
identificar sí van por `username`.
→ Guardián: `repository.TestNadieResuelveElNombreASuAire` y `nombres_sql_test.go`.

**Un egress muerto sigue diciendo que está vivo.** Si el pod de `livekit-egress`
se cae, LiveKit deja el registro en `EGRESS_ACTIVE` **para siempre**: ni el
estado, ni `updated_at` —que no late ni con el egress sano—, ni la presencia del
participante en la sala lo delatan. Las tres se midieron contra el SFU. Lo único
que lo distingue es pedirle que pare: 408 `deadline_exceeded` si no hay nadie,
412 `failed_precondition` si ya terminó. Y como preguntar **es** parar, sólo se
pregunta al cerrar; mientras se graba, un pod que muere no se detecta y se
acepta. Contar el 412 como muerte tira grabaciones buenas.
→ Guardianes: `service.TestADeadEgressWorkerDoesNotHangTheRecording` y
`TestAStaleListingDoesNotKillAFinishedTrack`. El porqué entero, con los números,
en `docs/grabacion.md`.

**La medida de lectura de un documento (68ch) es para la prosa**, no para tablas
ni bloques de código: puesta al cuerpo entero, una tabla ancha «cabe» partiendo
las palabras a mitad y su propio deslizamiento no se activa nunca.
→ Guardián: `app/src/components/markdown/enlaces.test.tsx` («la medida de
lectura de un documento»).

**Un aviso del sistema no le pregunta al foco.** Tener la ventana con el foco no
es estar mirando eso. La puerta que había («si tiene el foco, no avises») está
quitada a propósito; no la vuelvas a poner.
→ Guardián: `app/src/hooks/avisos-del-sistema.test.tsx` («la puerta del foco»).

**El rango de un tablero es un `NUMERIC`, y tiene que seguir siéndolo.** Hasta
el 10-sep-2026 era texto en base 62 (`0-9A-Za-z`) y lo ordenaba Postgres con
`ORDER BY rank`, así que el orden dependía de la colación: con una de locale
(glibc `en_US.utf8`) las cajas se entremezclan y **el segundo elemento de un
contenedor se pinta antes que el primero** —los primeros rangos eran «U» y «k»—.
Producción estaba en `C` y acertaba, pero nada en el esquema lo pedía. A un
número no hay colación que aplicarle. Devolver la columna a `varchar`
reintroduce el fallo entero **sin romper ninguna otra prueba**, porque en una
base `C` seguiría saliendo bien.
→ Guardián: `repository.TestEveryRankColumnIsNumeric`. Y una fila sin rango no
revienta la inserción porque la columna tiene `default:0.5`; los caminos que
saben dónde va la tarjeta le ponen el suyo.

**El webview de Linux es WebKitGTK, y no se porta como Chrome.** Lo que
funciona en la web (o en WhatsApp Web) puede no funcionar en la app. Soltar un
fichero desde Thunar escribía su ruta en el mensaje por tres cosas que **no
están documentadas juntas en ningún sitio**: WebKit no entrega el `drop` si el
`dragover` no dice `dropEffect = "copy"` (ProseMirror lo cancela, pero no basta);
deja `text/uri-list` **vacío** (no enseña rutas `file://` a una página); y la
ruta sólo viaja como **texto** de un `<a>` sin `href` en el `text/html`. Los
bytes los lee Rust (`read_dropped_file`), sólo de tipos que se adjuntan y
mirando la ruta real (un enlace simbólico `foto.png` → `~/.ssh/id_rsa` no
pasa). Ante algo así, **un log temporal primero** de lo que llega de verdad, no
hipótesis.
→ Guardianes: `app/src/lib/dropped.test.ts` (lleva el HTML real que entregó
WebKitGTK) y `dropped::tests` en `mcp/src/dropped.rs` (leer el fichero no
necesita Tauri y lo usa también el MCP; el comando de la app sigue en `lib.rs`).

**Pegar con el portapapeles sin dueño tumba WebKitGTK 2.54.** Si nadie contesta
(vacío, la app que copió ya se cerró, CopyQ cambiando de dueño), GTK responde a
la petición de formatos con un puntero nulo y `-1`, y `WebKit::Clipboard::formats`
lee la dirección 0: SIGSEGV de la ventana entera, antes de que la página vea el
`paste`. Se ataja en GTK (`paste_guard.rs`): al pulsar pegar se pregunta por los
formatos con la llamada síncrona y, si no hay, se descarta la tecla.
→ Guardián: `paste_guard::tests` (las teclas de pegar y la decisión de bloquear).

**Lo que la app lanza del sistema hereda la AppImage.** Dentro de la AppImage,
`LD_LIBRARY_PATH` y otra veintena de variables apuntan a las bibliotecas de la
imagen (Ubuntu 22.04). Un programa del sistema que las herede carga esas en vez
de las suyas, y tras un `pacman -Syu` el `bash` nuevo muere con la `readline`
vieja (`undefined symbol: rl_trim_arg_from_keyseq`). Así se murió el relanzado
tras actualizar, el Python de Ansible y, el 21-oct-2026, **todo enlace
externo**: el plugin `opener` lanza `xdg-open`, que es un script de shell. Todo
lanzamiento pasa por `host` (`host::command`, `host::open_url`,
`host::open_path`), nunca por `Command::new("…")` ni por el plugin. Limpiar el
entorno vive en `mcp/src/host.rs` (el MCP corre dentro de la misma AppImage con
`cac --mcp` y lanza `git`, `gs`, `op` y Ansible); abrir enlaces y ficheros, que
es de la app, en `app/src-tauri/src/host.rs`.
→ Guardián: `host::tests::every_system_program_is_launched_through_here`, con
ese nombre en los dos crates y mirando cada uno las fuentes de los dos (el de
`mcp/` es el que corre en el CI), y `app/src/lib/platform.test.ts` para los
enlaces.

**Un esquema propio es otro origen.** `cacmedia://` y `cacvideo://` los sirve
Rust, y para el webview son **otro origen**: un `<img>` carga igual, pero un
`fetch` sin `Access-Control-Allow-Origin` falla con «estado 0». pdf.js descarga
con `fetch`, así que el visor de PDF del escritorio **no abrió nunca nada** hasta
el 6-oct-2026. Toda respuesta de un esquema propio lleva la cabecera, también
las de error.
→ Guardián: `media::tests::every_answer_can_be_read_by_a_fetch`.

**La CSP del escritorio bloquea en silencio.** Desde la v1.6.89 la app tiene
CSP (`app/src-tauri/tauri.conf.json`). Algo que no esté permitido no da error:
simplemente no carga. Cada fuente permitida está atada a lo que la usa; si
añades un sitio nuevo al que la app hable o del que cargue, añádelo ahí y en la
prueba, y pruébalo con la app abierta, no sólo con vitest.
→ Guardián: `app/src/lib/desktop-csp.test.ts`.

**Un contenedor que se llena a mano no puede tener hijos de React.** El visor
de PDF dibujaba las páginas en el mismo `<div>` donde React ponía «Cargando…»;
al vaciarlo se llevaba ese `<p>` y React fallaba después con
`NotFoundError: The object can not be found here.` Lo mismo con montar React
dentro de una decoración de ProseMirror durante el render de otro árbol: se
monta en una microtarea, después.
→ Guardianes: `app/src/components/PdfPreview.test.tsx` y
`app/src/components/markdown/pdf-cards.mount.test.tsx`.

## El CI corre las pruebas de backend, app, transcriptor, agente y MCP

Corren **por delante** de lo que publican, así que rojo no sale:

| Workflow | Qué corre | Qué frena |
|---|---|---|
| `backend.yml` | `go test` con Postgres de verdad | el despliegue a producción (#38) |
| `app.yml` | tipos, eslint, la suite **bajo las tres zonas**, y `cargo test` del core (`core/`) | la release: `app-release.yml` lo llama antes de compilar (#69) |
| `transcriber.yml` | `pytest` | la imagen del worker |
| `swarm-manage.yml` | `go test` del agente | su imagen (`:vN` desde `swarm-manage/VERSION`) |
| `mcp.yml` | `cargo test` (incluye el script de instalar, corrido de verdad), el binario `cac-mcp` en release mirando con `ldd` que no enlaza interfaz, y el mismo binario estático con musl | la release del MCP: `mcp-release.yml` lo llama antes de compilar |

Al encender cada una salió algo que llevaba tiempo escondido: una prueba del
backend roja desde hacía meses, y en la app un error huérfano de tiptap que hacía
salir la suite con código 1 con todo en verde (tapado en `test-setup.ts`).

**Sin red sigue** el lado Rust de la app (`app/src-tauri`; el core y el MCP sí
corren), que no tiene pruebas propias en el CI: lo único que lo comprueba es que
la release compile en las tres plataformas. Eso incluye `tests/mcp_mode.rs`, la
prueba de que `cac --mcp` sigue sirviendo: corre en local con la puerta de abajo.

Una release con `app.yml` en rojo **se queda publicada sin instaladores**. El
actualizador no se entera, así que nadie se la come, pero esa versión ya está
gastada: se arregla y sale la siguiente.

Y ojo con el verde de `go test` **en local**: las pruebas que necesitan Postgres
se **saltan** sin `DB_HOST` (`t.Skip("no database configured")`), y son casi
sesenta ficheros. Sin `DB_HOST` puesto, un verde no significa que se hayan
corrido todas — en CI ya no puede pasar, en tu máquina sí. **Con `DB_HOST`
puesto, una base que no contesta falla, no se salta**: hasta el 21-oct-2026 se
saltaba en silencio, y con la base de pruebas apagada diez horas un verde no
había corrido ninguna.
→ Guardián: `repository.TestADatabaseThatDoesNotAnswerFailsInsteadOfSkipping`.

## El agente de cada servidor tiene identidad (desde su v2)

`swarm-manage` abre el socket de Docker de la máquina: su API es la puerta de
toda la VPS. Desde la v2 **no abre `/api/v1` sin identidad** —un token por
servidor que acuña el backend y una llave de sesión derivada, instalados como
Docker secrets por stdin— y cada petición de la app lleva un pase firmado. No
hay modo abierto al que caer; sin secrets contesta 503.
→ Guardianes: `TestNoAgentRouteWithoutAuth` (recorre el router del agente con
`chi.Walk`), `TestSessionVector` / `TestAgentSessionVector` (el **mismo**
vector en el agente y en el backend: si cambias uno, cambia el otro) y
`la_imagen_es_la_version_del_agente` (en `ops/`: las dos apps instalan la
imagen que publica el workflow; y `the_latest_version_is_the_one_published` en
el core, la versión con la que la app nativa dice si un agente quedó atrás).

Y el agente **actualiza servicios con `service update` por la API de Docker,
nunca con `stack deploy`**: un stack que creó el CI de un proyecto no se recrea
y conserva labels, redes y secrets. Es lo que permite que cada proyecto se sume
a los despliegues por cac cuando quiera.

## El MCP es su propio crate (`mcp/`), y la app lo usa por ruta

Desde command-and-control-388 el servidor MCP no vive en la app de Tauri: es el
crate `mcp/` (`cac_mcp`), que compila el binario **`cac-mcp`** sin Tauri, GTK ni
WebKit. La app depende de él por ruta, así que `cac --mcp` sirve lo mismo desde
la misma fuente y las instalaciones que apuntan a la app no cambian. **No se
copia nada entre los dos**: lo que comparten (Ansible, `op read`, el agente SSH,
comprimir imágenes y PDF, leer un fichero soltado, limpiar el entorno de la
AppImage) vive en `mcp/src/` y la app lo toma de ahí; en `app/src-tauri` queda
la capa de Tauri (los comandos `ansible_*`, abrir enlaces).

- **Nada que pinte entra en `mcp/`.** Ni directo ni de la mano de otra
  dependencia. → Guardián: `mcp/tests/no_ui.rs` (mira `Cargo.toml` y
  `Cargo.lock`); y `mcp.yml` lo mide sobre el binario con `ldd`.
- **Quién sirve se dice al arrancar** (`cac_mcp::Host`): el binario anuncia su
  versión (`mcp/Cargo.toml`) y `cac --mcp` la de la app, como siempre. Y sólo la
  app se fía de `APPIMAGE` y `OWD`: al binario suelto esas variables le llegan
  heredadas de otro programa, y vigilaría la AppImage de un editor para avisar
  de que «cac se actualizó».
  → Guardianes: `server::tests::only_the_desktop_app_trusts_the_appimage_variables`,
  `each_host_announces_its_own_version`, `mcp/tests/stdio.rs` (el binario,
  arrancado de verdad) y `app/src-tauri/tests/mcp_mode.rs` (`cac --mcp`, ídem,
  contra el mismo catálogo).
- **El MCP sólo llega a Ansible por `provision_cli::request`**, que sólo ensaya.
  Ansible está ahora en el mismo crate, así que la frontera es esa prueba y no
  la distancia. → Guardianes: `server::tests` (la de las tres herramientas de
  provisioning) y `provision_cli::tests::a_request_only_ever_dry_runs`.
- Las skills que instala (`mcp/skills/`) y los hooks (`.claude/hooks/`) van
  dentro del binario con `include_str!`.

### Repartir `cac-mcp`

Una sola lógica, en `mcp/src/update.rs`, para saber qué versión hay, bajarla,
comprobar su SHA-256 contra el `SHA256SUMS` publicado y ponerla en su sitio
(renombrando, nunca escribiendo sobre el binario en marcha; en Windows el viejo
se aparta a `.old`). La usan `cac-mcp update`, el aviso al agente de que hay
una versión nueva y la app nativa (`desktop/src/connect.rs`, «Conectar Claude
Code» en el menú de la cuenta); `scripts/install-mcp.sh` y `install-mcp.ps1`
hacen lo mismo para quien no tiene nada. **Nada se instala solo**: se avisa, y
actualiza quien lo pide.

- **Una release del MCP nunca se marca «latest».** Sale de este mismo repo con
  la etiqueta `mcp-vX.Y.Z` (`mcp-release.yml`, sólo a mano), y el actualizador
  de la app de Tauri lee `releases/latest/download/latest.json`: con una del
  MCP como última, esa dirección da 404 y **nadie actualiza la app**, sin un
  error a la vista. Por eso se publica con `--latest=false`, y por eso la
  última versión del MCP se lee de **`mcp/VERSION`** en `main` (por
  `raw.githubusercontent.com`), no de `releases/latest`.
  → Guardián: `mcp/tests/release.rs` (`an_mcp_release_is_never_marked_as_the_latest`).
- **`mcp/VERSION` y `mcp/Cargo.toml` se suben juntos**, y es la versión del
  MCP, no la de la app: tiene su propia línea desde 1.33.0. Una versión
  publicada no se re-corta (el workflow se niega si la etiqueta existe).
  → Guardián: `the_version_file_and_the_manifest_agree`, ahí mismo.
- **Los nombres de los ficheros están en cuatro sitios** (`update::ASSETS`, el
  workflow y los dos scripts) y tienen que decir lo mismo.
  → Guardián: `the_workflow_the_scripts_and_the_binary_name_the_same_files`.
- **Sólo el binario suelto pregunta por sus versiones** (`Host::updates_itself`):
  una vez al día, en un hilo aparte, recordado en su carpeta de estado, y se
  calla con `CAC_MCP_NO_UPDATE_CHECK=1`. A `cac --mcp` lo actualiza la app.
  → Guardianes: `mcp/tests/updates.rs` (con el binario de verdad contra una
  release de mentira) y `server::tests::only_the_standalone_binary_watches_for_its_own_updates`.
- **Ninguna prueba sale a internet**: `update::fake::FakeReleases` sirve
  `VERSION`, los binarios y las sumas en esta máquina, y las direcciones se
  cambian por el entorno (`CAC_MCP_VERSION_URL`, `CAC_MCP_DOWNLOAD_BASE`). El
  script de sh se prueba de verdad (`mcp/tests/install_script.rs`); **el de
  PowerShell no se ha corrido nunca**: sólo se comprueba que nombra lo mismo.
- La suma protege de una descarga rota o cambiada por el camino. **No** de
  alguien que pueda publicar en el repo: nada va firmado.

## La puerta de verificación

Entera, antes de dar algo por hecho. Desde la raíz del repo, o sólo lo que
puede romper lo que tocaste con `scripts/test-changed.sh` (mira el diff
contra la rama de arriba y corre esas suites enteras; `--plan` dice cuáles,
`--quick` acota el escritorio a los módulos tocados para una primera pasada).
Lo que depende de qué está escrito en el script y guardado en
`app/src/lib/test-changed.test.ts`:

```sh
(cd app           && npx tsc --noEmit)                            # tipos
(cd app           && bun run test)                                # eslint + vitest
(cd core          && cargo test && cargo check --features uniffi) # el core compartido
(cd mcp           && cargo test && cargo build --release)         # el MCP, su binario y los scripts de instalar
(cd voice         && cargo test --locked)                         # el motor de llamadas (LiveKit), sin audio ni red
(cd desktop       && cargo test --locked)                         # la app nativa (usa mcp/ y, en Linux, voice/ por ruta)
(cd app/src-tauri && cargo check)                                 # el lado Rust
(cd backend       && go build ./... && go vet ./... && go test ./...)
python3 .claude/hooks/hooks_test.py                                 # los hooks de cac
```

**Los hooks de cac** (`.claude/hooks/`) corren en cada sesión de Claude Code:
al arrancar meten en el contexto el espacio, lo que está en curso con su
«dónde quedó» y las trampas abiertas; en cada prompt que nombra un servidor o
un área que ya mordió (`gstd`, `cert-manager`…), le ponen delante esos
incidentes, una vez por sesión (`cac-prompt`); al editar un fichero, los
incidentes cuya prueba guardián vive ahí (`cac-edit`, por la ruta, el nombre,
el módulo de Rust o el paquete de Go del guardián; la misma «una vez por
sesión» que `cac-prompt`); cada vez que el agente escribe en cac, una línea
para quien mira la terminal con qué tarea y qué cambió (`cac-wrote`, en
`systemMessage`); y al parar no dejan irse sin
escribir el handoff de la tarea que nombran los commits de la sesión (o, sin
commits que la nombren, la que se tocó en ella). Para el resto de repos, el MCP
los instala globales con `install_hooks` (scripts embebidos desde aquí, en
`mcp/src/hooks_install.rs`); en este repo la copia global calla. **Si cambias un hook,
sube `# cac-hooks vN` en `cac_hooks.py`**: es lo que hace que se ofrezca
actualizar la copia global. El mismo `settings.json` está en la
raíz, en `app/` y en `backend/` porque Claude Code sólo lee el de la carpeta
donde se abre; los scripts buscan la raíz con git.

## Soltar una release

Hay skill: `/soltar`. El orden importa y no perdona: **el backend despliega antes
que la app**. Al revés, una app nueva habla con un servidor que no la entiende y
quien actualiza rápido se come el rato. La versión es siempre la **siguiente**;
nunca se re-corta una publicada.

**El número lo da `scripts/next-version.sh`, no la cabeza.** Es versionado
semántico sobre los prefijos de los commits: algún `feat` sube la del medio
(1.7.0), sólo arreglos la última (1.7.1), y algo que rompe la primera. Una
release sale con la feature **terminada**, juntando, no por tanda. Hasta la
v1.6.105 todo salió como parche.
→ Guardián: `app/src/lib/next-version.test.ts`.

## Tareas en cac

El trabajo de este repo se registra en la organización «Command and control»
(la skill `cac-work` explica cómo; el MCP de cac ofrece instalarla):

- **App** → `ca0bfd49-0909-43eb-8135-bc8ecd0f282c` (lo de `app/`)
- **Backend** → `91abe433-4d90-4519-93ff-616109ce40c9` (lo de `backend/`, `infra/` y `swarm-manage/`)
- **MCP** → `72fcff15-338a-4707-a52d-08af6191e10d` (el servidor MCP y la skill `cac-work`)

Las tareas del espacio se nombran `command-and-control-<número>`, en minúsculas:
así van al final de cada commit para que cac los enlace solos (`get_task` da
la `ref` lista). `cac#N` se sigue entendiendo, pero ya no se escribe.

## Convenciones

- **El código se escribe en inglés y se comenta en castellano.** Ficheros,
  funciones, tipos, variables y campos: inglés. Comentarios y prosa: castellano.
- **No imitar los nombres en castellano que ya existen.** Hay muchos
  —`nombreVisible`, `bandeja.ts`, `subtarea_interna_test.go`,
  `TestElTimbreSuenaEnUnSoloEscritorio`— y son deriva, no la convención. Copiar
  lo de alrededor es exactamente cómo se sigue mezclando. En código nuevo,
  inglés sin excepciones; y lo que ya está se renombra **al pasar por el
  fichero**, no en una barrida.
- **El texto que lee una persona no es código**: va en su idioma, y vive en los
  catálogos de `app/src/locales/`, nunca en un identificador.
- Commits de una sola línea, sin trailers ni líneas de atribución, firmados.
- Se commitea directo a `main`.
- `STATUS.md` es el doc de seguimiento: actualízalo al empezar o terminar algo,
  o al cambiar de rumbo.

## Y esto no obliga a nada

Un `CLAUDE.md` es una nota, no un guardián. Lo que obliga son las pruebas — por
eso las reglas de arriba que ya tienen una la citan al lado: para que quien las
cambie sepa qué se va a romper y por qué existe. Si añades una regla aquí sin
prueba, es un recordatorio; si te importa de verdad, escríbele la prueba.

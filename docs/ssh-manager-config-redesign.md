# SSH Manager — Rediseño a `~/.ssh/config` como fuente única

> Plan acordado para retomar en sesión nueva. Rama: `fix/ssh-manager-review`.

## Objetivo

Eliminar el doble almacén actual (SQLite interno = verdad + `~/.ssh/config` solo
importable) y usar **`~/.ssh/config` como fuente única**: cargar, conectar,
editar y crear entradas, persistiendo en el propio archivo — incluyendo
**grupos/carpetas, iconos y colores** como metadatos en comentarios,
**interoperables con la app SSH Config Editor (SCE)** que ya usa el usuario.

## Decisiones tomadas

1. **Fuente de datos: el config como única fuente.** Se retira SQLite. Pero
   **NO** se pierde la organización: grupos, iconos y colores se almacenan como
   comentarios `#SCE…` dentro del config (ver esquema abajo). Se replica el
   formato de **SSH Config Editor** para mantener compatibilidad bidireccional
   (el usuario sigue pudiendo usar SCE y Zap sobre el mismo archivo).
   - Se descartan OneKey y el clonado (no forman parte del formato SCE).
2. **Autenticación: clave + contraseña en llavero.** `HostName/User/Port/IdentityFile`
   se escriben en el config; las **contraseñas siguen en el Keychain de macOS**
   indexadas por host (ya existe `KeychainSecretStore` en
   `crates/warp_ssh_manager/src/secrets.rs`). Nunca secretos en texto en el config.
3. **Color → pestaña.** El color del host (`#SCETags`) se usa como **color de la
   pestaña del terminal al conectarse** a ese servidor.

## Esquema de metadatos de SSH Config Editor (SCE)

Decodificado del `~/.ssh/config` real del usuario. Todo son comentarios, así que
`ssh` los ignora; solo SCE (y, tras este trabajo, Zap) los interpretan.

| Metadato | Formato | Ejemplo | Ubicación |
|---|---|---|---|
| Grupo (carpeta) | `#SCE_GROUP:<UUID>:::<Nombre>` | `#SCE_GROUP:53B0A32B-…:::Dvuelta` | línea suelta (rodeada de líneas en blanco), antes de los hosts del grupo |
| Pertenencia a grupo | `#SCEGroup <UUID>` | `#SCEGroup 53B0A32B-…` | dentro del bloque `Host`, tab-indentado |
| Icono | `#SCEIcon <nombre>` | `#SCEIcon ubuntu` \| `home` \| `server` | dentro del `Host` |
| Color/etiqueta | `#SCETags <Color>` | `#SCETags Purple` | dentro del `Host` |
| Descripción | `# <texto>` | `# Servidor Kafka` | comentario en la línea anterior al `Host` |

Notas del formato observado:
- El UUID es un UUID v4 en mayúsculas; lo genera SCE. Zap debe generar uno igual
  al crear un grupo nuevo.
- Las directivas `#SCE…` van **al final del bloque** `Host`, tras las directivas
  SSH reales (`User, HostName, Port, IdentityFile, LocalForward, ProxyJump`).
- Hosts sin `#SCEGroup` quedan "sin grupo" (raíz), p.ej. `github.com`.
- Colores `#SCETags`: nombres tipo etiquetas de macOS Finder
  (`Red, Orange, Yellow, Green, Blue, Purple, Gray`, posiblemente `None`).
  Hace falta un mapa nombre→color para pintar fila y pestaña.
- Iconos `#SCEIcon`: nombres propios de SCE (`ubuntu, home, server, …`).
  Mapear a iconos disponibles en Zap (`crates/warp_core/src/ui/icons.rs`) con
  fallback genérico cuando no haya equivalente.
- Directivas SSH repetibles dentro de un host (varios `LocalForward`) deben
  conservarse todas (ver `dos2locos.es`, `awsprebios01`).
- `ProxyJump` aparece (`awsprebios01`, `ecs-valorindirecto-services`): soportar.

## Estado actual del código (investigado)

- **Leer config**: ✅ `crates/warp_ssh_manager/src/ssh_config_parser.rs`
  (`parse_ssh_config`, `load_candidates`, `default_ssh_config_path`). Pero
  **ignora los comentarios `#SCE…`** (solo extrae directivas SSH) → hay que
  extenderlo para leer grupo/icono/color/descripción.
- **Conectar desde config**: ✅ `crates/warp_ssh_manager/src/ssh_command.rs`
  (construye `ssh <alias>`).
- **File picker de clave privada**: ✅ **ya existe** en
  `app/src/ssh_manager/server_view.rs` (`open_key_file`, abre el explorador del
  sistema y escribe la ruta en el editor). Reusar, no recrear.
- **Escribir/editar/crear en config**: ❌ **no existe**. `sync_config.rs` es
  "Config → node only. We never write back to ~/.ssh/config." Toda la
  persistencia va hoy a SQLite (`db.rs`, `repository.rs`).
- UI: `app/src/ssh_manager/panel.rs` (~2943 líneas) y `server_view.rs` (~2890).
- Color de pestaña: investigar el sistema de pestañas/terminal (`app/src/tab.rs`,
  workspace/pane) para ver cómo asignar un color por pestaña al abrir la conexión.

## Plan por fases

### Fase 1 — Modelo + writer round-trip de `~/.ssh/config` (fundacional, TDD)
Pieza que falta y de la que depende todo. En `warp_ssh_manager`:
- Ampliar el parser a un **modelo editable que preserve** comentarios, orden,
  indentación, directivas no reconocidas y bloques `Host`/`Match`, **y que
  además lea/escriba los metadatos SCE** (`#SCE_GROUP`, `#SCEGroup`, `#SCEIcon`,
  `#SCETags`, descripción).
- API: grupos (`create_group`, `rename_group`, `delete_group`),
  hosts (`upsert_host`, `remove_host`, `rename_host`, `set_group/icon/color`).
- Escritura **atómica** (temp + rename), permisos `600`, backup previo.
- Directivas SSH: `HostName, User, Port, IdentityFile, ProxyJump,
  LocalForward/RemoteForward/DynamicForward` (repetibles preservadas).
- Tests: round-trip idempotente sobre el config real del usuario; upsert no
  destruye comentarios, `Include`, ni metadatos SCE; crear/editar/eliminar host
  y grupo; mover host entre grupos.

### Fase 2 — Repointar el panel al config (con grupos)
Sustituir `SshRepository` por lectura del config. El listado se **agrupa por
`#SCE_GROUP`** (carpetas colapsables), con icono y color por host. Fusionar la
sección de candidatos con el listado (ya son lo mismo).

### Fase 3 — Editor sobre el config (`server_view`)
- *General*: HostName, User, Port, **IdentityFile con el file picker existente**,
  contraseña (Keychain), **grupo** (selector), **icono** (selector), **color**
  (selector de etiqueta).
- *Port forwarding*: mapear a `LocalForward/RemoteForward/DynamicForward`.
- *Save* → escribe directivas + metadatos SCE en el config + password→Keychain.
  *Nuevo* → bloque `Host` nuevo (con `#SCEGroup` si aplica). *Play* → `ssh <alias>`.

### Fase 4 — Color de pestaña al conectar
Al abrir la conexión, propagar el color (`#SCETags`) del host a la **pestaña del
terminal**. Requiere mapa nombre→color y enganche con el sistema de pestañas
(`app/src/tab.rs` / workspace). Color por defecto si el host no tiene etiqueta.

### Fase 5 — Eliminar SQLite y código muerto (REDEFINIDA)
> **Cambio de rumbo (sesión 2026-06-21):** NO se elimina la sincronización. Se
> **reorienta** para sincronizar el `~/.ssh/config` (ver "Rediseño del sync"
> abajo). Por tanto SQLite se retira de la **capa de datos/panel**, pero el sync
> deja de depender de SQLite en vez de desaparecer.

Eliminar: `db.rs`, `repository.rs`, migraciones, `sync_config.rs`, onekey,
`candidates`, `on_clone_server`, `folders` en DB, variante `AuthType::OneKey`.
Sustituir `DbVersionStore` por un `FileVersionStore` (versión + meta de sync en
un fichero del directorio de datos de la app o en settings, sin SQLite).
Repuntar a `~/.ssh/config`/Keychain los consumidores que aún leen SQLite:
`search/command_palette/.../data_source.rs`, `pane_group/.../ssh_server_pane.rs`,
`sftp_ops.rs`/`workspace/view.rs` (`resolve_server_auth` → Keychain por alias),
e `integration_testing/ssh_manager/*`. Ajustar tests.

---

## Rediseño del sync: `~/.ssh/config` cifrado, estilo git

> Acordado en sesión 2026-06-21 tras revisar `zap_sync`. Decisiones del usuario:
> (1) merge de **texto completo línea a línea** (como git); (2) **cifrar** el
> contenido; (3) sincronizar **config + contraseñas + ficheros de clave**, pero
> (4) **reforzando antes el cifrado**; (5) **soportar varios backends** de
> almacenamiento.

### Motivación de seguridad (hallazgo clave)
El cifrado actual (`zap_sync/crypto.rs`) deriva la clave como
`SHA256(SHA256(token))`: **la clave de descifrado ES el token de GitHub**. Un
token filtrado = todo el contenido descifrable. Inaceptable para material de
clave privada. La seguridad del sync la determina el **cifrado en cliente**, no
el backend: con E2EE de conocimiento cero, el almacén solo ve un blob opaco.

### Fase 6 — Cripto E2EE v2 (fundacional, TDD)
En `zap_sync/crypto.rs`, formato **versionado** (header con versión + parámetros):
- **Passphrase independiente** elegida por el usuario (separa "acceso al almacén"
  de "poder descifrar"); nunca se persiste en claro (en memoria con `Zeroizing`,
  opcionalmente cacheada en el Keychain si el usuario lo elige). Verificador para
  validar la passphrase al introducirla.
- **KDF fuerte con sal aleatoria por blob**: Argon2id (recomendado; requiere
  añadir crate `argon2`) o, sin dep nueva, PBKDF2-HMAC-SHA256 ~600k iter
  (`pbkdf2` ya está en el árbol). Decisión menor pendiente.
- **AEAD**: XChaCha20-Poly1305 (nonce de 24B, `chacha20poly1305` ya disponible)
  o AES-256-GCM. Parámetros KDF (sal, iter) viajan en el header del blob.
- **Migración** v1→v2: leer blobs v1 (token) para una última bajada; reescribir
  en v2 al primer upload tras configurar passphrase.

### Fase 7 — Abstracción de backend de almacenamiento
Generalizar el actual `GistOps` a un trait `SyncBackend` neutro
(`load`/`store`/`exists` de un blob por sección + metadatos de versión).
Implementaciones: `GistBackend` (envuelve `GistClient`) y `FolderBackend`
(lee/escribe un fichero cifrado en una carpeta configurable → iCloud Drive /
Dropbox / Syncthing; sin API de terceros). `SyncEngine` genérico sobre
`SyncBackend`. UI: selector de backend + config por backend (token+plataforma /
ruta de carpeta).

### Fase 8 — Sync del config con merge a 3 vías
- **Base snapshot**: copia del config en la última sync correcta, en
  `<app_data>/ssh_sync/config.base` (NO en `~/.ssh`). Permite distinguir "lo
  cambié yo" de "lo cambió el otro equipo".
- **`merge3(base, local, remote)`**: merge de texto a 3 vías (crate `diffy`,
  recomendado; o `git2::merge_file` ya en el árbol). Limpio → escribe el
  resultado (atómico, `0600`, backup), actualiza base, sube (versión+1). Conflicto
  → marcadores `<<<<<<<`/`=======`/`>>>>>>>` para resolver en UI.
- **UI de diff/conflictos**: mostrar el diff de lo que entraría y, en conflicto,
  editor de resolución antes de escribir/subir.
- Primera sync sin blob remoto → sube el local y fija `base = local`.

### Fase 9 — Sync de secretos (bajo E2EE v2)
- **Contraseñas**: por alias, Keychain → cifrar (v2) → sección del blob; al bajar,
  descifrar → Keychain. Automático.
- **Ficheros de clave privada**: **opt-in por clave**, con aviso explícito. Leer
  bytes → cifrar (v2) → blob; al bajar, escribir con `0600` y confirmación. Nunca
  bajo cripto v1.

### Fase 10 — Verificar
`./script/run`: crear/editar/conectar, grupos, color de pestaña, file picker.
Sync: configurar passphrase + backend, upload/download entre dos perfiles, probar
merge limpio y conflicto, contraseñas, y (opt-in) una clave. Interoperabilidad:
abrir el config en SCE tras editar en Zap.

## Decisiones menores pendientes (no bloquean Fase 1)

1. **`Include`**: propuesta = editar solo el archivo principal y avisar si hay
   `Include` (no reescribir incluidos). Confirmar.
2. **Migración**: ¿exportar una vez al config los servidores ya guardados en la
   SQLite actual, o empezar limpio desde el config? (El usuario ya tiene su config
   completo en formato SCE → probablemente empezar limpio.)
3. **Mapa de iconos** SCE→Zap y **mapa de colores** de etiquetas: definir tablas.

## Notas

- Empezar por la **Fase 1** (autocontenida y testeable en aislamiento). Usar el
  `~/.ssh/config` real del usuario como fixture de round-trip (anonimizado).
- "Los botones de play/import no hacen nada" hoy → dependen del flujo
  SQLite/candidatos que se sustituye; no merece depurarlos, desaparecen.
- Bug ya corregido en esta rama: crash al abrir el editor por
  `CrossAxisAlignment::Stretch` bajo eje no acotado en `render_tab_bar`
  (`server_view.rs`); arreglado con borde inferior auto-dimensionado (commit
  e17e8bf6).
- Cómo ejecutar: `./script/run` (macOS empaqueta `.app`), `./script/run --dont-open`
  para recompilar sin abrir. Tests: `cargo nextest run --no-fail-fast --workspace
  --exclude command-signatures-v2`. Type-check rápido: `cargo check -p warp --lib`.

## Progreso

### Fase 1 — Modelo + writer round-trip ✅ (completada)

Implementada en `crates/warp_ssh_manager/`:
- **`config_model.rs`** (`SshConfigDocument`): parser/serializador **sin pérdidas** —
  round-trip byte a byte (comentarios, líneas en blanco, indentación con tabs,
  **casing** de directivas `localforward`/`LocalForward`, directivas no reconocidas
  `IdentitiesOnly`/`IdentityAgent`, e `Include`/`Match` intactos). Lee/escribe
  metadatos SCE (`#SCE_GROUP`, `#SCEGroup`, `#SCEIcon`, `#SCETags`, descripción).
  - API: `groups`, `create_group` (UUID v4 mayúsculas estilo SCE), `rename_group`,
    `delete_group` (huerfaniza hosts a la raíz); `host_view`, `outline`,
    `upsert_host` (quirúrgico — no toca SCE ni directivas desconocidas),
    `remove_host`, `rename_host`, `set_group`/`set_icon`/`set_color`/`set_description`.
  - Directivas: `HostName, User, Port, IdentityFile, ProxyJump` + forwards repetibles.
  - **Escritura atómica** `save_document_atomic`: backup `<path>.bak`, temp en el
    mismo dir, permisos `0600`, `rename` final. Sin dep de runtime nueva (usa `uuid`).
- **`config_tree.rs`** (`build_config_tree`): función **pura** que convierte el
  documento en el `Vec<SshNode>` (carpetas desde grupos, servidores desde hosts,
  con `parent_id`/`is_collapsed`) + mapa `alias → HostView`, agrupando por
  `#SCE_GROUP` y respetando orden de documento; uuids colgantes → raíz.
- Tests: 28 (`config_model`) + 7 (`config_tree`). `cargo test -p warp_ssh_manager`:
  **151 passed**. Clippy limpio, fmt aplicado.

### Fase 2 — Repointar el panel al config ✅ (núcleo de lectura cableado)

En `app/src/ssh_manager/panel.rs`:
- Campos nuevos en `SshManagerPanel`: `config_doc`, `config_path`, `host_meta`
  (`alias → HostView`), `collapsed: HashSet<String>` (colapso en memoria, el config
  no lo persiste).
- `refresh_tree` lee de `~/.ssh/config` (`load_document_from` + `build_config_tree`)
  en vez de `SshRepository::list_nodes`. Árbol agrupado por `#SCE_GROUP`.
- `on_toggle_node_collapsed` / `on_toggle_all_folders`: colapso en `collapsed`.
- `render`: sección de candidatos eliminada (fusionada en el árbol unificado).
- Verificado: `cargo check -p warp --lib` sin errores/avisos; 19 tests del panel OK.

**Estado intermedio (esperado):** el árbol **lee** del config; las acciones de
**escritura/conexión** (`on_add_server`, `on_edit`, `on_delete_selected`,
`commit_rename`, `on_move_node`, `on_connect`) siguen apuntando a SQLite y quedan
temporalmente desconectadas — se recablean en Fase 3.

**Pendiente de pulido en Fase 2 (no bloqueante):**
- Pintar **icono y color por fila** en `render_row` (el dato ya está en
  `self.host_meta[alias].icon/.color`). Falta el mapa SCE→iconos de Zap
  (`crates/warp_core/src/ui/icons.rs`) y el mapa nombre-de-etiqueta→color
  ("decisiones menores" punto 3).

### Fase 3 — Editor sobre el config (en curso)

**Alcance acordado: núcleo primero.** Icono/color (selectores + mapas SCE→Zap)
se posponen a una segunda tanda; el selector de **grupo** ya existe.

Hallazgos clave de la sesión:
- El `node_id` de un host en el árbol = **alias** (ver `config_tree::server_node`).
- `ForwardEntry.spec` usa el **formato de fichero** (`8080 localhost:80`, `1080`),
  no el CLI `-L`. Conversión `PortForward ↔ spec`:
  `PortForward::to_config_spec()` / `from_config_spec(kind, spec)` en `types.rs`.
- Conexión: ya hay dos rutas en `workspace/view.rs`:
  `open_ssh_terminal(node_id, SshServerInfo)` (con inyección de secreto; si falla
  `resolve_server_auth` en SQLite cae a usar el `node_id`=alias como lookup de
  Keychain — **funciona para hosts del config**) y `open_ssh_alias_terminal(alias)`
  (sin inyección). Para password-auth se usa la 1ª: construir `SshServerInfo`
  desde los campos del editor y emitir `OpenSshTerminal{node_id: alias, server}`.
- Password sigue en Keychain indexada por **alias** (= node_id).

Progreso (núcleo completado ✅):
- ✅ `PortForward::to_config_spec`/`from_config_spec` + 8 tests (types.rs).
- ✅ Editor `server_view.rs`: `reload` lee `~/.ssh/config` (`load_document_from` +
  `host_view(alias)`), construye un `SshServerInfo` sintético para los caminos de
  connect/test, grupos desde `doc.groups()`, forwards desde `host_view`. `on_save`
  → `upsert_host(CoreHostFields)` + `set_group` + `set_description` + `rename_host`
  si cambió el alias + `save_document_atomic` + password→Keychain(alias). Host
  nuevo: `node_id` vacío → modo nuevo (alias editable; puerto vacío = 22). Campo
  Host vacío en connect/test cae al alias.
- ✅ Panel `panel.rs`: helper `save_config_mut` (carga fresca + edit + guardado
  atómico). `on_add_server` abre editor en modo nuevo (node_id ""),
  `dispatch_connect_for`/`on_open_sftp` (SshServerInfo desde `host_meta[alias]` vía
  `server_info_from_host_view`), `on_delete_selected` (remove_host/delete_group),
  `commit_rename` (rename_host/rename_group), `on_move_node` (set_group, modelo
  plano: solo hosts), `on_add_folder_with_parent` (create_group).
- ✅ `cargo check -p warp --lib` limpio; clippy sin warnings nuevos; 159 tests del
  crate + 74 de `warp::ssh_manager` verdes.

Pendiente (segunda tanda / Fases 4-5):
- **Icono/color** (selectores en el editor + mapas SCE→icono y nombre→color). Fila
  del panel: pintar icono/color (dato ya en `host_meta`).
- `on_clone_server` y todo el bloque **OneKey** siguen en SQLite y quedan
  inservibles con el nuevo árbol — se eliminan en **Fase 5**.
- Verificación E2E (`./script/run`): crear/editar/conectar/renombrar/mover, abrir
  el config en SCE tras editar en Zap (interoperabilidad).

### Fase 4 — Color de pestaña al conectar ✅

En `app/src/workspace/view.rs`:
- `sce_tag_to_tab_color`: mapa `#SCETags`→`AnsiColorIdentifier` (Red→Red,
  Orange/Yellow→Yellow, Green→Green, Blue→Blue, Purple→Magenta, Gray→White;
  None/desconocido → sin color).
- `apply_config_tab_color(alias)`: lee el color del host en `~/.ssh/config` y, si
  existe, fija `selected_color` de la pestaña activa recién creada. Invocado en
  `open_ssh_terminal` y `open_ssh_alias_terminal`. Host sin etiqueta → color por
  defecto. (El SFTP abre un pane, no una pestaña de terminal, así que no aplica.)
- También localizado todo el módulo SFTP (en/ja/zh-CN) + prefijos de error vía
  `SftpOpsError::localized()`.

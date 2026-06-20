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

### Fase 5 — Eliminar SQLite y código muerto
`db.rs`, `repository.rs`, migraciones, `sync_config.rs`, `sync_provider.rs`,
onekey, folders en DB. Ajustar tests.

### Fase 6 — Verificar
`./script/run`: crear/editar/conectar, grupos, iconos, color de pestaña, file
picker. Comprobar interoperabilidad: abrir el config en SCE tras editar en Zap.

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

### Próximo paso
Fase 3 (editor sobre el config en `server_view`: General + grupo/icono/color,
port forwarding → `LocalForward/...`, Save → `upsert_host` + setters SCE +
password→Keychain, Nuevo → bloque `Host`, Play → `ssh <alias>`).

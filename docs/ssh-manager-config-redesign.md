# SSH Manager — Rediseño a `~/.ssh/config` como fuente única

> Plan acordado para retomar en sesión nueva. Rama: `fix/ssh-manager-review`.

## Objetivo

Eliminar el doble almacén actual (SQLite interno = verdad + `~/.ssh/config` solo
importable) y usar **`~/.ssh/config` como fuente única**: cargar, conectar,
editar y crear entradas, persistiendo en el propio archivo.

## Decisiones tomadas

1. **Fuente de datos: el config como única fuente.** Se retira SQLite y todo lo
   que el formato del config no soporta: carpetas/grupos, clonar, credenciales
   OneKey y el árbol jerárquico → **lista plana** de hosts. Los "candidatos"
   dejan de ser una sección aparte: pasan a ser *el* listado.
2. **Autenticación: clave + contraseña en llavero.** `HostName/User/Port/IdentityFile`
   se escriben en el config; las **contraseñas siguen en el Keychain de macOS**
   indexadas por host (ya existe `KeychainSecretStore` en
   `crates/warp_ssh_manager/src/secrets.rs`). Nunca secretos en texto en el config.

## Estado actual del código (investigado)

- **Leer config**: ✅ `crates/warp_ssh_manager/src/ssh_config_parser.rs`
  (`parse_ssh_config`, `load_candidates`, `default_ssh_config_path`).
- **Conectar desde config**: ✅ `crates/warp_ssh_manager/src/ssh_command.rs`
  (construye `ssh <alias>`).
- **File picker de clave privada**: ✅ **ya existe** en
  `app/src/ssh_manager/server_view.rs` (`open_key_file`, abre el explorador del
  sistema y escribe la ruta en el editor). Reusar, no recrear.
- **Escribir/editar/crear en config**: ❌ **no existe**. `sync_config.rs` es
  "Config → node only. We never write back to ~/.ssh/config." Toda la
  persistencia va hoy a SQLite (`db.rs`, `repository.rs`).
- UI: `app/src/ssh_manager/panel.rs` (~2943 líneas) y `server_view.rs` (~2890).

## Plan por fases

### Fase 1 — Writer round-trip de `~/.ssh/config` (fundacional, TDD)
Pieza que falta y de la que depende todo. En `warp_ssh_manager`:
- Ampliar el parser de solo-lectura a un **modelo editable que preserve**
  comentarios, orden, indentación, directivas no reconocidas y bloques
  `Host`/`Match`.
- API: `upsert_host(alias, fields)`, `remove_host`, `rename_host`.
- Escritura **atómica** (temp + rename), permisos `600`, backup previo.
- Directivas: `HostName, User, Port, IdentityFile, ProxyJump,
  LocalForward/RemoteForward/DynamicForward`.
- Tests: round-trip idempotente; upsert no destruye comentarios ni `Include`;
  crear / editar / eliminar host.

### Fase 2 — Repointar el panel al config
Sustituir `SshRepository` por lectura de hosts del config. Listado = lista plana.
Fusionar la sección de candidatos con el listado. Eliminar el árbol jerárquico.

### Fase 3 — Editor sobre el config (`server_view`)
- *General*: HostName, User, Port, **IdentityFile con el file picker existente**,
  contraseña (Keychain).
- *Port forwarding*: mapear a `LocalForward/RemoteForward/DynamicForward`.
- *Save* → `upsert_host` + password→Keychain. *Nuevo* → bloque `Host` nuevo.
  *Play* → `ssh <alias>` (ya existe).

### Fase 4 — Eliminar SQLite y código muerto
`db.rs`, `repository.rs`, migraciones, `sync_config.rs`, `sync_provider.rs`,
onekey, folders. Ajustar tests.

### Fase 5 — Verificar
`./script/run` (compila/empaqueta el `.app`): crear/editar/conectar + file picker.

## Decisiones menores pendientes (no bloquean Fase 1)

1. **`Include`**: propuesta = editar solo el archivo principal y avisar si hay
   `Include` (no reescribir archivos incluidos). Confirmar.
2. **Migración**: ¿exportar una vez al config los servidores ya guardados en la
   SQLite actual, o empezar limpio desde el config? Decidir.

## Notas

- Empezar por la **Fase 1** (autocontenida y testeable en aislamiento).
- "Los botones de play/import no hacen nada" hoy → dependen del flujo
  SQLite/candidatos que se sustituye; no merece depurarlos, desaparecen.
- Bug ya corregido en esta rama (contexto): crash al abrir el editor por
  `CrossAxisAlignment::Stretch` bajo eje no acotado en `render_tab_bar`
  (`server_view.rs`); arreglado con borde inferior auto-dimensionado.
- Cómo ejecutar: `./script/run` (macOS empaqueta `.app`), `./script/run --dont-open`
  para recompilar sin abrir. Tests: `cargo nextest run --no-fail-fast --workspace
  --exclude command-signatures-v2`. Type-check rápido: `cargo check -p warp --lib`.

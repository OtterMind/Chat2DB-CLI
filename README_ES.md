# Chat2DB CLI

[English](README.md) | [Francés](README_FR.md) | [Japonés](README_JA.md) | Español | [Coreano](README_KO.md)

Chat2DB CLI es el conjunto oficial de herramientas de línea de comandos e
integración con agentes para [Chat2DB](https://github.com/OtterMind/Chat2DB).
Este repositorio incluye una CLI en Rust para gestionar el ciclo de vida de la
aplicación, metadatos, SQL y MCP, además de Chat2DB Agent Skill. Chat2DB
Community es la edición predeterminada.

## Contenido

- `chat2db`: CLI nativa para macOS, Linux y Windows.
- `skill/`: Chat2DB Agent Skill para Codex y Claude Code.
- Salida `--json` legible por máquinas para scripts y agentes.
- Una única versión y publicación para la CLI y el Skill.

## Inicio rápido con Community

Instala la CLI en macOS o Linux:

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.sh | sh
```

En Windows PowerShell:

```powershell
iwr https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.ps1 -UseBasicParsing | iex
```

En macOS o Windows, instala y revisa Chat2DB Community:

```bash
chat2db install --edition community
chat2db status
```

`community` es la edición predeterminada, por lo que puedes omitir
`--edition community`. Community funciona de forma local y no requiere inicio
de sesión.
En Linux se admiten la CLI y el Agent Skill, pero los comandos de ciclo de vida
y Runtime requieren una versión de escritorio de Chat2DB compatible.

## Requisitos del sistema

- macOS 11 o posterior en arm64 o x86_64.
- Windows 10 u 11, solo de 64 bits. Windows en ARM64 ejecuta el binario x86_64
  mediante la capa de compatibilidad x64 del sistema. Windows de 32 bits no es
  compatible.
- GNU/Linux en arm64 o x86_64 con glibc 2.35 o posterior. Los artefactos de
  Linux publicados usan GNU libc; los sistemas que solo incluyen musl, como
  Alpine Linux, no son compatibles.

## Ediciones

| Capacidad | Community | Pro | Local |
| --- | :---: | :---: | :---: |
| Instalación y ciclo de vida en macOS/Windows | Sí | Sí | Sí |
| Runtime de CLI sin interfaz | Sí | Sí | Sí |
| CRUD de fuentes de datos y metadatos | Sí | Sí | Sí |
| Ejecución de SQL | Sí | Sí | Sí |
| Gestión de MCP y llamadas a herramientas | Sí | Sí | Sí |
| Inicio y cierre de sesión de cuenta | No aplica | Sí | Sí |
| Salida JSON | Sí | Sí | Sí |

Los comandos de Runtime, fuentes de datos, metadatos y SQL de Community
requieren Chat2DB Community 5.3.0 o una versión posterior compatible. La CLI
valida la edición del Runtime, la versión de la API y las capacidades antes de
enviar datos de negocio. Un servidor incompatible se rechaza sin cambiar a
Local o Pro.

Usa `--edition community`, `--edition pro` o `--edition local` para seleccionar
un producto. Los comandos compatibles con varios productos también aceptan
`--edition all`. Ejecuta `chat2db <command> --help` para consultar las opciones
exactas de cada comando.

## Flujos de la CLI

Gestiona la aplicación de escritorio Community:

```bash
chat2db status
chat2db start --edition community
chat2db stop --edition community
chat2db restart --edition community
```

Los ejemplos de Runtime, fuentes de datos y SQL que aparecen a continuación
requieren Chat2DB Community 5.3.0 o una versión posterior compatible. Un
Runtime anterior o incompatible se rechaza antes de enviar datos de negocio.

Revisa el Runtime local:

```bash
chat2db runtime status --edition community --json
chat2db runtime start --edition community --json
chat2db runtime stop --edition community --json
```

Lista fuentes de datos y metadatos:

```bash
chat2db db datasources --edition community --json
chat2db db databases --edition community --data-source-id 123 --json
chat2db db schemas --edition community --data-source-id 123 --database postgres --json
chat2db db tables --edition community --data-source-id 123 --database postgres --schema public --json
chat2db db table --edition community --data-source-id 123 --database postgres --schema public --table users --json
```

Prueba y crea una fuente de datos:

```bash
chat2db db connection-test --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --json
chat2db db datasource-create --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --environment-id 1 --json
```

Es preferible reutilizar una fuente guardada para evitar que la contraseña
quede en el historial del shell. No publiques contraseñas, tokens, hosts
privados ni datos de producción en registros o Issues.

Ejecuta SQL:

```bash
chat2db sql query --edition community --data-source-id 123 --database postgres --schema public --sql 'select 1' --json
```

Cuando un comando de base de datos lo necesita, la CLI inicia o reutiliza el
Runtime de la edición elegida, limitado a la interfaz de loopback.

## MCP

Activa MCP, revisa sus herramientas y muestra una configuración de cliente:

```bash
chat2db mcp enable --edition community --json
chat2db mcp tools --edition community --json
chat2db mcp config --edition community
```

Llama directamente a una herramienta:

```bash
chat2db mcp call list_all_datasources --edition community --args-json '{}' --json
```

`mcp config` incluye intencionadamente un token de autenticación. Trata su
salida como un secreto y no la pegues en registros públicos.

## Agent Skill

Instala el Skill incluido para Codex:

```bash
chat2db skill install --client codex
```

O para Claude Code:

```bash
chat2db skill install --client claude-code
```

Actualízalo o elimínalo con:

```bash
chat2db skill update --client codex
chat2db skill uninstall --client codex
```

El Skill se mantiene en [`skill/`](skill/) y se publica con la versión
correspondiente de la CLI. Utiliza la CLI en lugar de duplicar la lógica del
Runtime de Chat2DB.

## Automatización

Al iniciar, los comandos interactivos con salida legible realizan una
comprobación de actualizaciones de mejor esfuerzo. El manifiesto de red se
actualiza como máximo una vez cada 24 horas; los demás inicios reutilizan la
caché privada y siguen mostrando una versión más reciente ya conocida. La
comprobación solo muestra un aviso y nunca instala automáticamente. Se omite con
`--json`, en sesiones no interactivas y procesos en segundo plano, así como
en los comandos `update`, `completions` y `logs`. Define
`CHAT2DB_NO_UPDATE_CHECK=1` para desactivarla.

Usa `--json` cuando un script o agente consuma la salida:

```bash
chat2db status --json
chat2db version --json
```

Los comandos que modifican o destruyen datos pueden requerir `--yes`. Las
operaciones compatibles pueden inspeccionarse antes con `--dry-run`. Las
respuestas JSON no exponen intencionadamente contraseñas ni tokens Bearer del
Runtime.

## Instalación desde el código fuente

Instala la versión estable de Rust y ejecuta:

```bash
git clone https://github.com/OtterMind/Chat2DB-CLI.git
cd Chat2DB-CLI
cargo build --release
cargo test
```

El binario queda en `target/release/chat2db` o
`target/release/chat2db.exe` en Windows. El comando de desarrollo predeterminado
compila con el lockfile e instala transaccionalmente la CLI y el Skill de Codex.
La segunda variante usa Claude Code y la tercera actualiza solo el Skill de
Codex:

```bash
bash scripts/dev-install.sh
bash scripts/dev-install.sh --client claude-code
bash scripts/dev-install-skill.sh --client codex
```

Estos instaladores de desarrollo requieren Bash. En Windows, ejecútalos desde
Git Bash, MSYS2 o Cygwin; no se proporciona un instalador de desarrollo nativo
de PowerShell. El comando combinado confirma la CLI y el Skill juntos, restaura
ambos tras una instalación fallida o interrumpida y exige `--force` antes de
adoptar destinos sin procedencia coincidente del instalador.

## Desarrollo

Antes de abrir un Pull Request, ejecuta:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Si cambia el comportamiento o los comandos de la CLI, actualiza el Skill y los
cinco README en el mismo Pull Request. Consulta
[CONTRIBUTING.md](CONTRIBUTING.md).

## Estructura del repositorio

```text
src/          CLI en Rust
tests/        pruebas de integración
skill/        Chat2DB Agent Skill
scripts/      utilidades de instalación local y empaquetado
.github/      CI y automatización de publicaciones
```

## Publicaciones

El workflow de publicación se ejecuta manualmente y tiene dos modos. `cdn-only`
compila y sube objetos versionados sin modificar `latest`. `release` exige un
Tag firmado `vX.Y.Z` ya creado en el HEAD protegido de `main`; después promueve
los alias del CDN y publica juntos los binarios de la CLI y el paquete del Skill en
[GitHub Releases](https://github.com/OtterMind/Chat2DB-CLI/releases).

Cada GitHub Release incluye un SBOM CycloneDX y una atestación de procedencia
de compilación de GitHub. Cada archivo de la CLI y del Skill incluye `LICENSE`
y `THIRD_PARTY_LICENSES.txt`; ambos también se publican como recursos de primer
nivel de la Release. Las credenciales del CDN y el webhook de Feishu se
guardan únicamente en GitHub Actions Secrets y nunca en este repositorio. Los
mantenedores deben seguir [docs/RELEASING.md](docs/RELEASING.md).

Los instaladores de la CLI y del Skill leen sus manifiestos autoritativos desde
GitHub Releases; estos manifiestos alojados de forma independiente fijan el
checksum de cada artefacto del CDN. Los metadatos de instalación de escritorio
se revisan en `desktop-artifacts.json` y se compilan en la CLI. Las entradas
Community fijan artefactos de una GitHub Release formal con SHA-256; las de Pro
y Local fijan artefactos versionados exactos del CDN con su tamaño y SHA-512 en
Base64. La CLI nunca confía en un manifiesto mutable del CDN durante la
instalación. Actualiza la CLI para obtener un registro de artefactos de
escritorio más reciente y revisado.

## Seguridad

No informes vulnerabilidades en Issues públicos. Sigue
[SECURITY.md](SECURITY.md) y usa el sistema privado de GitHub. Chat2DB Community
es un producto local para un único usuario; no expongas su servicio local
directamente a redes no confiables.

## Contribuir

Aceptamos Issues y Pull Requests. Lee [CONTRIBUTING.md](CONTRIBUTING.md) y sigue
el [Código de conducta](CODE_OF_CONDUCT.md).

## Licencia

La CLI y el Agent Skill de este repositorio se distribuyen bajo la
[Licencia Apache 2.0](LICENSE). Esta licencia es independiente de la aplicada
al código fuente del servidor Chat2DB Community.

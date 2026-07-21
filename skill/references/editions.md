# Editions

Chat2DB CLI commands target Community by default. `mcp status` is the exception:
it defaults to `all` so one status call reports every edition.

## Values

| Value | Meaning |
| --- | --- |
| `community` | Chat2DB Community; default |
| `pro` | Chat2DB Pro |
| `local` | Chat2DB Local |
| `all` | Run Community, then Pro, then Local |

## Product Behavior

| Edition | Account login | Data directory | MCP cache |
| --- | --- | --- | --- |
| Community | Not applicable | `~/.chat2db-community` | `~/.chat2db-community/chat2db_cache_community` |
| Pro | Supported | `~/.chat2db` | Product-managed cache under the Pro data directory |
| Local | Supported | `~/.chat2db_local_edition` | Product-managed cache under the Local data directory |

Community is intentionally authentication-free. `login` and `logout` accept
`--edition community` so automation can use one edition value consistently,
but they return `status: not_applicable` and do not open or modify an
authentication flow.

Community `runtime`, `db`, and `sql` commands require a compatible Chat2DB
Community 5.3.0 or later build. The CLI validates the runtime edition, API
version, and capabilities before sending business data. A contract mismatch
fails closed and must not fall back to another edition's storage.

## Desktop Paths

| Edition | macOS | Windows |
| --- | --- | --- |
| Community | `/Applications/Chat2DB Community.app` | Community MSI installation discovered from Windows installation metadata |
| Pro | `/Applications/Chat2DB Pro.app` | Pro installation under the user's application directories |
| Local | `/Applications/Chat2DB Local.app` | Local installation under the user's application directories |

Desktop lifecycle and runtime commands are supported on macOS and Windows.
Linux is not a desktop-app target.

## Command Matrix

| Command | Accepted editions |
| --- | --- |
| `install`, `uninstall`, `start`, `stop`, `restart` | `community`, `pro`, `local`, `all` |
| `login` | `community`, `pro`, `local`, `all`; avoid `all` |
| `logout` | `community`, `pro`, `local`, `all` |
| `runtime`, `db`, `sql` | `community`, `pro`, `local` |
| `mcp status` | `community`, `pro`, `local`, `all` |
| `mcp enable/disable/restart/config/tools/call` | `community`, `pro`, `local` |
| `logs install` | `community`, `pro`, `local` |
| `status` | No edition flag; reports all three |
| `update`, `skill`, `completions`, `version` | No app edition |

## `all`

Commands that accept `all` run serially in Community, Pro, Local order. A
failure stops later editions. Community `not_applicable` login/logout behavior
is not a failure.

Do not use `login --edition all`. Pro and Local login require user interaction
and can block independently.

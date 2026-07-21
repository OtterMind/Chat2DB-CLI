# Chat2DB CLI

[English](README.md) | [Français](README_FR.md) | [日本語](README_JA.md) | [Español](README_ES.md) | 한국어

Chat2DB CLI는 [Chat2DB](https://github.com/OtterMind/Chat2DB)의 공식 명령줄 및
Agent 통합 도구입니다. 이 저장소에는 데스크톱 앱 수명 주기, 데이터소스 메타데이터,
SQL, MCP 작업을 위한 Rust CLI와 Chat2DB Agent Skill이 함께 포함됩니다. 기본
에디션은 Chat2DB Community입니다.

## 포함 항목

- `chat2db`: macOS, Linux, Windows용 네이티브 CLI.
- `skill/`: Codex와 Claude Code용 Chat2DB Agent Skill.
- 스크립트와 Agent를 위한 기계 판독 가능 `--json` 출력.
- CLI와 Skill에 적용되는 하나의 버전 및 릴리스.

## Community 빠른 시작

macOS 또는 Linux에 CLI를 설치합니다.

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.sh | sh
```

Windows PowerShell에서는 다음을 실행합니다.

```powershell
iwr https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.ps1 -UseBasicParsing | iex
```

macOS 또는 Windows에서 Chat2DB Community를 설치하고 상태를 확인합니다.

```bash
chat2db install --edition community
chat2db status
```

`community`가 기본 에디션이므로 명시적인 `--edition community`는 생략할 수
있습니다. Community는 로컬 우선 제품이며 로그인이 필요하지 않습니다.
Linux에서는 CLI 자체와 Agent Skill을 지원하지만 데스크톱 수명 주기 및 Runtime
명령에는 지원되는 Chat2DB 데스크톱 릴리스가 필요합니다.

## 시스템 요구 사항

- arm64 또는 x86_64 기반 macOS 11 이상.
- 64-bit Windows 10 또는 11. Windows ARM64에서는 운영 체제의 x64 호환 계층을
  통해 x86_64 바이너리를 실행합니다. 32-bit Windows는 지원하지 않습니다.
- glibc 2.35 이상을 사용하는 arm64 또는 x86_64 GNU/Linux. 배포되는 Linux
  아티팩트는 GNU libc용이며 Alpine Linux를 포함한 musl 전용 시스템은 지원하지
  않습니다.

## 에디션

| 기능 | Community | Pro | Local |
| --- | :---: | :---: | :---: |
| macOS/Windows 설치 및 데스크톱 수명 주기 | 지원 | 지원 | 지원 |
| 헤드리스 CLI Runtime | 지원 | 지원 | 지원 |
| 데이터소스 CRUD 및 메타데이터 | 지원 | 지원 | 지원 |
| SQL 실행 | 지원 | 지원 | 지원 |
| MCP 관리 및 도구 호출 | 지원 | 지원 | 지원 |
| 계정 로그인 및 로그아웃 | 해당 없음 | 지원 | 지원 |
| JSON 출력 | 지원 | 지원 | 지원 |

Community Runtime, 데이터소스, 메타데이터 및 SQL 명령에는 호환되는 Chat2DB
Community 5.3.0 이상이 필요합니다. CLI는 비즈니스 데이터를 보내기 전에 Runtime
에디션, API 버전 및 기능을 검증하며 호환되지 않는 서버를 거부합니다. Local 또는
Pro로 폴백하지 않습니다.

`--edition community`, `--edition pro`, `--edition local`로 대상 제품을
선택합니다. 여러 제품을 지원하는 명령은 `--edition all`도 허용합니다. 각 명령의
정확한 옵션은 `chat2db <command> --help`로 확인하십시오.

## CLI 워크플로

Community 데스크톱 앱을 관리합니다.

```bash
chat2db status
chat2db start --edition community
chat2db stop --edition community
chat2db restart --edition community
```

아래 Runtime, 데이터소스 및 SQL 예시에는 호환되는 Chat2DB Community 5.3.0
이상이 필요합니다. 오래되었거나 일치하지 않는 Runtime은 비즈니스 데이터를 보내기
전에 거부됩니다.

로컬 Runtime을 확인합니다.

```bash
chat2db runtime status --edition community --json
chat2db runtime start --edition community --json
chat2db runtime stop --edition community --json
```

데이터소스와 메타데이터를 조회합니다.

```bash
chat2db db datasources --edition community --json
chat2db db databases --edition community --data-source-id 123 --json
chat2db db schemas --edition community --data-source-id 123 --database postgres --json
chat2db db tables --edition community --data-source-id 123 --database postgres --schema public --json
chat2db db table --edition community --data-source-id 123 --database postgres --schema public --table users --json
```

데이터소스를 테스트하고 생성합니다.

```bash
chat2db db connection-test --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --json
chat2db db datasource-create --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --environment-id 1 --json
```

비밀번호가 Shell 기록에 남지 않도록 저장된 데이터소스를 재사용하는 것이 좋습니다.
로그나 Issue에 비밀번호, 토큰, 비공개 호스트 또는 운영 데이터를 게시하지 마십시오.

SQL을 실행합니다.

```bash
chat2db sql query --edition community --data-source-id 123 --database postgres --schema public --sql 'select 1' --json
```

데이터베이스 명령에 필요할 때 CLI는 선택한 에디션의 루프백 전용 Runtime을 시작하거나
재사용합니다.

## MCP

MCP를 활성화하고 도구를 확인한 다음 클라이언트 설정을 출력합니다.

```bash
chat2db mcp enable --edition community --json
chat2db mcp tools --edition community --json
chat2db mcp config --edition community
```

도구를 직접 호출합니다.

```bash
chat2db mcp call list_all_datasources --edition community --args-json '{}' --json
```

`mcp config`에는 인증 토큰이 의도적으로 포함됩니다. 출력을 비밀 정보로 취급하고 공개
로그에 붙여 넣지 마십시오.

## Agent Skill

포함된 Skill을 Codex에 설치합니다.

```bash
chat2db skill install --client codex
```

Claude Code에 설치하려면 다음을 실행합니다.

```bash
chat2db skill install --client claude-code
```

업데이트하거나 제거합니다.

```bash
chat2db skill update --client codex
chat2db skill uninstall --client codex
```

Skill은 [`skill/`](skill/)에서 관리하며 해당 CLI 버전과 함께 릴리스합니다. Chat2DB
Runtime 로직을 중복 구현하지 않고 CLI를 사용합니다.

## 자동화

사람이 읽을 수 있는 출력을 사용하는 일반 대화형 명령은 시작할 때 최선형 업데이트 확인을
수행합니다. 네트워크 Manifest는 최대 24시간에 한 번 새로 고치며, 그 밖의 시작에서는 비공개
캐시를 다시 사용해 이미 확인한 새 버전을 계속 알립니다. 알림만 표시하며 자동으로 설치하지
않습니다. `--json`, 비대화형 세션, 백그라운드 프로세스와 `update`,
`completions`, `logs` 명령에서는 확인을 건너뜁니다. 비활성화하려면
`CHAT2DB_NO_UPDATE_CHECK=1`을 설정하십시오.

스크립트나 Agent가 출력을 사용할 때는 `--json`을 사용하십시오.

```bash
chat2db status --json
chat2db version --json
```

변경 또는 삭제 작업에는 `--yes`가 필요할 수 있습니다. 지원되는 작업은
`--dry-run`으로 먼저 확인할 수 있습니다. JSON 응답은 데이터소스 비밀번호나 Runtime
Bearer Token을 의도적으로 노출하지 않습니다.

## 소스에서 설치

안정 버전 Rust 도구 체인을 설치한 후 실행합니다.

```bash
git clone https://github.com/OtterMind/Chat2DB-CLI.git
cd Chat2DB-CLI
cargo build --release
cargo test
```

바이너리는 `target/release/chat2db`에 생성되며 Windows에서는
`target/release/chat2db.exe`입니다. 기본 개발 명령은 Lockfile로 빌드하고 CLI와 Codex
Skill을 트랜잭션 방식으로 함께 설치합니다. 두 번째 명령은 Claude Code용이며 세 번째
명령은 Codex Skill만 갱신합니다.

```bash
bash scripts/dev-install.sh
bash scripts/dev-install.sh --client claude-code
bash scripts/dev-install-skill.sh --client codex
```

## 개발

Pull Request를 열기 전에 실행합니다.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

CLI 동작이나 명령을 변경하면 같은 Pull Request에서 Skill과 다섯 개 README를 함께
업데이트하십시오. 자세한 내용은 [CONTRIBUTING.md](CONTRIBUTING.md)를 참조하십시오.

## 저장소 구조

```text
src/          Rust CLI
tests/        통합 테스트
skill/        Chat2DB Agent Skill
scripts/      로컬 설치 및 패키징 도우미
.github/      CI 및 릴리스 자동화
```

## 릴리스

수동으로 실행하는 릴리스 Workflow에는 두 가지 모드가 있습니다. `cdn-only`는 버전별
객체를 빌드하고 업로드하지만 `latest`는 변경하지 않습니다. `release`는 보호된 `main`
HEAD에 서명된 `vX.Y.Z` Tag가 이미 있어야 하며, CDN Alias를 승격하고 CLI 바이너리와
Skill Bundle을 함께
[GitHub Releases](https://github.com/OtterMind/Chat2DB-CLI/releases)에 게시합니다.

각 GitHub Release에는 CycloneDX SBOM과 GitHub 빌드 출처 증명이 포함됩니다. CLI와
Skill 아카이브에는 각각 `LICENSE` 및 `THIRD_PARTY_LICENSES.txt`가 포함되며 두 파일은
Release 최상위 아티팩트로도 게시됩니다. CDN
자격 증명과 Feishu 알림 Webhook은 GitHub Actions Secrets에만 보관하며 저장소에는
저장하지 않습니다. 관리자는 [docs/RELEASING.md](docs/RELEASING.md)를 따라야 합니다.

CLI와 Skill 설치 프로그램은 GitHub Releases의 기준 Manifest를 읽으며, 별도로 호스팅된
Manifest가 각 CDN 아티팩트의 체크섬을 고정합니다. 데스크톱 설치 메타데이터는
`desktop-artifacts.json`에서 검토한 뒤 CLI에 포함됩니다. Community 항목은 정식
GitHub Release 아티팩트를 SHA-256으로 고정하고, Pro와 Local 항목은 정확한 버전의 CDN
아티팩트를 크기 및 Base64 SHA-512와 함께 고정합니다. 설치할 때 변경 가능한 CDN
Manifest를 신뢰하지 않습니다. 더 최근에 검토된 데스크톱 아티팩트 목록이 필요하면
CLI를 업데이트하십시오.

## 보안

취약점을 공개 Issue에 보고하지 마십시오. [SECURITY.md](SECURITY.md)를 따르고 GitHub의
비공개 취약점 보고 기능을 사용하십시오. Chat2DB Community는 단일 사용자용 로컬 우선
제품입니다. 로컬 서비스를 신뢰할 수 없는 네트워크에 직접 노출하지 마십시오.

## 기여

Issue와 Pull Request를 환영합니다. [CONTRIBUTING.md](CONTRIBUTING.md)를 읽고
[행동 강령](CODE_OF_CONDUCT.md)을 준수하십시오.

## 라이선스

이 저장소의 CLI와 Agent Skill은 [Apache License 2.0](LICENSE)에 따라 제공됩니다.
이 저장소의 라이선스는 Chat2DB Community 서버 소스에 적용되는 라이선스와 독립적입니다.

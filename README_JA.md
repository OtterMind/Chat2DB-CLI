# Chat2DB CLI

[English](README.md) | [Français](README_FR.md) | 日本語 | [Español](README_ES.md) | [한국어](README_KO.md)

Chat2DB CLI は [Chat2DB](https://github.com/OtterMind/Chat2DB) 公式のコマンドラインおよび
Agent 連携ツールです。このリポジトリには、デスクトップアプリのライフサイクル、
データソースのメタデータ、SQL、MCP を操作する Rust CLI と Chat2DB Agent Skill が
含まれています。デフォルトのエディションは Chat2DB Community です。

## 含まれるもの

- `chat2db`: macOS、Linux、Windows 向けネイティブ CLI。
- `skill/`: Codex と Claude Code 向け Chat2DB Agent Skill。
- スクリプトや Agent 向けの機械可読な `--json` 出力。
- CLI と Skill で共通のバージョンおよびリリース。

## Community クイックスタート

macOS または Linux に CLI をインストールします。

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.sh | sh
```

Windows PowerShell の場合:

```powershell
iwr https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.ps1 -UseBasicParsing | iex
```

macOS または Windows で Chat2DB Community をインストールして状態を確認します。

```bash
chat2db install --edition community
chat2db status
```

`community` はデフォルトのエディションなので、明示的な
`--edition community` は省略できます。Community はローカルファーストで、
ログインは不要です。
Linux では CLI 本体と Agent Skill を利用できますが、デスクトップのライフサイクルと
Runtime コマンドには対応する Chat2DB デスクトップリリースが必要です。

## システム要件

- arm64 または x86_64 の macOS 11 以降。
- 64-bit の Windows 10 または 11。Windows ARM64 では、OS の x64 互換レイヤーを
  介して x86_64 バイナリを実行します。32-bit Windows はサポートされません。
- glibc 2.35 以降を備えた arm64 または x86_64 の GNU/Linux。公開される Linux
  アーティファクトは GNU libc 用です。Alpine Linux などの musl 専用環境は
  サポートされません。

## エディション

| 機能 | Community | Pro | Local |
| --- | :---: | :---: | :---: |
| macOS/Windows でのインストールとデスクトップ管理 | 対応 | 対応 | 対応 |
| ヘッドレス CLI Runtime | 対応 | 対応 | 対応 |
| データソース CRUD とメタデータ | 対応 | 対応 | 対応 |
| SQL 実行 | 対応 | 対応 | 対応 |
| MCP 管理とツール呼び出し | 対応 | 対応 | 対応 |
| アカウントのログインとログアウト | 対象外 | 対応 | 対応 |
| JSON 出力 | 対応 | 対応 | 対応 |

Community の Runtime、データソース、メタデータ、SQL コマンドには、互換性のある
Chat2DB Community 5.3.0 以降が必要です。CLI は業務データを送信する前に Runtime
のエディション、API バージョン、機能を検証し、互換性のないサーバーを拒否します。
Local または Pro へのフォールバックは行いません。

対象製品は `--edition community`、`--edition pro`、`--edition local` で指定します。
複数製品に対応するコマンドでは `--edition all` も利用できます。正確なオプションは
`chat2db <command> --help` で確認してください。

## CLI ワークフロー

Community デスクトップアプリを管理します。

```bash
chat2db status
chat2db start --edition community
chat2db stop --edition community
chat2db restart --edition community
```

以下の Runtime、データソース、SQL の例には、互換性のある Chat2DB Community
5.3.0 以降が必要です。古い Runtime や一致しない Runtime は、業務データの送信前に
拒否されます。

ローカル Runtime を確認します。

```bash
chat2db runtime status --edition community --json
chat2db runtime start --edition community --json
chat2db runtime stop --edition community --json
```

データソースとメタデータを一覧表示します。

```bash
chat2db db datasources --edition community --json
chat2db db databases --edition community --data-source-id 123 --json
chat2db db schemas --edition community --data-source-id 123 --database postgres --json
chat2db db tables --edition community --data-source-id 123 --database postgres --schema public --json
chat2db db table --edition community --data-source-id 123 --database postgres --schema public --table users --json
```

データソースをテストして作成します。

```bash
chat2db db connection-test --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --json
chat2db db datasource-create --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --environment-id 1 --json
```

パスワードを Shell 履歴に残さないため、保存済みデータソースの再利用を推奨します。
パスワード、トークン、非公開ホスト、実データをログや Issue に投稿しないでください。

SQL を実行します。

```bash
chat2db sql query --edition community --data-source-id 123 --database postgres --schema public --sql 'select 1' --json
```

データベースコマンドで必要になると、CLI は選択したエディションのループバック限定
Runtime を起動または再利用します。

## MCP

MCP を有効化し、ツールを確認してクライアント設定を出力します。

```bash
chat2db mcp enable --edition community --json
chat2db mcp tools --edition community --json
chat2db mcp config --edition community
```

ツールを直接呼び出します。

```bash
chat2db mcp call list_all_datasources --edition community --args-json '{}' --json
```

`mcp config` には認証トークンが意図的に含まれます。出力はシークレットとして扱い、
公開ログに貼り付けないでください。

## Agent Skill

同梱 Skill を Codex にインストールします。

```bash
chat2db skill install --client codex
```

Claude Code の場合:

```bash
chat2db skill install --client claude-code
```

更新または削除します。

```bash
chat2db skill update --client codex
chat2db skill uninstall --client codex
```

Skill は [`skill/`](skill/) で管理され、対応する CLI バージョンと一緒にリリースされます。
Chat2DB Runtime のロジックを重複実装せず、CLI を利用します。

## 自動化

人が読む形式で出力する通常の対話コマンドは、起動時にベストエフォートで更新を確認します。
ネットワーク Manifest の更新は 24 時間に最大 1 回で、それ以外の起動では非公開キャッシュを
再利用し、既知の新しいバージョンを引き続き通知します。通知を表示するだけで、自動インストール
は行いません。`--json`、非対話セッション、バックグラウンドプロセス、および `update`、
`completions`、`logs` コマンドでは確認を行いません。無効にするには
`CHAT2DB_NO_UPDATE_CHECK=1` を設定します。

スクリプトや Agent が出力を利用する場合は `--json` を使用します。

```bash
chat2db status --json
chat2db version --json
```

変更や破壊を伴うコマンドでは `--yes` が必要な場合があります。対応する操作は
`--dry-run` で事前確認できます。JSON 応答がデータソースのパスワードや Runtime の
Bearer Token を意図的に公開することはありません。

## ソースからのインストール

安定版 Rust ツールチェーンをインストールしてから実行します。

```bash
git clone https://github.com/OtterMind/Chat2DB-CLI.git
cd Chat2DB-CLI
cargo build --release
cargo test
```

バイナリは `target/release/chat2db`、Windows では
`target/release/chat2db.exe` に生成されます。標準の開発コマンドは Lockfile を使って
ビルドし、CLI と Codex Skill を Transaction としてインストールします。2 番目は
Claude Code 用、3 番目は Codex Skill だけを更新する場合に使います。

```bash
bash scripts/dev-install.sh
bash scripts/dev-install.sh --client claude-code
bash scripts/dev-install-skill.sh --client codex
```

## 開発

Pull Request を作成する前に実行します。

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

CLI の動作やコマンドを変更した場合は、同じ Pull Request で Skill と 5 言語の README
も更新してください。詳細は [CONTRIBUTING.md](CONTRIBUTING.md) を参照してください。

## リポジトリ構成

```text
src/          Rust CLI
tests/        統合テスト
skill/        Chat2DB Agent Skill
scripts/      ローカルインストールとパッケージ用ヘルパー
.github/      CI とリリース自動化
```

## リリース

手動で起動するリリース Workflow には 2 つのモードがあります。`cdn-only` は
バージョン付きオブジェクトをビルドしてアップロードし、`latest` は変更しません。
`release` は保護された `main` の HEAD に署名済み `vX.Y.Z` Tag が存在することを要求し、
CDN Alias を昇格して CLI バイナリと Skill Bundle を一緒に
[GitHub Releases](https://github.com/OtterMind/Chat2DB-CLI/releases) へ公開します。

各 GitHub Release には CycloneDX SBOM と GitHub のビルド来歴証明も含まれます。
CLI と Skill の各アーカイブには `LICENSE` と `THIRD_PARTY_LICENSES.txt` が含まれ、
両ファイルは Release のトップレベルアセットとしても公開されます。
CDN の認証情報と Feishu 通知 Webhook は GitHub Actions Secrets のみに保存し、
リポジトリには保存しません。メンテナーは
[docs/RELEASING.md](docs/RELEASING.md) に従ってください。

CLI と Skill のインストーラーは GitHub Releases の正規 Manifest を読み取り、
独立してホストされた Manifest が各 CDN 成果物のチェックサムを固定します。
デスクトップインストールのメタデータは `desktop-artifacts.json` でレビューされ、
CLI にコンパイルされます。Community は正式な GitHub Release 成果物を SHA-256 で固定し、
Pro と Local は正確なバージョン付き CDN 成果物をサイズと Base64 SHA-512 で固定します。
インストール時に可変 CDN Manifest を信頼しません。新しいレビュー済みレジストリを
取得するには CLI を更新してください。

## セキュリティ

脆弱性を公開 Issue で報告しないでください。[SECURITY.md](SECURITY.md) に従い、
GitHub の非公開脆弱性報告を利用してください。Chat2DB Community は単一ユーザー向けの
ローカルファースト製品です。ローカルサービスを信頼できないネットワークへ直接公開しないでください。

## コントリビューション

Issue と Pull Request を歓迎します。[CONTRIBUTING.md](CONTRIBUTING.md) を読み、
[行動規範](CODE_OF_CONDUCT.md) に従ってください。

## ライセンス

このリポジトリの CLI と Agent Skill は [Apache License 2.0](LICENSE) で提供されます。
このライセンスは Chat2DB Community サーバーソースのライセンスとは独立しています。

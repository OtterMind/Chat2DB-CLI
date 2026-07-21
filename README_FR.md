# Chat2DB CLI

[English](README.md) | Français | [Japanese](README_JA.md) | [Español](README_ES.md) | [Korean](README_KO.md)

Chat2DB CLI est la boîte à outils officielle en ligne de commande et
d'intégration avec les agents pour [Chat2DB](https://github.com/OtterMind/Chat2DB).
Elle fournit une CLI Rust pour gérer le cycle de vie de l'application de bureau,
les métadonnées des sources de données, SQL et les workflows MCP, ainsi que le
Chat2DB Agent Skill dans le même dépôt. Chat2DB Community est l'édition par
défaut.

## Contenu

- `chat2db` : une CLI native pour macOS, Linux et Windows.
- `skill/` : le Chat2DB Agent Skill pour Codex et Claude Code.
- Une sortie `--json` lisible par une machine pour les scripts et les agents.
- Une version et une release uniques pour la CLI et le Skill.

## Démarrage rapide avec Community

Installez la CLI sur macOS ou Linux :

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.sh | sh
```

Sur Windows PowerShell :

```powershell
iwr https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.ps1 -UseBasicParsing | iex
```

Sur macOS ou Windows, installez et inspectez Chat2DB Community :

```bash
chat2db install --edition community
chat2db status
```

`community` est l'édition par défaut, l'option explicite
`--edition community` peut donc être omise. Community privilégie une utilisation
locale et ne nécessite pas de connexion à un compte. Sous Linux, la CLI et
l'Agent Skill sont pris en charge, tandis que les commandes de cycle de vie de
l'application de bureau et de runtime nécessitent une release de bureau Chat2DB
compatible.

## Configuration requise

- macOS 11 ou version ultérieure sur arm64 ou x86_64.
- Windows 10 ou 11, uniquement en 64 bits. Windows sur ARM64 exécute le
  binaire x86_64 via la couche de compatibilité x64 du système. Windows 32 bits
  n'est pas pris en charge.
- GNU/Linux sur arm64 ou x86_64 avec glibc 2.35 ou version ultérieure. Les
  artefacts Linux publiés utilisent GNU libc ; les systèmes uniquement musl,
  notamment Alpine Linux, ne sont pas pris en charge.

## Éditions

| Fonctionnalité | Community | Pro | Local |
| --- | :---: | :---: | :---: |
| Installation et cycle de vie de l'application de bureau sur macOS/Windows | Oui | Oui | Oui |
| Runtime CLI sans interface | Oui | Oui | Oui |
| CRUD et métadonnées des sources de données | Oui | Oui | Oui |
| Exécution SQL | Oui | Oui | Oui |
| Gestion de MCP et appels d'outils | Oui | Oui | Oui |
| Connexion et déconnexion au compte | Non applicable | Oui | Oui |
| Sortie JSON | Oui | Oui | Oui |

Les commandes Community de runtime, de sources de données, de métadonnées et
de SQL nécessitent Chat2DB Community 5.3.0 ou une version ultérieure compatible.
La CLI valide l'édition du runtime, la version de l'API et les fonctionnalités
avant d'envoyer des données métier. Un serveur incompatible est refusé sans
basculement vers Local ou Pro.

Utilisez `--edition community`, `--edition pro` ou `--edition local` pour cibler
un produit. Les commandes qui prennent en charge plusieurs produits acceptent
également `--edition all`. Exécutez `chat2db <command> --help` pour connaître les
options exactes disponibles pour une commande.

## Workflows de la CLI

Gérez l'application de bureau Community :

```bash
chat2db status
chat2db start --edition community
chat2db stop --edition community
chat2db restart --edition community
```

Les exemples de runtime, de source de données et de SQL ci-dessous nécessitent
Chat2DB Community 5.3.0 ou une version ultérieure compatible. Un runtime plus
ancien ou incompatible est refusé avant l'envoi de données métier.

Inspectez le runtime local :

```bash
chat2db runtime status --edition community --json
chat2db runtime start --edition community --json
chat2db runtime stop --edition community --json
```

Listez les sources de données et les métadonnées :

```bash
chat2db db datasources --edition community --json
chat2db db databases --edition community --data-source-id 123 --json
chat2db db schemas --edition community --data-source-id 123 --database postgres --json
chat2db db tables --edition community --data-source-id 123 --database postgres --schema public --json
chat2db db table --edition community --data-source-id 123 --database postgres --schema public --table users --json
```

Testez et créez une source de données :

```bash
chat2db db connection-test --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --json
chat2db db datasource-create --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --environment-id 1 --json
```

Réutilisez de préférence une source de données enregistrée afin qu'aucun mot de
passe ne soit inscrit dans l'historique du shell. Ne publiez jamais de mots de
passe, de jetons, d'hôtes privés ni de données de production dans les journaux
ou les Issues.

Exécutez du SQL :

```bash
chat2db sql query --edition community --data-source-id 123 --database postgres --schema public --sql 'select 1' --json
```

La CLI démarre ou réutilise le runtime limité à l'interface de bouclage de
l'édition sélectionnée lorsqu'une commande de base de données en a besoin.

## MCP

Activez MCP, inspectez ses outils et affichez une configuration client :

```bash
chat2db mcp enable --edition community --json
chat2db mcp tools --edition community --json
chat2db mcp config --edition community
```

Appelez directement un outil :

```bash
chat2db mcp call list_all_datasources --edition community --args-json '{}' --json
```

`mcp config` contient volontairement un jeton d'authentification. Traitez sa
sortie comme un secret et ne la collez pas dans des journaux publics.

## Agent Skill

Installez le Skill intégré pour Codex :

```bash
chat2db skill install --client codex
```

Ou pour Claude Code :

```bash
chat2db skill install --client claude-code
```

Mettez-le à jour ou supprimez-le avec :

```bash
chat2db skill update --client codex
chat2db skill uninstall --client codex
```

Le Skill est maintenu dans [`skill/`](skill/) et publié avec la version
correspondante de la CLI. Il utilise la CLI au lieu de dupliquer la logique du
runtime Chat2DB.

## Automatisation

Au démarrage, les commandes interactives avec une sortie lisible effectuent une
vérification de mise à jour sans garantie. Le manifest réseau est actualisé au
plus une fois toutes les 24 heures ; les autres démarrages réutilisent le cache
privé et continuent de signaler une version plus récente connue. La vérification
affiche uniquement une notification et n'installe jamais automatiquement. Elle
est ignorée avec `--json`, dans les sessions non interactives, les processus en
arrière-plan et les commandes `update`, `completions` et `logs`.
Définissez `CHAT2DB_NO_UPDATE_CHECK=1` pour la désactiver.

Utilisez `--json` lorsque la sortie est consommée par un script ou un agent :

```bash
chat2db status --json
chat2db version --json
```

Les commandes qui modifient ou détruisent des données peuvent nécessiter
`--yes`. Les opérations prises en charge peuvent être inspectées avec
`--dry-run`. Les réponses JSON n'exposent jamais intentionnellement les mots de
passe des sources de données ni les jetons bearer du runtime.

## Installation depuis les sources

Installez la chaîne d'outils Rust stable, puis exécutez :

```bash
git clone https://github.com/OtterMind/Chat2DB-CLI.git
cd Chat2DB-CLI
cargo build --release
cargo test
```

Le binaire est écrit dans `target/release/chat2db` (ou
`target/release/chat2db.exe` sous Windows). Pour une installation de
développement, la commande par défaut compile avec le lockfile et installe de
manière transactionnelle la CLI et le Skill Codex. La deuxième forme cible
Claude Code et la troisième actualise uniquement le Skill Codex :

```bash
bash scripts/dev-install.sh
bash scripts/dev-install.sh --client claude-code
bash scripts/dev-install-skill.sh --client codex
```

Ces installateurs de développement nécessitent Bash. Sous Windows, exécutez-les
depuis Git Bash, MSYS2 ou Cygwin ; aucun installateur de développement PowerShell
natif n'est fourni. La commande combinée valide la CLI et le Skill ensemble,
restaure les deux après un échec ou une interruption, et exige `--force` avant
de prendre en charge une cible sans provenance correspondante.

## Développement

Avant d'ouvrir une pull request, exécutez :

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Lorsque le comportement ou les commandes de la CLI changent, mettez à jour le
Skill et les cinq fichiers README dans la même pull request. Consultez
[CONTRIBUTING.md](CONTRIBUTING.md).

## Organisation du dépôt

```text
src/          CLI Rust
tests/        tests d'intégration
skill/        Chat2DB Agent Skill
scripts/      assistants d'installation locale et de packaging
.github/      automatisation de la CI et des releases
```

## Releases

Le workflow de release, lancé manuellement, propose deux modes. `cdn-only`
construit et téléverse des objets versionnés sans modifier `latest`. `release`
exige un Tag signé `vX.Y.Z` déjà présent sur la HEAD protégée de `main`, puis
promeut les alias CDN et publie ensemble les binaires de la CLI et le bundle du
Skill dans
[GitHub Releases](https://github.com/OtterMind/Chat2DB-CLI/releases).

Chaque GitHub Release comprend un SBOM CycloneDX et une attestation GitHub de
provenance du build. Chaque archive de la CLI et du Skill contient `LICENSE` et
`THIRD_PARTY_LICENSES.txt` ; les deux fichiers sont également des ressources de
premier niveau de la Release. Les identifiants du CDN et le webhook de notification
Feishu sont conservés uniquement dans les secrets GitHub Actions ; ils ne sont
jamais stockés dans ce dépôt. Les mainteneurs doivent suivre
[docs/RELEASING.md](docs/RELEASING.md).

Les programmes d'installation de la CLI et du Skill lisent leurs manifests
faisant autorité depuis GitHub Releases ; ces manifests hébergés indépendamment
épinglent le checksum de chaque artefact du CDN. Les métadonnées d'installation
de bureau sont révisées dans `desktop-artifacts.json` et compilées dans la CLI.
Les entrées Community épinglent les artefacts d'une GitHub Release formelle avec
SHA-256 ; les entrées Pro et Local épinglent des artefacts CDN versionnés précis
avec leur taille et leur SHA-512 en Base64. La CLI ne fait jamais confiance à
un manifest mutable du CDN pendant l'installation. Mettez la CLI à jour pour
obtenir un registre d'artefacts de bureau plus récent et révisé.

## Sécurité

Ne signalez pas les vulnérabilités dans les Issues publiques. Suivez
[SECURITY.md](SECURITY.md) et utilisez le signalement privé des vulnérabilités
de GitHub. Chat2DB Community est un produit mono-utilisateur qui privilégie une
utilisation locale ; n'exposez pas directement son service local à des réseaux
non fiables.

## Contribution

Les Issues et les pull requests sont les bienvenues. Lisez
[CONTRIBUTING.md](CONTRIBUTING.md) et respectez le
[Code of Conduct](CODE_OF_CONDUCT.md).

## Licence

La CLI et l'Agent Skill de ce dépôt sont distribués sous
[Apache License 2.0](LICENSE). La licence de ce dépôt est indépendante de celle
appliquée au code source du serveur Chat2DB Community.

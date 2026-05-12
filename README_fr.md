# SubDispatch

[中文](README.md) | [English](README_en.md) | [日本語](README_ja.md) | [한국어](README_ko.md) | [Français](README_fr.md)

SubDispatch est une structure locale permettant à un LLM principal d'exécuter des agents de codage enfants en parallèle. Le LLM principal assure la planification, la revue, les décisions de fusion et la résolution des conflits. SubDispatch fournit uniquement l'exécution isolée, le polling de statut, la collecte des artifacts et le nettoyage des worktrees. Il se présente sous forme de binaire Rust unique pour CLI, MCP stdio, orchestration de workers, gestion des worktrees git, enregistrement des hooks Claude et UI locale Setup/Activity.

## Pourquoi cinq langues

Parce que le principe du projet est de déléguer le travail. Livrer un dispatcher d'agents parallèles avec un README monolingue aurait un petit côté « on a recruté une équipe, puis on a demandé à une seule personne d'écrire toutes les pancartes ». Le chinois reste l'entrée par défaut ; l'anglais, le japonais, le coréen et le français sont là pour que SubDispatch puisse au moins faire semblant d'avoir pensé à son passeport.

Les dépendances d'exécution sont intentionnellement minimales :

- `git`
- une CLI d'agent de code externe configurée, par défaut `claude`
- les identifiants API du modèle dans le fichier `.env` du workspace

Aucun runtime Python ou Node n'est requis.

## Hors objectifs

- Planification automatique des tâches
- Revue automatique
- Fusion ou cherry-pick automatique
- Résolution automatique des conflits
- Abstraction multi-provider

## Modèle central

SubDispatch suit deux entités :

- `Worker` : une commande d'agent de codage externe configurée. La valeur par défaut est `claude-code`.
- `Task` : une exécution d'agent enfant dans sa propre branche et worktree git.

Chaque tâche enregistre son commit de base, sa branche, le chemin du worktree, l'ID du processus, les logs, le chemin du manifest de résultat et le répertoire des artifacts.

## Configuration

SubDispatch lit la configuration locale du projet depuis `.env` à la racine du workspace. `.env` est ignoré par git. `.env.example` documente les clés supportées.

Créez le fichier local avec la CLI Rust :

```bash
subdispatch init-env
```

Puis modifiez `.env` directement. SubDispatch supporte le worker `claude-code` par défaut :

- `SUBDISPATCH_WORKER_MODE`
- `SUBDISPATCH_CLAUDE_ENABLED`
- `SUBDISPATCH_CLAUDE_PERMISSION_MODE`
- `SUBDISPATCH_CLAUDE_COMMAND`
- `SUBDISPATCH_CLAUDE_MODEL`
- `SUBDISPATCH_CLAUDE_MAX_CONCURRENCY`
- `ANTHROPIC_API_KEY`
- `ANTHROPIC_BASE_URL`

Le mode worker par défaut est `trusted-worktree` avec `bypassPermissions` de Claude Code. Ceci est intentionnel pour les boucles de codage déléguées où l'agent principal transfère la propriété d'exécution à l'agent enfant. Ce n'est pas un bac à sable de sécurité. SubDispatch s'appuie sur des worktrees git isolés, un périmètre de tâche explicite, des logs et une revue des artifacts post-tâche plutôt que sur une containisation pré-exécution.

La configuration des prompts est stockée séparément dans `.subdispatch/prompts.json`. Ce fichier est optionnel ; les valeurs par défaut intégrées sont utilisées s'il n'existe pas. La page Prompts de l'UI Web permet de modifier :

- Les descriptions des outils MCP
- Le template de prompt de l'agent enfant, les règles de sécurité et le schéma du manifest
- La stratégie de sélection du worker et les consignes de collect/review

Les métadonnées du worker sont configurées uniquement dans Setup/.env. Ceci maintient `description`, `strengths`, `cost`, `speed` et `delegation_trust` comme source de vérité unique. `delegation_trust` est une indication de routage pour l'agent principal, pas une garantie de sécurité.

Les modifications de prompt s'appliquent aux nouvelles listes d'outils MCP et aux nouvelles tâches enfants lancées. Les tâches existantes ne sont pas réécrites.

## Interfaces

### `list_workers`

Retourne les workers disponibles et leur capacité actuelle :

- ID du worker
- Commande du runner
- Modèle configuré
- Concurrent maximal
- Nombre en cours d'exécution
- Nombre en file d'attente
- Slots disponibles
- Confiance de délégation
- Raison d'indisponibilité, le cas échéant

L'outil MCP expose cette interface via `list_workers` ; la commande CLI correspondante est `subdispatch workers --workspace <path>`.

### `start_task`

Démarre une tâche enfant fournie par le LLM principal. SubDispatch crée une branche et un worktree, écrit un prompt de tâche, et lance le worker configuré lorsque la capacité est disponible. Une tâche dépassant la limite de concurrence du worker reste en file d'attente.

La délégation nécessite un checkpoint propre et commité. L'agent principal gère sa propre stratégie de branche/worktree et doit commiter les modifications en cours avant d'appeler `start_task`. SubDispatch ne gère pas de branche d'intégration cachée. Si le workspace contient des modifications non commitées, `start_task` retourne une erreur au lieu de créer un worktree enfant. Lorsque `base`/`base_branch` est omis, la tâche démarre depuis le `HEAD` courant.

Le parallélisme est explicite : l'agent principal appelle `start_task` plusieurs fois, sélectionne les workers en fonction des slots disponibles et de l'adéquation de la tâche, puis revue chaque résultat indépendamment.

Les tâches peuvent inclure un `context` ou `context_files` optionnel fourni par l'agent principal. C'est la bonne façon de donner à un agent enfant des diffs non commitées, des notes d'audit temporaires ou tout autre contexte qui n'est pas présent dans le commit de base du worktree enfant.

`read_scope`/`write_scope` ne doivent pas chevaucher `forbidden_paths`. SubDispatch rejette les contrats de périmètre contradictoires avant de créer un worktree de tâche. Le chemin du manifest de résultat géré est la seule écriture interne `.subdispatch` qu'une tâche enfant est censée effectuer.

### `poll_tasks`

Retourne le statut factuel des tâches, filtrable par `task_ids`, `status` ou `active_only`. Le polling rafraichit l'état du processus et démarre les tâches en file d'attente lorsque des slots de worker se libèrent.

Les statuts de tâche sont :

- `queued`
- `running`
- `completed`
- `failed`
- `cancelled`
- `missing`

### `collect_task`

Collecte les artifacts d'une tâche. SubDispatch calcule les fichiers modifiés et les diffs depuis Git plutôt que de faire confiance au manifest du worker. Il inclut les modifications non commitées du worktree car les agents enfants ne sont pas tenus de commiter.

Les artifacts retournés incluent :

- L'instruction originale
- Le manifest du worker, s'il est présent
- Les fins de stdout/stderr
- Les résultats compactés des commandes de validation du transcript Claude
- Les tentatives d'accès aux chemins interdits observées par les hooks scopés à la tâche
- Les fichiers modifiés
- La diff
- Le chemin du patch
- Le commit de base
- La branche de la tâche
- La vérification des chemins interdits
- La vérification du périmètre d'écriture

Considérez le manifest comme un auto-rapport du worker. Git diff, les vérifications de périmètre, `transcript_tool_results_tail` et `forbidden_path_attempts_tail` sont des preuves de revue plus fortes.

### `delete_worktree`

Supprime un worktree de tâche géré par SubDispatch. Il refuse de supprimer une tâche en cours d'exécution sauf en mode forcé. Par défaut, il préserve la branche et le répertoire des artifacts.

## Contraintes fortes

- Les agents enfants ne s'exécutent jamais dans le worktree principal.
- Chaque tâche a sa propre branche.
- Chaque tâche a son propre worktree.
- Chaque tâche enregistre un commit de base.
- `start_task` refuse les workspaces principaux sales.
- `collect_task` utilise Git comme source de vérité.
- La suppression du worktree vérifie que la cible est sous la racine des worktrees SubDispatch.
- Les artifacts sont préservés par défaut.
- Les limites de concurrence du worker sont appliquées.

## CLI Rust

Pendant le développement local :

```bash
cargo run -- workers --workspace .
cargo run -- mcp --workspace .
cargo run -- serve --workspace . --bind 127.0.0.1:8765
```

L'utilisation packagée est identique sans `cargo run --` :

```bash
subdispatch workers --workspace .
subdispatch mcp --workspace .
subdispatch serve --workspace . --bind 127.0.0.1:8765
```

L'UI Web n'est intentionnellement pas une console de tâches. Elle fournit les vérifications Setup, l'initialisation de `.env`, la capacité des workers, le statut des tâches, le nombre de fichiers modifiés et l'activité des hooks Claude. Le LLM principal crée toujours les tâches via MCP ou CLI.

## Installation et publication

Installez l'entrée MCP globale et le skill intégré une fois :

```bash
subdispatch install-skill
subdispatch install --global
```

Puis initialisez chaque projet :

```bash
cd /path/to/project
subdispatch init-env --workspace .
subdispatch doctor --workspace .
```

Créez une archive de publication locale :

```bash
scripts/release.sh
```

Voir [docs/rust-release.md](docs/rust-release.md) pour les détails d'empaquetage et [docs/python-removal-plan.md](docs/python-removal-plan.md) pour l'historique de suppression du Python MVP.

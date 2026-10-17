<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/rustload-lockup-dark.svg">
    <img src="docs/brand/rustload-lockup.svg" alt="Rustload" width="340">
  </picture>
</h1>

<p align="center">
  Le gestionnaire de téléchargements de votre serveur. Collez n'importe quoi : Rustload vérifie, range
  et télécharge à pleine vitesse.<br>
  Un seul petit binaire, une image Docker, une app de bureau et une interface web qui marche aussi
  sur téléphone.
</p>

<p align="center">
  <a href="https://github.com/rustload/rustload/releases/latest"><img src="https://img.shields.io/github/v/release/rustload/rustload?color=18212a" alt="Dernière version"></a>
  <a href="https://github.com/rustload/rustload/stargazers"><img src="https://img.shields.io/github/stars/rustload/rustload?color=c24e1b" alt="Étoiles GitHub"></a>
  <a href="https://github.com/rustload/rustload/actions/workflows/ci.yml"><img src="https://github.com/rustload/rustload/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/rustload/rustload/pkgs/container/rustload"><img src="https://img.shields.io/badge/docker-ghcr.io%2Frustload%2Frustload-c24e1b" alt="Image Docker ghcr.io/rustload/rustload"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-18212a" alt="Licence MIT"></a>
</p>

<p align="center">
  <a href="README.md">English version</a> ·
  <a href="docs/configuration.md">Configuration (EN)</a> ·
  <a href="docs/API.md">API (EN)</a> ·
  <a href="docs/architecture.md">Architecture (EN)</a> ·
  <a href="extension/README.md">Extension de navigateur (EN)</a>
</p>

![Tableau de bord : ce que fait Rustload en une phrase, puis chaque paquet avec sa progression et ses fichiers](docs/screenshots/dashboard.png)

## Ce que fait Rustload

**Tout récupérer.** Collez des liens, des magnets ou une page web entière, ou déposez des fichiers
`.torrent`. Le « link grabber » vérifie chaque lien avant de démarrer : nom, taille, disponibilité,
compte qui va le télécharger, et doublons déjà présents. Les archives en plusieurs parties, les
dossiers et les playlists forment leur propre paquet. Les pages web sont fouillées à la demande pour
en extraire les liens téléchargeables.

**Télécharger vite.** Chaque fichier est découpé entre plusieurs connexions qui se répartissent le
travail, s'adaptent aux serveurs qui limitent les connexions et reprennent exactement là où elles
s'étaient arrêtées. La file d'attente le montre : une carte de chaque fichier, avec un repère
lumineux là où chaque connexion écrit.

**Débrideurs et comptes premium.** AllDebrid, Real-Debrid, Premiumize.me, Debrid-Link et TorBox
pour des centaines d'hébergeurs, les magnets et les torrents ; comptes premium 1fichier, Rapidgator
et Keep2Share (FileBoom, Tezfiles) via leurs API officielles. Les comptes sont essayés dans votre
ordre, et Rustload passe au suivant quand un quota est atteint.

**Sites vidéo et audio.** YouTube, Vimeo, Twitch, SoundCloud, X, Instagram, TikTok et une vingtaine
d'autres sites grâce à yt-dlp, plus toute page que yt-dlp connaît si *Essayer yt-dlp sur les
autres pages web* est activé : choix de la qualité ou audio seul, playlists complètes, sous-titres,
SponsorBlock. Les flux HLS (`.m3u8`) deviennent un seul fichier vidéo et les listes M3U leurs
liens. yt-dlp se met à jour tout seul ; ffmpeg est inclus dans l'image Docker et s'installe en un
clic dans l'app Windows.

**Des fichiers fiables.** Extraction automatique (RAR, 7z, ZIP, tar, multi-volumes, liste de mots de
passe), vérification des sommes de contrôle (SFV, MD5, SHA) avec une marque « vérifié », catégories
qui choisissent leur dossier par règle ou par extension (volumes montés bienvenus), et propriétaires de fichiers propres sous Docker
(`PUID`/`PGID`/`UMASK`).

**Autonome.** Plage horaire de téléchargement et règles de vitesse (« lent en journée, pleine
vitesse la nuit »), protection de l'espace disque, notifications (Discord, Telegram, ntfy, Gotify,
webhooks, navigateur), scripts en fin de téléchargement, statistiques, diagnostic, sauvegarde et
restauration.

**Envoyer des liens de partout.** L'extension de navigateur (Chrome, Edge, Brave, Firefox), un
favori, le menu de partage du téléphone (installez Rustload comme une application), un bot
Telegram, un dossier surveillé, Click'n'Load et une API REST avec clé. Des métriques Prometheus pour
votre Grafana.

**Agréable à utiliser.** Pensé pour le clavier (palette de commandes <kbd>Ctrl</kbd>+<kbd>K</kbd>,
<kbd>N</kbd> pour ajouter, collez des liens n'importe où), thèmes sombre et clair, anglais et
français, conçu aussi pour le téléphone.

Rustload est écrit en Rust et reste léger : un binaire d'environ 16 Mo avec l'interface intégrée,
environ 8 Mo de mémoire au repos et moins de 20 Mo en téléchargeant à 35 Mio/s sur 8 connexions,
SQLite pour le stockage. Il peut remplacer pyLoad ou
JDownloader sur un serveur maison ; la résolution de captchas pour les hébergeurs gratuits n'est pas
prise en charge : utilisez un compte débrideur ou premium pour ceux-là.

## Captures d'écran

| Link grabber | Statistiques | Téléphone |
|---|---|---|
| ![Le link grabber vérifie les liens avant de les ajouter](docs/screenshots/grabber.png) | ![Statistiques : volume par jour, heures les plus actives, hébergeurs et sources](docs/screenshots/stats.png) | ![Tableau de bord sur téléphone, thème clair](docs/screenshots/mobile.png) |

## Démarrage rapide (Docker)

```yaml
# docker-compose.yml
services:
  rustload:
    image: ghcr.io/rustload/rustload:latest
    container_name: rustload
    restart: unless-stopped
    ports:
      - "8800:8800"
    environment:
      PUID: "1000"   # les fichiers appartiennent à cet utilisateur…
      PGID: "1000"   # …et à ce groupe (voir `id` sur l'hôte)
      UMASK: "022"
      TZ: "Europe/Paris"
    volumes:
      - ./config:/config
      - /srv/downloads:/downloads
```

```sh
docker compose up -d
```

Ouvrez `http://<serveur>:8800`, créez le compte administrateur et suivez la liste de configuration
du tableau de bord. Les images sont publiées pour `linux/amd64` et `linux/arm64`.
[docs/configuration.md](docs/configuration.md) détaille toutes les options : variables
d'environnement, catégories, planification et règles de vitesse, comptes, sites vidéo, dossier
surveillé, Click'n'Load, bot Telegram, Prometheus, scripts et proxy inverse.

## App de bureau

Rustload existe aussi en app pour Windows, macOS et Linux (AppImage, .deb), dans la
[dernière version](https://github.com/rustload/rustload/releases/latest) : une fenêtre à part et une
icône dans la zone de notification qui affiche la vitesse, met tout en pause et continue une fois la
fenêtre fermée. Au premier lancement, elle demande comment vous voulez l'utiliser :

- **Sur cet ordinateur** : Rustload télécharge dans *Téléchargements › Rustload*, s'ouvre sans
  connexion et peut se lancer au démarrage. Sous Windows, 7-Zip et ffmpeg s'installent en un clic
  depuis les paramètres.
- **Avec votre serveur Rustload** : l'app pilote le Rustload de votre NAS ou de votre serveur Docker
  avec son adresse et sa clé d'API (*Paramètres › Intégrations*), sans ouvrir de navigateur.

L'extension de navigateur se connecte en un clic à l'app du même ordinateur : choisissez *L'app
Rustload sur cet ordinateur* dans ses réglages et autorisez-la dans l'app.

Dans les deux cas, elle ouvre les liens magnet, signale les paquets terminés ou en échec par des
notifications système et se met à jour toute seule (mises à jour signées, installées quand vous ne
l'utilisez pas). Sous Windows, `Rustload_…_x64-portable.zip` fonctionne sans installation :
décompressez-le où vous voulez, clé USB comprise ; son dossier `data` garde tout ce que l'app écrit. Les installeurs ne sont pas encore signés : Windows SmartScreen demande une
confirmation (*Informations complémentaires › Exécuter quand même*) et macOS veut que l'app soit
ouverte depuis son menu contextuel la première fois.

## Sans Docker

Chaque [version](https://github.com/rustload/rustload/releases/latest) fournit un binaire prêt à
l'emploi pour Linux (x86-64 et ARM64, statique : toutes distributions), Windows (x86-64) et macOS
(Apple silicon et Intel). Décompressez-le et lancez-le :

```sh
./rustload        # rustload.exe sous Windows
```

Rustload écoute sur le port 8800 et garde ses réglages dans `./config` et ses téléchargements dans
`./downloads`, à côté de l'endroit où il est lancé ; `RUSTLOAD_PORT`, `RUSTLOAD_CONFIG_DIR` et
`RUSTLOAD_DOWNLOAD_DIR` changent cela. Rustload installe yt-dlp lui-même depuis *Paramètres › Sites
vidéo* ; sous Windows, il installe de la même façon 7-Zip (archives RAR/7z) et ffmpeg (sites vidéo),
ailleurs ils viennent de votre gestionnaire de paquets. Le binaire macOS
n'est pas notarié : autorisez-le une fois avec `xattr -d com.apple.quarantine rustload`.

## Compiler depuis les sources

Prérequis : Rust 1.94+, Node.js 22.12+ avec pnpm. Facultatif à l'exécution : 7-Zip (`7zz`, `7z` ou
`7za`) pour RAR/7z, yt-dlp et ffmpeg pour les sites vidéo (Rustload sait installer yt-dlp seul).

```sh
cd web && pnpm install && pnpm build && cd ..   # l'interface est intégrée au binaire
cargo build --release
RUSTLOAD_CONFIG_DIR=./config RUSTLOAD_DOWNLOAD_DIR=./downloads ./target/release/rustload
```

Pour construire l'image Docker : `docker buildx build --platform linux/amd64,linux/arm64 -t rustload .`
Voir [CONTRIBUTING.md](CONTRIBUTING.md) pour le développement.

## Feuille de route

- Résolution de captchas assistée par le navigateur pour les hébergeurs gratuits
- D'autres hébergeurs premium avec API officielle
- Surveillance de flux RSS et de sites pour ajouter automatiquement les nouveautés

## Usage responsable

Rustload télécharge ce que vous lui demandez, depuis les services auxquels vous avez accès.
Respectez le droit d'auteur et les conditions d'utilisation des services que vous utilisez :
Rustload n'héberge aucun contenu et ne donne accès à rien que vous ne pourriez télécharger
vous-même. Il n'est ni affilié aux débrideurs, hébergeurs et sites vidéo avec lesquels il
fonctionne, ni approuvé par eux ; leurs noms sont des marques de leurs propriétaires respectifs.

## Licence

[MIT](LICENSE). La licence couvre le code, pas le nom ni le logo de Rustload, décrits dans la
[charte graphique](docs/brand/README.md) (en anglais) ; les œuvres tierces incluses sont listées dans
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). Rustload est un projet indépendant, sans lien avec
le Rust Project ni la Rust Foundation. Les contributions suivent le
[code de conduite](CODE_OF_CONDUCT.md) (en anglais).

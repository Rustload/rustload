//! Backup and restore of the configuration: settings, categories, debrid accounts (with their
//! credentials) and the unfinished queue, saved as the links to add again. Users, sessions, the
//! history and statistics are not part of it.
//!
//! A restore replaces the settings, merges categories (by name) and accounts (by provider and
//! what identifies the account in its credentials) in one transaction, then optionally adds the
//! saved queue again, paused.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    db::{
        self, Db, Table,
        categories::{CategoryInput, normalize_extensions},
    },
    debrid,
    engine::{
        Engine,
        intake::{self, AddOptions, Source},
        remote_torrent_path,
    },
    media::MediaOptions,
    models::{AccountRow, CategoryRow, SourceKind, Stage},
    settings::{Settings, SettingsStore},
    util::now,
};

pub const FORMAT: u64 = 1;
pub const APP: &str = "rustload";

/// The backup document (see `docs/API.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub format: u64,
    pub app: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub created_at: i64,
    pub settings: Settings,
    #[serde(default)]
    pub categories: Vec<BackupCategory>,
    #[serde(default)]
    pub accounts: Vec<BackupAccount>,
    #[serde(default)]
    pub queue: Vec<BackupPackage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupCategory {
    /// Id in the saved instance: maps `settings.watch.category_id` to the restored category.
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupAccount {
    pub provider: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Provider specific (API key, OAuth tokens...), without session tokens (see
    /// [`debrid::without_session`]).
    pub credentials: Value,
}

fn default_true() -> bool {
    true
}

/// An unfinished package, as what is needed to add it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupPackage {
    pub name: String,
    /// Category name; `None` = automatic.
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub options: PackageOptions,
    /// Links of the files not downloaded yet (`links` packages).
    #[serde(default)]
    pub links: Vec<String>,
    pub source_kind: SourceKind,
    /// Magnet URI, or file name of the torrent.
    #[serde(default)]
    pub source: Option<String>,
    /// Content of the `.torrent` file, base64.
    #[serde(default)]
    pub torrent: Option<String>,
}

/// Missing values use the settings defaults, like [`AddOptions`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PackageOptions {
    pub create_subfolder: Option<bool>,
    pub extract: Option<bool>,
    pub delete_archives: Option<bool>,
    pub archive_password: Option<String>,
    /// Quality and extras of media links (yt-dlp), absent for other packages.
    pub media: Option<MediaOptions>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum BackupError {
    #[error("this file is not a Rustload backup")]
    NotABackup,
    #[error("this backup uses format {0}, which this version of Rustload cannot read")]
    UnsupportedFormat(u64),
    #[error("invalid backup: {0}")]
    Invalid(String),
}

/// What a restore did.
#[derive(Debug, Serialize)]
pub struct RestoreSummary {
    pub settings: bool,
    /// Categories updated or created.
    pub categories: usize,
    /// Accounts added.
    pub accounts: usize,
    /// Packages added to the queue again.
    pub packages: usize,
    /// Parts that could not be restored (unknown provider, package rejected...).
    pub errors: Vec<String>,
}

/// Builds a backup of the current configuration and queue.
pub async fn export(db: &Db, settings: &Settings, config_dir: &Path) -> anyhow::Result<Backup> {
    let categories = db::categories::list(db).await?;
    let accounts = db::accounts::list(db).await?;
    let packages = db::packages::list_queue(db).await?;
    let ids: Vec<i64> = packages.iter().map(|p| p.id).collect();
    let mut files = db::downloads::list_by_packages(db, &ids).await?;

    let mut queue = Vec::new();
    for package in packages {
        // Everything is downloaded already: there is nothing to add again.
        if package.stage == Stage::Extract {
            continue;
        }
        let mut entry = BackupPackage {
            category: package.category_id.and_then(|id| categories.iter().find(|c| c.id == id)).map(|c| c.name.clone()),
            options: PackageOptions {
                create_subfolder: Some(package.create_subfolder),
                extract: Some(package.extract),
                delete_archives: Some(package.delete_archives),
                archive_password: package.archive_password.clone(),
                media: package.media_options(),
            },
            links: Vec::new(),
            source_kind: package.source_kind,
            source: package.source.clone(),
            torrent: None,
            name: package.name,
        };
        match package.source_kind {
            SourceKind::Links => {
                let files = files.remove(&package.id).unwrap_or_default();
                entry.links = files.into_iter().filter(|f| !f.status.is_done()).map(|f| f.url).collect();
                if entry.links.is_empty() {
                    continue;
                }
            }
            SourceKind::Magnet if entry.source.is_none() => continue,
            SourceKind::Magnet => {}
            SourceKind::Torrent => {
                let path = remote_torrent_path(config_dir, package.id);
                entry.torrent = tokio::fs::read(path).await.ok().map(|data| BASE64.encode(data));
            }
        }
        queue.push(entry);
    }

    Ok(Backup {
        format: FORMAT,
        app: APP.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        created_at: now(),
        settings: settings.clone(),
        categories: categories
            .into_iter()
            .map(|c| BackupCategory {
                id: Some(c.id),
                extensions: c.extension_list(),
                name: c.name,
                path: c.path,
                is_default: c.is_default,
            })
            .collect(),
        accounts: accounts
            .into_iter()
            .map(|a| BackupAccount {
                credentials: debrid::without_session(&a.credentials),
                provider: a.provider,
                label: a.label,
                enabled: a.enabled,
            })
            .collect(),
        queue,
    })
}

/// Reads a backup file. `app` and `format` are checked first, so that any other JSON file gets
/// a clear message; the settings are validated like the ones saved from the interface.
pub fn parse(data: &[u8]) -> Result<Backup, BackupError> {
    let value: Value = serde_json::from_slice(data).map_err(|_| BackupError::NotABackup)?;
    if value.get("app").and_then(Value::as_str) != Some(APP) {
        return Err(BackupError::NotABackup);
    }
    match value.get("format").and_then(Value::as_u64) {
        Some(FORMAT) => {}
        Some(other) => return Err(BackupError::UnsupportedFormat(other)),
        None => return Err(BackupError::NotABackup),
    }
    let mut backup: Backup = serde_json::from_value(value).map_err(|e| BackupError::Invalid(e.to_string()))?;
    backup.settings = backup.settings.validate().map_err(BackupError::Invalid)?;
    if backup.categories.iter().any(|c| c.name.trim().is_empty()) {
        return Err(BackupError::Invalid("every category needs a name".into()));
    }
    Ok(backup)
}

/// Applies a backup; the queue is added again (paused) when `with_queue` is set.
pub async fn restore(engine: &Arc<Engine>, backup: Backup, with_queue: bool) -> anyhow::Result<RestoreSummary> {
    let applied = apply_configuration(&engine.settings, &backup).await?;
    engine.debrid.reload().await?;
    if !applied.accounts.is_empty() {
        // Fetch premium status and traffic of the new accounts in the background.
        let (engine, ids) = (engine.clone(), applied.accounts.clone());
        tokio::spawn(async move {
            for id in ids {
                if let Ok(Some(row)) = db::accounts::get(&engine.db, id).await {
                    let _ = engine.debrid.check(&row).await;
                }
            }
        });
    }

    let mut summary = RestoreSummary {
        settings: true,
        categories: applied.categories,
        accounts: applied.accounts.len(),
        packages: 0,
        errors: applied.errors,
    };
    if with_queue {
        for package in &backup.queue {
            match add_again(engine, package, &applied.category_ids).await {
                Ok(()) => summary.packages += 1,
                Err(error) => summary.errors.push(format!("{}: {error}", package.name)),
            }
        }
    }
    tracing::info!(
        "backup restored: settings, {} categories, {} new account(s), {} package(s)",
        summary.categories,
        summary.accounts,
        summary.packages
    );
    for error in &summary.errors {
        tracing::warn!("backup restore: {error}");
    }
    Ok(summary)
}

/// Outcome of the database part of a restore.
struct Applied {
    categories: usize,
    /// Ids of the added accounts.
    accounts: Vec<i64>,
    /// Category ids by lowercase name, to place restored packages.
    category_ids: HashMap<String, i64>,
    errors: Vec<String>,
}

/// Settings, categories and accounts, in one transaction.
async fn apply_configuration(store: &SettingsStore, backup: &Backup) -> anyhow::Result<Applied> {
    let mut transaction = store.begin().await?;
    let conn = &mut *transaction.tx;

    let current = db::categories::list(&mut *conn).await?;
    let plan = plan_categories(&current, &backup.categories);
    let mut saved = Vec::with_capacity(plan.items.len());
    for (position, item) in plan.items.iter().enumerate() {
        let input = CategoryInput {
            name: item.name.clone(),
            path: item.path.clone(),
            extensions: item.extensions.clone(),
            is_default: false,
        };
        saved.push(match item.existing {
            Some(id) => {
                db::categories::update_in(conn, id, &input).await?;
                id
            }
            None => db::categories::insert_at(conn, &input, position as i64).await?,
        });
    }
    if !saved.is_empty() {
        // Backup order first, then the categories missing from the backup.
        db::reorder_in(conn, Table::Categories, &saved).await?;
    }
    if let Some(index) = plan.default {
        db::categories::set_default(&mut *conn, saved[index]).await?;
    }
    let mut category_ids: HashMap<String, i64> = HashMap::new();
    let names = current.iter().map(|c| (c.name.as_str(), c.id));
    for (name, id) in plan.items.iter().map(|i| i.name.as_str()).zip(saved.iter().copied()).chain(names) {
        category_ids.entry(name_key(name)).or_insert(id);
    }

    let current_accounts = db::accounts::list(&mut *conn).await?;
    let accounts = plan_accounts(&current_accounts, &backup.accounts);
    let mut added = Vec::with_capacity(accounts.add.len());
    for account in &accounts.add {
        let label = match account.label.trim() {
            "" => debrid::provider(&account.provider).map(|p| p.name.to_owned()).unwrap_or_default(),
            label => label.to_owned(),
        };
        let credentials = match &account.credentials {
            Value::String(raw) => raw.clone(),
            other => other.to_string(),
        };
        added.push(db::accounts::insert(&mut *conn, &account.provider, &label, &credentials, account.enabled).await?);
    }

    let mut settings = backup.settings.clone();
    settings.watch.category_id = settings
        .watch
        .category_id
        .and_then(|old| plan.items.iter().position(|item| item.backup_id == Some(old)).map(|index| saved[index]));
    if settings.integrations.api_key.is_empty() {
        settings.integrations.api_key = store.get().integrations.api_key.clone();
    }
    transaction.commit(settings).await?;

    Ok(Applied { categories: saved.len(), accounts: added, category_ids, errors: accounts.errors })
}

async fn add_again(engine: &Engine, package: &BackupPackage, categories: &HashMap<String, i64>) -> Result<(), String> {
    let options = AddOptions {
        name: Some(package.name.clone()),
        category_id: package.category.as_deref().and_then(|name| categories.get(&name_key(name)).copied()),
        create_subfolder: package.options.create_subfolder,
        extract: package.options.extract,
        delete_archives: package.options.delete_archives,
        archive_password: package.options.archive_password.clone(),
        paused: Some(true),
        media: package.options.media.clone().map(Into::into),
    };
    let source = match package.source_kind {
        SourceKind::Links => Source::Links(package.links.clone()),
        SourceKind::Magnet => Source::Magnet(package.source.clone().ok_or("the magnet link is missing")?),
        SourceKind::Torrent => {
            let data = package
                .torrent
                .as_deref()
                .and_then(|data| BASE64.decode(data).ok())
                .ok_or("the .torrent file is missing")?;
            Source::torrent(package.source.as_deref(), data)
        }
    };
    intake::add_again(engine, source, &options).await.map(|_| ()).map_err(|error| error.to_string())
}

/// A category after the restore, in the order of the backup.
#[derive(Debug, Clone, PartialEq)]
struct PlannedCategory {
    /// Current category with the same name, updated in place (its id stays the same).
    existing: Option<i64>,
    backup_id: Option<i64>,
    name: String,
    path: String,
    extensions: Vec<String>,
}

#[derive(Debug, PartialEq)]
struct CategoryPlan {
    items: Vec<PlannedCategory>,
    /// Item becoming the default category. `None` keeps the current default.
    default: Option<usize>,
}

/// Matches backup categories with current ones by name (case-insensitive). Current categories
/// missing from the backup are kept.
fn plan_categories(current: &[CategoryRow], backup: &[BackupCategory]) -> CategoryPlan {
    let mut matched = HashSet::new();
    let mut items = Vec::new();
    let mut default = None;
    for category in backup {
        let name = category.name.trim();
        if name.is_empty() {
            continue;
        }
        let existing =
            current.iter().find(|c| !matched.contains(&c.id) && name_key(&c.name) == name_key(name)).map(|c| c.id);
        matched.extend(existing);
        if category.is_default && default.is_none() {
            default = Some(items.len());
        }
        items.push(PlannedCategory {
            existing,
            backup_id: category.id,
            name: name.to_owned(),
            path: category.path.trim().to_owned(),
            extensions: normalize_extensions(&category.extensions),
        });
    }
    CategoryPlan { items, default }
}

#[derive(Debug, PartialEq)]
struct AccountPlan<'a> {
    add: Vec<&'a BackupAccount>,
    errors: Vec<String>,
}

/// Accounts to add: those of a known provider that are not already present (same provider and
/// same [`debrid::account_identity`]). Current accounts are compared as they would be exported.
fn plan_accounts<'a>(current: &[AccountRow], backup: &'a [BackupAccount]) -> AccountPlan<'a> {
    let identity = |credentials: &str| debrid::account_identity(&debrid::without_session(credentials));
    let mut known: Vec<(&str, Value)> =
        current.iter().map(|a| (a.provider.as_str(), identity(&a.credentials))).collect();
    let mut plan = AccountPlan { add: Vec::new(), errors: Vec::new() };
    for account in backup {
        if debrid::provider(&account.provider).is_none() {
            plan.errors.push(format!("account `{}`: unknown provider `{}`", account.label, account.provider));
            continue;
        }
        let identity = debrid::account_identity(&account.credentials);
        // Values compare objects regardless of key order.
        if known.iter().any(|(provider, known)| *provider == account.provider && *known == identity) {
            continue;
        }
        known.push((&account.provider, identity));
        plan.add.push(account);
    }
    plan
}

fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn category(id: i64, name: &str, is_default: bool) -> CategoryRow {
        CategoryRow { id, name: name.into(), path: String::new(), extensions: String::new(), is_default, position: id }
    }

    fn saved_category(id: i64, name: &str, path: &str, is_default: bool) -> BackupCategory {
        BackupCategory { id: Some(id), name: name.into(), path: path.into(), extensions: Vec::new(), is_default }
    }

    fn account(provider: &str, credentials: Value) -> BackupAccount {
        BackupAccount { provider: provider.into(), label: format!("{provider} account"), enabled: true, credentials }
    }

    fn account_row(id: i64, provider: &str, credentials: &str) -> AccountRow {
        AccountRow {
            id,
            provider: provider.into(),
            label: provider.into(),
            credentials: credentials.into(),
            enabled: true,
            position: id,
            username: None,
            premium: false,
            premium_until: None,
            traffic_left: None,
            last_check: None,
            last_error: None,
        }
    }

    fn document(extra: Value) -> Vec<u8> {
        let mut doc = json!({ "format": 1, "app": "rustload", "version": "0.1.0", "settings": {} });
        doc.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::to_vec(&doc).unwrap()
    }

    #[test]
    fn checks_app_and_format_first() {
        assert_eq!(parse(b"not json").unwrap_err(), BackupError::NotABackup);
        assert_eq!(parse(br#"{"format": 1, "settings": {}}"#).unwrap_err(), BackupError::NotABackup);
        assert_eq!(parse(br#"{"format": 1, "app": "pyload"}"#).unwrap_err(), BackupError::NotABackup);
        assert_eq!(parse(br#"{"app": "rustload"}"#).unwrap_err(), BackupError::NotABackup);
        assert_eq!(parse(br#"{"format": 2, "app": "rustload"}"#).unwrap_err(), BackupError::UnsupportedFormat(2));
        // Right app and format, but no settings.
        assert!(matches!(parse(br#"{"format": 1, "app": "rustload"}"#), Err(BackupError::Invalid(_))));
    }

    #[test]
    fn validates_settings_and_categories() {
        let backup = parse(&document(json!({ "settings": { "downloads": { "connections": 4 } } }))).unwrap();
        assert_eq!(backup.settings.downloads.connections, 4);
        assert!(backup.queue.is_empty() && backup.accounts.is_empty());

        let bad_settings = document(json!({ "settings": { "downloads": { "connections": 0 } } }));
        assert!(matches!(parse(&bad_settings), Err(BackupError::Invalid(message)) if message.contains("connections")));
        let unnamed = document(json!({ "categories": [{ "name": " " }] }));
        assert!(matches!(parse(&unnamed), Err(BackupError::Invalid(_))));
    }

    #[test]
    fn merges_categories_by_name() {
        let current = [category(1, "Downloads", true), category(2, "Videos", false), category(3, "Mine", false)];
        let mut books = saved_category(7, "Books", "books", true);
        books.extensions = vec![".EPUB".into(), "pdf".into(), "pdf".into(), "bad ext".into()];
        let backup =
            [saved_category(5, " videos ", "/media/videos", false), books, saved_category(6, "Downloads", "", false)];
        let plan = plan_categories(&current, &backup);

        let summary: Vec<_> =
            plan.items.iter().map(|i| (i.existing, i.backup_id, i.name.as_str(), i.path.as_str())).collect();
        assert_eq!(
            summary,
            [
                (Some(2), Some(5), "videos", "/media/videos"),
                (None, Some(7), "Books", "books"),
                (Some(1), Some(6), "Downloads", "")
            ]
        );
        assert_eq!(plan.items[1].extensions, ["epub", "pdf"]);
        assert_eq!(plan.default, Some(1));

        // Without a default in the backup, the current default stays.
        assert_eq!(plan_categories(&current, &[saved_category(5, "Videos", "", false)]).default, None);
    }

    #[test]
    fn duplicate_names_match_distinct_categories() {
        let current = [category(1, "Videos", false), category(2, "Videos", false)];
        let backup = [
            saved_category(1, "Videos", "a", false),
            saved_category(2, "Videos", "b", false),
            saved_category(3, "Videos", "c", false),
        ];
        let existing: Vec<_> = plan_categories(&current, &backup).items.iter().map(|i| i.existing).collect();
        assert_eq!(existing, [Some(1), Some(2), None]);
    }

    #[test]
    fn adds_only_new_accounts() {
        let current = [account_row(1, "alldebrid", r#"{"api_key":"abc","extra":1}"#)];
        let backup = [
            // Same credentials, other key order: already present.
            account("alldebrid", json!({ "extra": 1, "api_key": "abc" })),
            account("alldebrid", json!({ "api_key": "def" })),
            // Same key, other provider: a different account.
            account("realdebrid", json!({ "api_key": "abc" })),
            // Listed twice in the backup.
            account("realdebrid", json!({ "api_key": "abc" })),
            account("nodebrid", json!({ "api_key": "x" })),
        ];
        let plan = plan_accounts(&current, &backup);
        assert_eq!(plan.add, [&backup[1], &backup[2]]);
        assert_eq!(plan.errors, ["account `nodebrid account`: unknown provider `nodebrid`"]);
    }

    #[test]
    fn recognises_accounts_whose_tokens_changed() {
        let current = [
            account_row(1, "rapidgator", r#"{"username":"Me@Example.com","password":"pw","token":"new"}"#),
            account_row(
                2,
                "realdebrid",
                r#"{"client_id":"C1","client_secret":"s","access_token":"a2","refresh_token":"r2","expires_at":200}"#,
            ),
        ];
        let backup = [
            // Older session token, or none (exported backups have none).
            account("rapidgator", json!({ "username": " me@example.com", "password": "pw", "token": "old" })),
            account("rapidgator", json!({ "username": "me@example.com", "password": "pw" })),
            // Older OAuth tokens of the same authorisation.
            account(
                "realdebrid",
                json!({ "client_id": "C1", "client_secret": "s", "access_token": "a1", "refresh_token": "r1", "expires_at": 100 }),
            ),
            // Another login, another authorisation.
            account("rapidgator", json!({ "username": "other@example.com", "password": "pw" })),
            account(
                "realdebrid",
                json!({ "client_id": "C2", "client_secret": "s", "access_token": "a", "refresh_token": "r", "expires_at": 1 }),
            ),
        ];
        let plan = plan_accounts(&current, &backup);
        assert_eq!(plan.add, [&backup[3], &backup[4]]);
        assert!(plan.errors.is_empty());
    }

    #[tokio::test]
    async fn exports_and_restores_through_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(&dir.path().join("rustload.db")).await.unwrap();
        db::categories::seed_defaults(&db).await.unwrap();
        db::accounts::insert(&db, "alldebrid", "Mine", r#"{"api_key":"abc"}"#, true).await.unwrap();
        let store = SettingsStore::load(db.clone()).await.unwrap();
        let api_key = store.get().integrations.api_key.clone();

        let mut backup = export(&db, &store.get(), dir.path()).await.unwrap();
        assert_eq!(backup.categories.len(), 4);
        assert_eq!(backup.accounts[0].credentials, json!({ "api_key": "abc" }));
        assert_eq!(backup.settings.integrations.api_key, api_key);

        // Restored elsewhere: Videos moves, Books is new and becomes the default, the watch
        // folder points to Books by its id in the saved instance.
        let videos = backup.categories.iter_mut().find(|c| c.name == "Videos").unwrap();
        videos.path = "/media/videos".into();
        let videos_id = videos.id;
        backup.categories.iter_mut().for_each(|c| c.is_default = false);
        backup.categories.insert(0, saved_category(42, "Books", "books", true));
        backup.settings.watch.category_id = Some(42);
        backup.settings.integrations.api_key = String::new();
        backup.accounts.push(account("torbox", json!({ "api_key": "xyz" })));
        let backup = parse(&serde_json::to_vec(&backup).unwrap()).unwrap();

        let applied = apply_configuration(&store, &backup).await.unwrap();
        assert_eq!(applied.categories, 5);
        assert_eq!(applied.accounts.len(), 1);
        let categories = db::categories::list(&db).await.unwrap();
        let books = categories.iter().find(|c| c.name == "Books").unwrap();
        assert!(books.is_default && books.position == 0);
        assert_eq!(categories.iter().filter(|c| c.is_default).count(), 1);
        let videos = categories.iter().find(|c| c.name == "Videos").unwrap();
        assert_eq!((Some(videos.id), videos.path.as_str()), (videos_id, "/media/videos"));
        assert_eq!(applied.category_ids["books"], books.id);
        assert_eq!(db::accounts::list(&db).await.unwrap().len(), 2);

        let settings = store.get();
        assert_eq!(settings.watch.category_id, Some(books.id));
        assert_eq!(settings.integrations.api_key, api_key);
        db.close().await;
    }
}

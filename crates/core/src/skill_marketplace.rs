//! Swarm-hosted skill marketplace catalog.
//!
//! Swarm owns discovery and validation of shared marketplace indexes; Dyson
//! owns installing validated `SKILL.md` bodies into its local workspace.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::db::skill_marketplace::{SkillMarketplaceSourceRow, SqlxSkillMarketplaceSourceStore};
use crate::error::StoreError;
use crate::http::ExternalHttpClient;
use crate::upstream_policy::OutboundUrlPolicy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_INDEX_BYTES: usize = 2 * 1024 * 1024;
const MAX_SKILL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SkillMarketplaceSourceConfig {
    File { id: String, path: PathBuf },
    Http { id: String, url: String },
}

impl SkillMarketplaceSourceConfig {
    pub fn id(&self) -> &str {
        match self {
            Self::File { id, .. } | Self::Http { id, .. } => id,
        }
    }

    pub fn source_type(&self) -> &'static str {
        match self {
            Self::File { .. } => "file",
            Self::Http { .. } => "http",
        }
    }

    pub fn location(&self) -> String {
        match self {
            Self::File { path, .. } => path.display().to_string(),
            Self::Http { url, .. } => url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillMarketplaceSourceView {
    pub id: String,
    pub source_type: String,
    pub location: String,
    /// Kept for older Dyson clients. DB-backed marketplaces are never
    /// implicit defaults, so this is always false.
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillMarketplaceAdminSourceView {
    pub id: String,
    pub source_type: String,
    pub location: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_fetch_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceIndex {
    pub schema_version: u32,
    pub marketplace: MarketplaceInfo,
    #[serde(default)]
    pub skills: Vec<MarketplaceSkillPackage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceSkillPackage {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub min_dyson_version: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    pub content: SkillPackageContent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SkillPackageContent {
    Inline { skill_md: String },
    Url { url: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogSkill {
    pub marketplace_id: String,
    pub marketplace_name: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub tags: Vec<String>,
    pub license: Option<String>,
    pub min_dyson_version: Option<String>,
    pub sha256: Option<String>,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<CatalogSkillAuthor>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogSkillAuthor {
    pub name: String,
    pub instance_id: String,
    pub href: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogError {
    pub marketplace_id: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogListing {
    pub sources: Vec<SkillMarketplaceSourceView>,
    pub skills: Vec<CatalogSkill>,
    pub errors: Vec<CatalogError>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillPackageDetail {
    pub skill: CatalogSkill,
    pub preview: String,
    pub computed_sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillPackageBody {
    pub marketplace_id: String,
    pub marketplace_name: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub declared_sha256: Option<String>,
    pub computed_sha256: String,
    pub skill_md: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SkillMarketplaceError {
    #[error("marketplace not found: {0}")]
    MarketplaceNotFound(String),
    #[error("skill not found: {marketplace}/{skill}")]
    SkillNotFound { marketplace: String, skill: String },
    #[error("invalid marketplace: {0}")]
    Invalid(String),
    #[error("marketplace I/O: {0}")]
    Io(String),
    #[error("marketplace HTTP: {0}")]
    Http(String),
    #[error("marketplace store: {0}")]
    Store(String),
}

#[derive(Clone)]
pub struct SkillMarketplaceService {
    store: Option<Arc<SqlxSkillMarketplaceSourceStore>>,
    external_http: ExternalHttpClient,
}

struct LoadedIndex {
    source: SkillMarketplaceSourceConfig,
    index: MarketplaceIndex,
}

impl SkillMarketplaceService {
    pub fn new(store: Arc<SqlxSkillMarketplaceSourceStore>) -> Self {
        Self::from_store(
            Some(store),
            ExternalHttpClient::new(Arc::new(OutboundUrlPolicy::default())),
        )
    }

    pub fn new_with_external_client(
        store: Arc<SqlxSkillMarketplaceSourceStore>,
        external_http: ExternalHttpClient,
    ) -> Self {
        Self::from_store(Some(store), external_http)
    }

    pub fn empty() -> Self {
        Self::from_store(
            None,
            ExternalHttpClient::new(Arc::new(OutboundUrlPolicy::default())),
        )
    }

    fn from_store(
        store: Option<Arc<SqlxSkillMarketplaceSourceStore>>,
        external_http: ExternalHttpClient,
    ) -> Self {
        Self {
            store,
            external_http,
        }
    }

    pub async fn source_views(
        &self,
    ) -> Result<Vec<SkillMarketplaceSourceView>, SkillMarketplaceError> {
        Ok(self
            .enabled_source_rows()
            .await?
            .into_iter()
            .map(|row| source_view(&row.source))
            .collect())
    }

    pub async fn admin_source_views(
        &self,
    ) -> Result<Vec<SkillMarketplaceAdminSourceView>, SkillMarketplaceError> {
        Ok(self
            .source_rows()
            .await?
            .into_iter()
            .map(|row| admin_source_view(&row))
            .collect())
    }

    pub async fn upsert_source(
        &self,
        source: SkillMarketplaceSourceConfig,
        enabled: bool,
    ) -> Result<SkillMarketplaceAdminSourceView, SkillMarketplaceError> {
        let store = self.store()?;
        let row = store.upsert(&source, enabled).await.map_err(store_err)?;
        Ok(admin_source_view(&row))
    }

    pub async fn delete_source(&self, id: &str) -> Result<bool, SkillMarketplaceError> {
        let store = self.store()?;
        store.delete(id).await.map_err(store_err)
    }

    pub async fn catalog(&self) -> CatalogListing {
        let rows = match self.enabled_source_rows().await {
            Ok(rows) => rows,
            Err(e) => {
                return CatalogListing {
                    sources: Vec::new(),
                    skills: Vec::new(),
                    errors: vec![CatalogError {
                        marketplace_id: "database".into(),
                        error: e.to_string(),
                    }],
                };
            }
        };
        let sources = rows
            .iter()
            .map(|row| source_view(&row.source))
            .collect::<Vec<_>>();
        let mut skills = Vec::new();
        let mut errors = Vec::new();

        for row in rows {
            let source = row.source;
            match self.load_index_recording(&source).await {
                Ok(loaded) => {
                    for package in &loaded.index.skills {
                        match catalog_skill(&loaded.index.marketplace, package) {
                            Ok(skill) => skills.push(skill),
                            Err(e) => errors.push(CatalogError {
                                marketplace_id: loaded.index.marketplace.id.clone(),
                                error: e.to_string(),
                            }),
                        }
                    }
                }
                Err(e) => errors.push(CatalogError {
                    marketplace_id: source.id().to_owned(),
                    error: e.to_string(),
                }),
            }
        }

        skills.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.marketplace_id.cmp(&b.marketplace_id))
        });
        CatalogListing {
            sources,
            skills,
            errors,
        }
    }

    pub async fn detail(
        &self,
        marketplace: &str,
        skill: &str,
    ) -> Result<SkillPackageDetail, SkillMarketplaceError> {
        let loaded = self.load_marketplace(marketplace).await?;
        let package = find_package(&loaded.index, skill).ok_or_else(|| {
            SkillMarketplaceError::SkillNotFound {
                marketplace: marketplace.to_owned(),
                skill: skill.to_owned(),
            }
        })?;
        let skill = catalog_skill(&loaded.index.marketplace, package)?;
        let body = self.fetch_skill_body(&loaded.source, package).await?;
        let preview = preview(&body);
        let computed_sha256 = skill_body_sha256(&body);
        Ok(SkillPackageDetail {
            skill,
            preview,
            computed_sha256,
        })
    }

    pub async fn content(
        &self,
        marketplace: &str,
        skill: &str,
    ) -> Result<SkillPackageBody, SkillMarketplaceError> {
        let loaded = self.load_marketplace(marketplace).await?;
        let package = find_package(&loaded.index, skill).ok_or_else(|| {
            SkillMarketplaceError::SkillNotFound {
                marketplace: marketplace.to_owned(),
                skill: skill.to_owned(),
            }
        })?;
        validate_skill_name(&package.name)?;
        let skill_md = self.fetch_skill_body(&loaded.source, package).await?;
        validate_skill_body(&skill_md)?;
        let computed_sha256 = skill_body_sha256(&skill_md);
        if let Some(declared) = package.sha256.as_deref()
            && !declared.eq_ignore_ascii_case(&computed_sha256)
        {
            return Err(SkillMarketplaceError::Invalid(format!(
                "sha256 mismatch for {marketplace}/{skill}: declared {declared}, computed {computed_sha256}"
            )));
        }
        Ok(SkillPackageBody {
            marketplace_id: loaded.index.marketplace.id.clone(),
            marketplace_name: loaded.index.marketplace.name.clone(),
            name: package.name.clone(),
            version: package.version.clone(),
            description: package.description.clone(),
            declared_sha256: package.sha256.clone(),
            computed_sha256,
            skill_md,
        })
    }

    async fn load_marketplace(
        &self,
        marketplace: &str,
    ) -> Result<LoadedIndex, SkillMarketplaceError> {
        for row in self.enabled_source_rows().await? {
            let source = row.source;
            if source.id() == marketplace {
                return self.load_index_recording(&source).await;
            }
        }
        Err(SkillMarketplaceError::MarketplaceNotFound(
            marketplace.to_owned(),
        ))
    }

    fn store(&self) -> Result<&Arc<SqlxSkillMarketplaceSourceStore>, SkillMarketplaceError> {
        self.store.as_ref().ok_or_else(|| {
            SkillMarketplaceError::Store("skill marketplace source store is not configured".into())
        })
    }

    async fn source_rows(&self) -> Result<Vec<SkillMarketplaceSourceRow>, SkillMarketplaceError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(Vec::new());
        };
        store.list().await.map_err(store_err)
    }

    async fn enabled_source_rows(
        &self,
    ) -> Result<Vec<SkillMarketplaceSourceRow>, SkillMarketplaceError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(Vec::new());
        };
        store.list_enabled().await.map_err(store_err)
    }

    async fn load_index_recording(
        &self,
        source: &SkillMarketplaceSourceConfig,
    ) -> Result<LoadedIndex, SkillMarketplaceError> {
        let loaded = self.load_index(source).await;
        if let Some(store) = self.store.as_ref() {
            match &loaded {
                Ok(_) => {
                    if let Err(err) = store.record_fetch_success(source.id()).await {
                        tracing::warn!(
                            error = %err,
                            marketplace = source.id(),
                            "skill marketplace fetch status update failed"
                        );
                    }
                }
                Err(fetch_err) => {
                    if let Err(err) = store
                        .record_fetch_error(source.id(), &fetch_err.to_string())
                        .await
                    {
                        tracing::warn!(
                            error = %err,
                            marketplace = source.id(),
                            "skill marketplace fetch error update failed"
                        );
                    }
                }
            }
        }
        loaded
    }

    async fn load_index(
        &self,
        source: &SkillMarketplaceSourceConfig,
    ) -> Result<LoadedIndex, SkillMarketplaceError> {
        let bytes = match source {
            SkillMarketplaceSourceConfig::File { path, .. } => {
                let path = expand_tilde(path);
                tokio::fs::read(&path)
                    .await
                    .map_err(|e| SkillMarketplaceError::Io(format!("{}: {e}", path.display())))?
            }
            SkillMarketplaceSourceConfig::Http { url, .. } => {
                let (http, url) = self
                    .external_http
                    .for_url(url)
                    .await
                    .map_err(|e| SkillMarketplaceError::Invalid(format!("bad URL: {e}")))?;
                if url.scheme() != "https" {
                    return Err(SkillMarketplaceError::Invalid(
                        "HTTP marketplace indexes must use https".into(),
                    ));
                }
                let resp = http
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| SkillMarketplaceError::Http(e.to_string()))?;
                if !resp.status().is_success() {
                    return Err(SkillMarketplaceError::Http(format!(
                        "index fetch returned {}",
                        resp.status()
                    )));
                }
                resp.bytes()
                    .await
                    .map_err(|e| SkillMarketplaceError::Http(e.to_string()))?
                    .to_vec()
            }
        };
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(SkillMarketplaceError::Invalid(format!(
                "index exceeds {MAX_INDEX_BYTES} bytes"
            )));
        }
        let index: MarketplaceIndex = serde_json::from_slice(&bytes)
            .map_err(|e| SkillMarketplaceError::Invalid(format!("index JSON: {e}")))?;
        validate_index(source, &index)?;
        Ok(LoadedIndex {
            source: source.clone(),
            index,
        })
    }

    async fn fetch_skill_body(
        &self,
        source: &SkillMarketplaceSourceConfig,
        package: &MarketplaceSkillPackage,
    ) -> Result<String, SkillMarketplaceError> {
        let body = match &package.content {
            SkillPackageContent::Inline { skill_md } => skill_md.clone(),
            SkillPackageContent::Url { url } => match source {
                SkillMarketplaceSourceConfig::File { path, .. } => {
                    let root = expand_tilde(path)
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from("."));
                    let rel = clean_relative_content_path(url)?;
                    let path = root.join(rel);
                    let bytes = tokio::fs::read(&path).await.map_err(|e| {
                        SkillMarketplaceError::Io(format!("{}: {e}", path.display()))
                    })?;
                    String::from_utf8(bytes)
                        .map_err(|e| SkillMarketplaceError::Invalid(format!("utf8: {e}")))?
                }
                SkillMarketplaceSourceConfig::Http { url: index_url, .. } => {
                    let index = reqwest::Url::parse(index_url)
                        .map_err(|e| SkillMarketplaceError::Invalid(format!("bad URL: {e}")))?;
                    let resolved = index
                        .join(url)
                        .map_err(|e| SkillMarketplaceError::Invalid(format!("content URL: {e}")))?;
                    if resolved.scheme() != "https" {
                        return Err(SkillMarketplaceError::Invalid(
                            "HTTP marketplace content URLs must use https".into(),
                        ));
                    }
                    let (http, resolved) = self
                        .external_http
                        .for_url(resolved.as_str())
                        .await
                        .map_err(|e| SkillMarketplaceError::Invalid(format!("content URL: {e}")))?;
                    let resp = http
                        .get(resolved)
                        .send()
                        .await
                        .map_err(|e| SkillMarketplaceError::Http(e.to_string()))?;
                    if !resp.status().is_success() {
                        return Err(SkillMarketplaceError::Http(format!(
                            "content fetch returned {}",
                            resp.status()
                        )));
                    }
                    let bytes = resp
                        .bytes()
                        .await
                        .map_err(|e| SkillMarketplaceError::Http(e.to_string()))?;
                    String::from_utf8(bytes.to_vec())
                        .map_err(|e| SkillMarketplaceError::Invalid(format!("utf8: {e}")))?
                }
            },
        };
        validate_skill_body(&body)?;
        Ok(body)
    }
}

fn source_view(source: &SkillMarketplaceSourceConfig) -> SkillMarketplaceSourceView {
    SkillMarketplaceSourceView {
        id: source.id().to_owned(),
        source_type: source.source_type().to_owned(),
        location: source.location(),
        is_default: false,
    }
}

fn admin_source_view(row: &SkillMarketplaceSourceRow) -> SkillMarketplaceAdminSourceView {
    SkillMarketplaceAdminSourceView {
        id: row.source.id().to_owned(),
        source_type: row.source.source_type().to_owned(),
        location: row.source.location(),
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
        last_fetch_at: row.last_fetch_at,
        last_success_at: row.last_success_at,
        last_error: row.last_error.clone(),
    }
}

fn store_err(err: StoreError) -> SkillMarketplaceError {
    SkillMarketplaceError::Store(err.to_string())
}

fn catalog_skill(
    marketplace: &MarketplaceInfo,
    package: &MarketplaceSkillPackage,
) -> Result<CatalogSkill, SkillMarketplaceError> {
    validate_skill_name(&package.name)?;
    Ok(CatalogSkill {
        marketplace_id: marketplace.id.clone(),
        marketplace_name: marketplace.name.clone(),
        name: package.name.clone(),
        version: package.version.clone(),
        description: package.description.clone(),
        tags: package.tags.clone(),
        license: package.license.clone(),
        min_dyson_version: package.min_dyson_version.clone(),
        sha256: package.sha256.clone(),
        content_type: match package.content {
            SkillPackageContent::Inline { .. } => "inline",
            SkillPackageContent::Url { .. } => "url",
        }
        .into(),
        author: None,
    })
}

fn find_package<'a>(
    index: &'a MarketplaceIndex,
    skill: &str,
) -> Option<&'a MarketplaceSkillPackage> {
    index.skills.iter().find(|package| package.name == skill)
}

fn validate_index(
    source: &SkillMarketplaceSourceConfig,
    index: &MarketplaceIndex,
) -> Result<(), SkillMarketplaceError> {
    if index.schema_version != 1 {
        return Err(SkillMarketplaceError::Invalid(format!(
            "unsupported schema_version {}",
            index.schema_version
        )));
    }
    if index.marketplace.id != source.id() {
        return Err(SkillMarketplaceError::Invalid(format!(
            "source id {:?} does not match index marketplace id {:?}",
            source.id(),
            index.marketplace.id
        )));
    }
    validate_marketplace_id(&index.marketplace.id)?;
    for package in &index.skills {
        validate_skill_name(&package.name)?;
    }
    Ok(())
}

pub fn validate_marketplace_source_config(
    source: &SkillMarketplaceSourceConfig,
) -> Result<(), SkillMarketplaceError> {
    validate_marketplace_id(source.id())?;
    match source {
        SkillMarketplaceSourceConfig::File { path, .. } => {
            if path.as_os_str().is_empty() {
                return Err(SkillMarketplaceError::Invalid(
                    "file marketplace path is empty".into(),
                ));
            }
        }
        SkillMarketplaceSourceConfig::Http { url, .. } => {
            let url = reqwest::Url::parse(url)
                .map_err(|e| SkillMarketplaceError::Invalid(format!("bad URL: {e}")))?;
            if url.scheme() != "https" {
                return Err(SkillMarketplaceError::Invalid(
                    "HTTP marketplace indexes must use https".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_marketplace_id(id: &str) -> Result<(), SkillMarketplaceError> {
    if is_valid_slug(id) {
        Ok(())
    } else {
        Err(SkillMarketplaceError::Invalid(format!(
            "invalid marketplace id {id:?}"
        )))
    }
}

pub fn validate_skill_name(name: &str) -> Result<(), SkillMarketplaceError> {
    if is_valid_slug(name) {
        Ok(())
    } else {
        Err(SkillMarketplaceError::Invalid(format!(
            "invalid skill name {name:?}"
        )))
    }
}

fn is_valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

pub fn validate_skill_body(body: &str) -> Result<(), SkillMarketplaceError> {
    if body.trim().is_empty() {
        return Err(SkillMarketplaceError::Invalid(
            "SKILL.md body is empty".into(),
        ));
    }
    if body.len() > MAX_SKILL_BYTES {
        return Err(SkillMarketplaceError::Invalid(format!(
            "SKILL.md exceeds {MAX_SKILL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn clean_relative_content_path(path: &str) -> Result<PathBuf, SkillMarketplaceError> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(SkillMarketplaceError::Invalid(
            "content path must be relative".into(),
        ));
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) if !part.is_empty() => out.push(part),
            _ => {
                return Err(SkillMarketplaceError::Invalid(
                    "content path must be clean".into(),
                ));
            }
        }
    }
    Ok(out)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn skill_body_sha256(body: &str) -> String {
    sha256_hex(body.as_bytes())
}

fn preview(body: &str) -> String {
    skill_body_preview(body)
}

pub fn skill_body_preview(body: &str) -> String {
    const MAX_PREVIEW: usize = 4096;
    if body.len() <= MAX_PREVIEW {
        return body.to_owned();
    }
    let mut end = MAX_PREVIEW;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn index_json(skill_md: &str, sha256: Option<&str>) -> String {
        let mut value = serde_json::json!({
            "schema_version": 1,
            "marketplace": {
                "id": "local",
                "name": "Local Skills"
            },
            "skills": [{
                "name": "code-review",
                "version": "1.0.0",
                "description": "Review code.",
                "tags": ["coding"],
                "content": {
                    "type": "inline",
                    "skill_md": skill_md
                }
            }]
        });
        if let Some(sha) = sha256 {
            value["skills"][0]["sha256"] = serde_json::Value::String(sha.to_string());
        }
        value.to_string()
    }

    #[tokio::test]
    async fn file_marketplace_lists_and_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        let skill = "---\ndescription: Review code.\n---\n\nRead the diff.";
        let hash = sha256_hex(skill.as_bytes());
        std::fs::write(
            dir.path().join("marketplace.json"),
            index_json(skill, Some(&hash)),
        )
        .unwrap();
        let pool = open_in_memory().await.unwrap();
        let store = Arc::new(SqlxSkillMarketplaceSourceStore::new(pool));
        store
            .upsert(
                &SkillMarketplaceSourceConfig::File {
                    id: "local".into(),
                    path: dir.path().join("marketplace.json"),
                },
                true,
            )
            .await
            .unwrap();
        let svc = SkillMarketplaceService::new(store);

        let listing = svc.catalog().await;
        assert!(listing.errors.is_empty());
        assert_eq!(listing.skills[0].name, "code-review");

        let body = svc.content("local", "code-review").await.unwrap();
        assert_eq!(body.skill_md, skill);
        assert_eq!(body.computed_sha256, hash);
    }

    #[tokio::test]
    async fn disabled_db_sources_do_not_feed_the_catalog() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("marketplace.json"),
            index_json("body", None),
        )
        .unwrap();
        let pool = open_in_memory().await.unwrap();
        let store = Arc::new(SqlxSkillMarketplaceSourceStore::new(pool));
        store
            .upsert(
                &SkillMarketplaceSourceConfig::File {
                    id: "local".into(),
                    path: dir.path().join("marketplace.json"),
                },
                false,
            )
            .await
            .unwrap();
        let svc = SkillMarketplaceService::new(store);

        let listing = svc.catalog().await;
        assert!(listing.sources.is_empty());
        assert!(listing.skills.is_empty());
        assert!(listing.errors.is_empty());
    }

    #[tokio::test]
    async fn rejects_bad_skill_names() {
        let idx = MarketplaceIndex {
            schema_version: 1,
            marketplace: MarketplaceInfo {
                id: "local".into(),
                name: "Local".into(),
                homepage: None,
            },
            skills: vec![MarketplaceSkillPackage {
                name: "../bad".into(),
                version: "1".into(),
                description: "bad".into(),
                tags: vec![],
                license: None,
                min_dyson_version: None,
                sha256: None,
                content: SkillPackageContent::Inline {
                    skill_md: "body".into(),
                },
            }],
        };
        let err = validate_index(
            &SkillMarketplaceSourceConfig::File {
                id: "local".into(),
                path: "marketplace.json".into(),
            },
            &idx,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid skill name"));
    }
}

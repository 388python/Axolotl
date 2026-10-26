use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::state::store::content_favorites::{
    ContentFavoriteKey, ContentFavoriteRecord, ContentFavorites,
};
use crate::state::store::resource_source_projects::{
    ResourceSourceProjectKey, ResourceSourceProjects,
};
use crate::state::store::{Collection as _, Handle};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFavoriteProvider {
    Modrinth,
    Curseforge,
    Mcarchive,
}

impl ContentFavoriteProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Modrinth => "modrinth",
            Self::Curseforge => "curseforge",
            Self::Mcarchive => "mcarchive",
        }
    }

    pub(crate) fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "modrinth" => Ok(Self::Modrinth),
            "curseforge" => Ok(Self::Curseforge),
            "mcarchive" => Ok(Self::Mcarchive),
            _ => Err(crate::ErrorKind::InputError(format!(
                "Unsupported content favorite provider: {value}"
            ))
            .into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFavoriteType {
    Mod,
    Resourcepack,
    Datapack,
    Shader,
}

impl ContentFavoriteType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::Resourcepack => "resourcepack",
            Self::Datapack => "datapack",
            Self::Shader => "shader",
        }
    }

    pub(crate) fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "mod" => Ok(Self::Mod),
            "resourcepack" => Ok(Self::Resourcepack),
            "datapack" => Ok(Self::Datapack),
            "shader" => Ok(Self::Shader),
            _ => Err(crate::ErrorKind::InputError(format!(
                "Unsupported content favorite type: {value}"
            ))
            .into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentFavorite {
    #[serde(default)]
    pub resource_id: Option<String>,
    pub provider: ContentFavoriteProvider,
    pub project_id: String,
    pub content_type: ContentFavoriteType,
    pub saved_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentFavoriteInput {
    pub provider: ContentFavoriteProvider,
    pub project_id: String,
    pub content_type: ContentFavoriteType,
}

impl ContentFavoriteInput {
    fn validate(&self) -> crate::Result<()> {
        if self.project_id.trim().is_empty() {
            return Err(crate::ErrorKind::InputError(
                "A content favorite must have a project ID".to_string(),
            )
            .into());
        }
        Ok(())
    }
}

pub async fn list(handle: &Handle) -> crate::Result<Vec<ContentFavorite>> {
    let mut favorites = Vec::new();
    for (key, record) in ContentFavorites::get_all(handle).await? {
        favorites.push(with_resource(handle, key, record).await?);
    }

    favorites.sort_by(|left, right| {
        right
            .saved_at
            .cmp(&left.saved_at)
            .then_with(|| left.provider.as_str().cmp(right.provider.as_str()))
            .then_with(|| left.project_id.cmp(&right.project_id))
    });
    Ok(favorites)
}

pub async fn add(
    favorite: ContentFavoriteInput,
    handle: &Handle,
) -> crate::Result<ContentFavorite> {
    favorite.validate()?;
    let key = ContentFavoriteKey {
        provider: favorite.provider,
        project_id: favorite.project_id.trim().to_string(),
    };
    // Re-adding a favorite keeps the moment it was first saved, so the list
    // only moves when a person saves something new.
    let saved_at = match ContentFavorites::get(handle, &key).await? {
        Some(existing) => existing.saved_at,
        None => Utc::now().timestamp_millis(),
    };
    let record = ContentFavoriteRecord {
        content_type: favorite.content_type,
        saved_at,
    };

    ContentFavorites::set(handle, &key, Some(&record)).await?;
    with_resource(handle, key, record).await
}

pub async fn remove(
    provider: ContentFavoriteProvider,
    project_id: &str,
    handle: &Handle,
) -> crate::Result<()> {
    if project_id.trim().is_empty() {
        return Err(crate::ErrorKind::InputError(
            "A content favorite must have a project ID".to_string(),
        )
        .into());
    }

    let key = ContentFavoriteKey {
        provider,
        project_id: project_id.trim().to_string(),
    };
    ContentFavorites::set(handle, &key, None).await
}

async fn with_resource(
    handle: &Handle,
    key: ContentFavoriteKey,
    record: ContentFavoriteRecord,
) -> crate::Result<ContentFavorite> {
    let resource_id = ResourceSourceProjects::get(
        handle,
        &ResourceSourceProjectKey {
            source: key.provider.as_str().to_string(),
            project_id: key.project_id.clone(),
        },
    )
    .await?
    .map(|project| project.resource_id);

    Ok(ContentFavorite {
        resource_id,
        provider: key.provider,
        project_id: key.project_id,
        content_type: record.content_type,
        saved_at: record.saved_at,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ContentFavoriteInput, ContentFavoriteProvider, ContentFavoriteType,
        add, list, remove,
    };
    use crate::state::store::content_favorites::{
        ContentFavoriteKey, ContentFavoriteRecord, ContentFavorites,
    };
    use crate::state::store::{Collection as _, Handle};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::time::Duration;

    async fn handle() -> Handle {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::migrate!().run(&pool).await.expect("migrations");
        Handle::new(pool)
    }

    #[tokio::test]
    async fn content_favorites_are_provider_qualified_and_idempotent() {
        let handle = handle().await;
        let modrinth = add(
            ContentFavoriteInput {
                provider: ContentFavoriteProvider::Modrinth,
                project_id: "same-id".to_string(),
                content_type: ContentFavoriteType::Mod,
            },
            &handle,
        )
        .await
        .expect("add Modrinth favorite");
        let duplicate = add(
            ContentFavoriteInput {
                provider: ContentFavoriteProvider::Modrinth,
                project_id: "same-id".to_string(),
                content_type: ContentFavoriteType::Mod,
            },
            &handle,
        )
        .await
        .expect("add duplicate favorite");
        add(
            ContentFavoriteInput {
                provider: ContentFavoriteProvider::Curseforge,
                project_id: "same-id".to_string(),
                content_type: ContentFavoriteType::Shader,
            },
            &handle,
        )
        .await
        .expect("add CurseForge favorite");

        let favorites = list(&handle).await.expect("list favorites");
        assert_eq!(favorites.len(), 2);
        assert_eq!(duplicate.saved_at, modrinth.saved_at);
        assert!(favorites.iter().any(|favorite| {
            favorite.provider == ContentFavoriteProvider::Modrinth
                && favorite.project_id == "same-id"
        }));
        assert!(favorites.iter().any(|favorite| {
            favorite.provider == ContentFavoriteProvider::Curseforge
                && favorite.project_id == "same-id"
        }));
    }

    #[tokio::test]
    async fn removing_a_content_favorite_is_idempotent() {
        let handle = handle().await;
        remove(ContentFavoriteProvider::Modrinth, "missing", &handle)
            .await
            .expect("remove missing favorite");
        assert!(list(&handle).await.expect("list favorites").is_empty());
    }

    #[tokio::test]
    async fn readding_after_remove_gets_a_new_saved_time() {
        let handle = handle().await;
        let first = add(
            ContentFavoriteInput {
                provider: ContentFavoriteProvider::Modrinth,
                project_id: "sodium".to_string(),
                content_type: ContentFavoriteType::Mod,
            },
            &handle,
        )
        .await
        .expect("add favorite");
        remove(ContentFavoriteProvider::Modrinth, "sodium", &handle)
            .await
            .expect("remove favorite");
        tokio::time::sleep(Duration::from_millis(2)).await;
        let second = add(
            ContentFavoriteInput {
                provider: ContentFavoriteProvider::Modrinth,
                project_id: "sodium".to_string(),
                content_type: ContentFavoriteType::Mod,
            },
            &handle,
        )
        .await
        .expect("re-add favorite");

        assert!(second.saved_at > first.saved_at);
    }

    #[tokio::test]
    async fn favorites_are_ordered_by_when_they_were_saved() {
        let handle = handle().await;
        for (provider, project_id, saved_at) in [
            (ContentFavoriteProvider::Modrinth, "older", 1),
            (ContentFavoriteProvider::Curseforge, "newer", 2),
        ] {
            ContentFavorites::set(
                &handle,
                &ContentFavoriteKey {
                    provider,
                    project_id: project_id.to_string(),
                },
                Some(&ContentFavoriteRecord {
                    content_type: ContentFavoriteType::Mod,
                    saved_at,
                }),
            )
            .await
            .expect("seed favorite");
        }

        let favorites = list(&handle).await.expect("list favorites");
        assert_eq!(
            favorites
                .iter()
                .map(|favorite| favorite.project_id.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
    }
}

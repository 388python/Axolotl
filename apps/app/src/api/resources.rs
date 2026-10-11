use dashmap::DashMap;
use std::sync::{Arc, LazyLock};
use theseus::resources::favorites::ResourceFavorite;
use theseus::resources::providers::{
    ProjectDocument, ProviderQuery, ProviderValue, ReleaseDocument,
};
use theseus::resources::{ipc::*, *};
use tokio_util::sync::CancellationToken;

use super::Result;

static REQUESTS: LazyLock<DashMap<RequestId, CancellationToken>> =
    LazyLock::new(DashMap::new);

struct RequestGuard(RequestId);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        REQUESTS.remove(&self.0);
    }
}

fn request(id: &RequestId) -> Result<(RequestGuard, CancellationToken)> {
    let token = CancellationToken::new();
    match REQUESTS.entry(id.clone()) {
        dashmap::mapref::entry::Entry::Occupied(_) => Err(
            ResourceError::InvalidRequest("request ID already active".into())
                .into(),
        ),
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            entry.insert(token.clone());
            Ok((RequestGuard(id.clone()), token))
        }
    }
}

async fn service() -> Result<Arc<ResourceService>> {
    Ok(theseus::State::get().await?.resources().await?.clone())
}

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("resources")
        .invoke_handler(tauri::generate_handler![
            resource_sources,
            resource_search,
            resource_get,
            resource_get_description,
            resource_list_releases,
            resource_get_release,
            resource_get_changelog,
            resource_list_changelogs,
            resource_cancel,
            resource_preferred_source,
            resource_set_preferred_source,
            resource_cache_info,
            resource_cache_clear,
            resource_locate,
            resource_project_document,
            resource_project_documents,
            resource_project_protocol,
            resource_release_documents,
            resource_favorites,
            resource_favorite_add,
            resource_favorite_remove,
            resource_prepare_operation,
            resource_revise_plan,
            resource_queue_plan
        ])
        .build()
}

#[tauri::command]
pub async fn resource_prepare_operation(
    instance_id: String,
    intent: ResourceOperationIntent,
    selections: Vec<ResourceSelection>,
    install_dependencies: Option<bool>,
    request_id: RequestId,
) -> Result<ResourceOperationPlan> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(theseus::resources::plans::prepare(
        instance_id,
        intent,
        selections,
        install_dependencies.unwrap_or(true),
        &cancel,
    )
    .await?)
}

#[tauri::command]
pub async fn resource_revise_plan(
    plan_id: PlanId,
    revision: u32,
    decisions: Vec<ResourcePlanDecision>,
    request_id: RequestId,
) -> Result<ResourceOperationPlan> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(
        theseus::resources::plans::revise(
            plan_id, revision, decisions, &cancel,
        )
        .await?,
    )
}

#[tauri::command]
pub async fn resource_queue_plan(
    plan_id: PlanId,
    revision: u32,
) -> Result<ResourceOperationPlan> {
    Ok(theseus::resources::plans::queue(plan_id, revision).await?)
}

#[tauri::command]
pub async fn resource_favorites() -> Result<Vec<ResourceFavorite>> {
    Ok(theseus::resources::favorites::list().await?)
}

#[tauri::command]
pub async fn resource_favorite_add(
    project: SourceProjectRef,
    content_type: ResourceType,
    locator: Option<String>,
) -> Result<()> {
    theseus::resources::favorites::add(project, content_type, locator).await?;
    Ok(())
}

#[tauri::command]
pub async fn resource_favorite_remove(project: SourceProjectRef) -> Result<()> {
    theseus::resources::favorites::remove(&project).await?;
    Ok(())
}

#[tauri::command]
pub async fn resource_locate(
    source: SourceId,
    locator: String,
    request_id: RequestId,
    cache_policy: Option<CachePolicy>,
) -> Result<ProjectDocument> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(service()
        .await?
        .locate_with_policy(
            &source,
            locator,
            &cancel,
            cache_policy.unwrap_or(CachePolicy::UseFresh),
        )
        .await?)
}

#[tauri::command]
pub async fn resource_project_document(
    project: SourceProjectRef,
    locator: Option<String>,
    request_id: RequestId,
) -> Result<ProjectDocument> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(service()
        .await?
        .project_document(&project, locator, &cancel)
        .await?)
}

#[tauri::command]
pub async fn resource_release_documents(
    source: SourceId,
    release_ids: Vec<SourceReleaseId>,
    project_id: Option<SourceProjectId>,
    request_id: RequestId,
    cache_policy: Option<CachePolicy>,
) -> Result<Vec<ReleaseDocument>> {
    let (_guard, cancel) = request(&request_id)?;
    let service = service().await?;
    let queries = release_ids
        .into_iter()
        .map(|release_id| ProviderQuery::Release {
            project_id: project_id.clone(),
            release_id,
            slug: None,
        })
        .collect();
    service
        .presentation_many_with_policy(
            &source,
            queries,
            &cancel,
            cache_policy.unwrap_or(CachePolicy::UseFresh),
        )
        .await
        .into_iter()
        .filter_map(|result| match result {
            Ok(QueryOutcome {
                value: ProviderValue::Release(document),
                ..
            }) => Some(Ok(document)),
            Ok(QueryOutcome {
                value: ProviderValue::NotFound,
                ..
            }) => None,
            Err(error) => Some(Err(error.into())),
            _ => Some(Err(ResourceError::InvalidResponse(
                "expected release document".into(),
            )
            .into())),
        })
        .collect()
}

#[tauri::command]
pub async fn resource_project_documents(
    source: SourceId,
    project_ids: Vec<SourceProjectId>,
    request_id: RequestId,
    cache_policy: Option<CachePolicy>,
) -> Result<Vec<ProjectDocument>> {
    let (_guard, cancel) = request(&request_id)?;
    let queries = project_ids
        .into_iter()
        .map(|id| ProviderQuery::Project { id, slug: None })
        .collect();
    service()
        .await?
        .presentation_many_with_policy(
            &source,
            queries,
            &cancel,
            cache_policy.unwrap_or(CachePolicy::UseFresh),
        )
        .await
        .into_iter()
        .filter_map(|result| match result {
            Ok(QueryOutcome {
                value: ProviderValue::Project(document),
                ..
            }) => Some(Ok(document)),
            Ok(QueryOutcome {
                value: ProviderValue::NotFound,
                ..
            }) => None,
            Err(error) => Some(Err(error.into())),
            _ => Some(Err(ResourceError::InvalidResponse(
                "expected project document".into(),
            )
            .into())),
        })
        .collect()
}

#[tauri::command]
pub async fn resource_project_protocol(
    locators: Vec<String>,
    protocol_version: String,
    request_id: RequestId,
    cache_policy: Option<CachePolicy>,
) -> Result<Vec<ProjectDocument>> {
    let (_guard, cancel) = request(&request_id)?;
    let queries = locators
        .into_iter()
        .map(|locator| ProviderQuery::ProjectProtocol {
            locator,
            version: protocol_version.clone(),
        })
        .collect();
    service()
        .await?
        .presentation_many_with_policy(
            &SourceId::modrinth(),
            queries,
            &cancel,
            cache_policy.unwrap_or(CachePolicy::UseFresh),
        )
        .await
        .into_iter()
        .filter_map(|result| match result {
            Ok(QueryOutcome {
                value: ProviderValue::Project(document),
                ..
            }) => Some(Ok(document)),
            Ok(QueryOutcome {
                value: ProviderValue::NotFound,
                ..
            }) => None,
            Err(error) => Some(Err(error.into())),
            _ => Some(Err(ResourceError::InvalidResponse(
                "expected project protocol document".into(),
            )
            .into())),
        })
        .collect()
}

#[tauri::command]
pub async fn resource_sources() -> Result<Vec<ResourceSource>> {
    Ok(service().await?.sources())
}

#[tauri::command]
pub async fn resource_search(
    query: ResourceSearchQuery,
    cursor: Option<String>,
    request_id: RequestId,
) -> Result<ResourcePage<ResourceSearchResult>> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(service()
        .await?
        .search(query, cursor, request_id, &cancel)
        .await?)
}

#[tauri::command]
pub async fn resource_get(
    reference: ResourceRef,
    source: Option<SourceId>,
    locator: Option<String>,
    request_id: RequestId,
) -> Result<Resource> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(service()
        .await?
        .get(&reference, source, locator, &cancel)
        .await?)
}

#[tauri::command]
pub async fn resource_get_description(
    reference: ResourceRef,
    source: SourceId,
    locator: Option<String>,
    request_id: RequestId,
) -> Result<ResourceText> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(service()
        .await?
        .description(&reference, &source, locator, &cancel)
        .await?)
}

#[tauri::command]
pub async fn resource_list_releases(
    reference: ResourceRef,
    target: ResourceTarget,
    cursor: Option<String>,
    locator: Option<String>,
    request_id: RequestId,
) -> Result<ResourcePage<ResourceRelease>> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(service()
        .await?
        .releases(&reference, target, cursor, request_id, locator, &cancel)
        .await?)
}

#[tauri::command]
pub async fn resource_get_release(
    reference: SourceReleaseRef,
    locator: Option<String>,
    request_id: RequestId,
) -> Result<ResourceRelease> {
    let (_guard, cancel) = request(&request_id)?;
    Ok(service()
        .await?
        .release(&reference, locator, &cancel)
        .await?)
}

#[tauri::command]
pub async fn resource_get_changelog(
    release_ref: SourceReleaseRef,
    locator: Option<String>,
    request_id: RequestId,
) -> Result<ResourceChangelog> {
    let (_guard, cancel) = request(&request_id)?;
    let service = service().await?;
    let release = service
        .release(&release_ref, locator.clone(), &cancel)
        .await?;
    let text = service.changelog(&release_ref, locator, &cancel).await?;
    Ok(ResourceChangelog {
        release: release_ref,
        published_at: release.published_at,
        text,
    })
}

#[tauri::command]
pub async fn resource_list_changelogs(
    resource_ref: ResourceRef,
    cursor: Option<String>,
    request_id: RequestId,
) -> Result<ResourcePage<ResourceChangelog>> {
    let (_guard, cancel) = request(&request_id)?;
    let service = service().await?;
    let page = service
        .releases(
            &resource_ref,
            ResourceTarget {
                game_versions: vec![],
                loaders: vec![],
                channels: vec![],
            },
            cursor,
            request_id.clone(),
            None,
            &cancel,
        )
        .await?;
    let mut entries = Vec::new();
    let mut failures = page.source_failures;
    for release in page.entries {
        match service.changelog(&release.reference, None, &cancel).await {
            Ok(text) => entries.push(ResourceChangelog {
                release: release.reference,
                published_at: release.published_at,
                text,
            }),
            Err(ResourceError::Canceled) => {
                return Err(ResourceError::Canceled.into());
            }
            Err(error) => failures.push(ResourceSourceFailure {
                source: release.reference.project.source,
                code: ResourceIssueCode::SourceUnavailable,
                message: error.to_string(),
                retryable: true,
            }),
        }
    }
    Ok(ResourcePage {
        request_id,
        entries,
        source_failures: failures,
        has_more: page.has_more,
        cursor: page.cursor,
        total: page.total,
        exhausted_sources: page.exhausted_sources,
    })
}

#[tauri::command]
pub fn resource_cancel(request_id: RequestId) {
    if let Some(token) = REQUESTS.get(&request_id) {
        token.cancel();
    }
}

#[tauri::command]
pub async fn resource_preferred_source() -> Result<SourceId> {
    Ok(service().await?.preferred_source().await?)
}

#[tauri::command]
pub async fn resource_set_preferred_source(source: SourceId) -> Result<()> {
    let service = service().await?;
    if !service
        .sources()
        .iter()
        .any(|candidate| candidate.id == source)
    {
        return Err(
            ResourceError::InvalidRequest("unknown source".into()).into()
        );
    }
    service
        .store()
        .ok_or_else(|| {
            ResourceError::Database("resource store unavailable".into())
        })?
        .set_preferred_source(&source)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn resource_cache_info() -> Result<ResourceCacheInfo> {
    Ok(service().await?.cache().info().await?)
}

#[tauri::command]
pub async fn resource_cache_clear() -> Result<()> {
    service().await?.cache().clear().await?;
    Ok(())
}

//! Protocol projections at the boundary of the existing environment solver.

use super::{providers::*, *};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

fn error(error: ResourceError) -> crate::Error {
    crate::ErrorKind::InputError(error.to_string()).into()
}

pub(crate) async fn exact<T: DeserializeOwned>(
    source: SourceId,
    project_id: Option<String>,
    release_id: String,
    state: &crate::State,
) -> crate::Result<Option<T>> {
    let service = state.resources().await.map_err(error)?;
    let value = service
        .presentation_query(
            &source,
            ProviderQuery::Release {
                project_id: project_id
                    .map(SourceProjectId::new)
                    .transpose()
                    .map_err(|error| {
                        crate::ErrorKind::InputError(error.to_string())
                    })?,
                release_id: SourceReleaseId::new(release_id).map_err(
                    |error| crate::ErrorKind::InputError(error.to_string()),
                )?,
                slug: None,
            },
            &CancellationToken::new(),
        )
        .await
        .map_err(error)?;
    match value.value {
        ProviderValue::Release(document) => document
            .protocol
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into),
        ProviderValue::NotFound => Ok(None),
        _ => Err(crate::ErrorKind::InputError(
            "unexpected release catalog response".into(),
        )
        .into()),
    }
}

pub(crate) async fn page<T: DeserializeOwned>(
    source: SourceId,
    project_id: String,
    target: ipc::ResourceTarget,
    state: &crate::State,
) -> crate::Result<(Vec<T>, bool)> {
    let service = state.resources().await.map_err(error)?;
    let project_id = SourceProjectId::new(project_id)
        .map_err(|error| crate::ErrorKind::InputError(error.to_string()))?;
    let cancel = CancellationToken::new();
    let value = service
        .query(
            &source,
            ProviderQuery::Releases {
                project_id: project_id.clone(),
                slug: None,
                target,
                cursor: None,
                page_size: 50,
            },
            CachePolicy::UseFresh,
            &cancel,
        )
        .await
        .map_err(error)?;
    let ProviderValue::Releases(page) = value.value else {
        return Err(crate::ErrorKind::InputError(
            "unexpected release page".into(),
        )
        .into());
    };
    let values = service
        .presentation_many(
            &source,
            page.entries
                .iter()
                .map(|release| ProviderQuery::Release {
                    project_id: Some(project_id.clone()),
                    release_id: release.reference.release_id.clone(),
                    slug: None,
                })
                .collect(),
            &cancel,
        )
        .await;
    let mut documents = Vec::new();
    for value in values {
        match value.map_err(error)?.value {
            ProviderValue::Release(document) => {
                documents.push(serde_json::from_value(
                    document.protocol.ok_or_else(|| {
                        crate::ErrorKind::InputError(
                            "release protocol projection missing".into(),
                        )
                    })?,
                )?)
            }
            _ => {
                return Err(crate::ErrorKind::InputError(
                    "release disappeared from catalog".into(),
                )
                .into());
            }
        }
    }
    Ok((documents, page.cursor.is_some() || page.window_exhausted))
}

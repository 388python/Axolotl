use super::*;
use crate::resources::{ResourceResult, export::plan::invalid};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(super) async fn run(
    job_id: Uuid,
    job_state: &mut InstallJobState,
    plan: &crate::resources::ResourceExportPlan,
    output: &Path,
) -> crate::Result<()> {
    validate(plan)
        .await
        .map_err(|e| ErrorKind::InputError(e.to_string()))?;
    let root = crate::api::instance::get_full_path(&plan.instance_id).await?;
    let reporter = InstallProgressReporter::new(job_id, job_state.clone());
    let cancel = reporter.cancellation_token();
    let progress = Arc::new(AtomicU64::new(0));
    let total: u64 = plan
        .files
        .iter()
        .filter_map(|a| match a {
            crate::resources::ExportFileAction::Embed { snapshot } => {
                snapshot.integrity.size
            }
            _ => None,
        })
        .fold(0u64, u64::saturating_add);
    let track = progress.clone();
    let on_progress = move |current, _| {
        track.store(current, Ordering::Relaxed);
    };
    let writing = crate::resources::export::writer::write_checked(
        &root,
        plan,
        output,
        &cancel,
        &on_progress,
        || async {
            validate(plan).await?;
            if crate::api::instance::get_full_path(&plan.instance_id).await?
                != root
            {
                return Err(invalid(
                    "game directory changed after export analysis",
                ));
            }
            Ok(())
        },
    );
    tokio::pin!(writing);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        tokio::select! {
            result = &mut writing => { result.map_err(|e| ErrorKind::InputError(e.to_string()))?; break; }
            _ = tick.tick() => {
                reporter.update(InstallPhaseId::Exporting, Some(InstallProgress { current: progress.load(Ordering::Relaxed), total, secondary: None }), InstallPhaseDetails::Empty).await?;
            }
        }
    }
    Ok(())
}

async fn validate(
    plan: &crate::resources::ResourceExportPlan,
) -> ResourceResult<()> {
    let state = State::get().await?;
    let current = crate::state::get_instance(&plan.instance_id, &state.pool)
        .await?
        .ok_or_else(|| invalid("instance no longer exists"))?;
    if current.applied_content_set.game_version
        != plan.requirements.game_version
        || current.applied_content_set.loader_version
            != plan.requirements.loader_version
        || (current.applied_content_set.loader != ModLoader::Vanilla)
            .then(|| current.applied_content_set.loader.as_str().to_string())
            != plan.requirements.loader
    {
        return Err(invalid("game requirements changed after export analysis; reopen the export window").into());
    }
    if current.instance.is_direct_linked() {
        return Err(invalid(
            "directly associated instances cannot be exported",
        )
        .into());
    }
    Ok(())
}

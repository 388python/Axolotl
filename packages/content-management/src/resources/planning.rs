use super::*;
use std::collections::HashSet;

pub fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
}

pub fn artifact_path(
    scope: &InstallScope,
    kind: ResourceType,
    artifact: &ResourceArtifact,
) -> Result<String, String> {
    let file = artifact
        .file_name
        .as_ref()
        .ok_or_else(|| "artifact has no filename".to_string())?;
    if !safe_relative_path(file) || file.contains('/') {
        return Err("invalid artifact filename".into());
    }
    let base = match scope {
        InstallScope::Instance => match kind {
            ResourceType::Mod => "mods",
            ResourceType::Plugin => "plugins",
            ResourceType::Shader => "shaderpacks",
            ResourceType::ResourcePack => "resourcepacks",
            _ => {
                return Err("unsupported content scope".into());
            }
        },
        InstallScope::World { relative_path } => {
            if kind != ResourceType::DataPack
                || !safe_relative_path(relative_path)
            {
                return Err("unsupported content scope".into());
            }
            return Ok(format!("{relative_path}/datapacks/{file}"));
        }
    };
    Ok(format!("{base}/{file}"))
}

/// Compatibility remains unknown when the source did not declare a dimension.
pub fn assess_compatibility(
    release: &ResourceRelease,
    target: &ipc::ResourceTarget,
    kind: ResourceType,
) -> CompatibilityAssessment {
    let game_version = dimension(
        &release.compatibility.game_versions,
        &target.game_versions,
        matches_game_version,
    );
    let loader = if matches!(
        kind,
        ResourceType::ResourcePack
            | ResourceType::Shader
            | ResourceType::DataPack
            | ResourceType::World
    ) {
        CompatibilityState::Compatible
    } else {
        dimension(&release.compatibility.loaders, &target.loaders, |a, b| {
            a.eq_ignore_ascii_case(b)
        })
    };
    let mut reasons = vec![];
    if game_version != CompatibilityState::Compatible {
        reasons.push("gameVersion".into());
    }
    if loader != CompatibilityState::Compatible {
        reasons.push("loader".into());
    }
    CompatibilityAssessment {
        game_version,
        loader,
        reasons,
    }
}

fn dimension(
    values: &[String],
    targets: &[String],
    matches: impl Fn(&str, &str) -> bool,
) -> CompatibilityState {
    if values.is_empty() || targets.is_empty() {
        CompatibilityState::Unknown
    } else if values.iter().any(|a| targets.iter().any(|b| matches(a, b))) {
        CompatibilityState::Compatible
    } else {
        CompatibilityState::Incompatible
    }
}

pub fn matches_game_version(pattern: &str, version: &str) -> bool {
    let parts: Vec<_> = pattern.split('.').collect();
    let actual: Vec<_> = version.split('.').collect();
    for (index, part) in parts.iter().enumerate() {
        if matches!(*part, "x" | "X" | "*") {
            return index + 1 == parts.len() && actual.len() > index;
        }
        if actual.get(index) != Some(part) {
            return false;
        }
    }
    parts.len() == actual.len()
}

fn numeric_version(value: &str) -> Option<Vec<u64>> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let parts: Option<Vec<_>> = value
        .split('.')
        .map(|part| {
            if part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
            {
                None
            } else {
                part.parse().ok()
            }
        })
        .collect();
    parts.filter(|parts| !parts.is_empty())
}

pub fn release_is_newer(
    target: &ResourceRelease,
    current: &ResourceRelease,
) -> bool {
    match (
        numeric_version(&target.version),
        numeric_version(&current.version),
    ) {
        (Some(target_version), Some(current_version))
            if target_version.len() == current_version.len() =>
        {
            target_version > current_version
                || (target_version == current_version
                    && target.published_at > current.published_at)
        }
        _ => {
            matches!((target.published_at, current.published_at), (Some(target), Some(current)) if target > current)
        }
    }
}

pub fn release_is_known_lower(
    target: &ResourceRelease,
    current: &ResourceRelease,
) -> bool {
    matches!((numeric_version(&target.version), numeric_version(&current.version)), (Some(target), Some(current)) if target.len() == current.len() && target < current)
}

/// Use one ordering basis for the complete set, never pairwise mixed ordering.
pub fn select_automatic_release(
    mut releases: Vec<ResourceRelease>,
    preferred: &SourceId,
) -> Option<(ResourceRelease, ReleaseSelectionBasis)> {
    if releases.is_empty() {
        return None;
    }
    let versions: Option<Vec<_>> = releases
        .iter()
        .map(|release| numeric_version(&release.version))
        .collect();
    let comparable = versions.as_ref().is_some_and(|versions| {
        versions
            .iter()
            .all(|version| version.len() == versions[0].len())
    });
    releases.sort_by(|a, b| {
        let order = if comparable {
            numeric_version(&a.version).cmp(&numeric_version(&b.version))
        } else {
            a.published_at.cmp(&b.published_at)
        };
        order
            .then(a.published_at.cmp(&b.published_at))
            .then(a.reference.project.source.cmp(&b.reference.project.source))
            .then(a.reference.release_id.cmp(&b.reference.release_id))
    });
    let selected = releases.pop()?;
    let equivalent = releases.into_iter().rev().find(|release| {
        selected.reference.project.source != *preferred
            && release.reference.project.source == *preferred
            && !release_is_known_lower(release, &selected)
            && unique_primary(release)
                .zip(unique_primary(&selected))
                .is_some_and(|(a, b)| equal_bytes(&a.integrity, &b.integrity))
    });
    if let Some(equivalent) = equivalent {
        Some((equivalent, ReleaseSelectionBasis::EquivalentPreferredSource))
    } else {
        Some((
            selected,
            if comparable {
                ReleaseSelectionBasis::ComparableVersion
            } else {
                ReleaseSelectionBasis::PublishedAt
            },
        ))
    }
}

fn unique_primary(release: &ResourceRelease) -> Option<&ResourceArtifact> {
    let mut artifacts = release
        .artifacts
        .iter()
        .filter(|artifact| artifact.role == ArtifactRole::Primary);
    let primary = artifacts.next()?;
    artifacts.next().is_none().then_some(primary)
}

pub fn equal_bytes(
    left: &ArtifactIntegrity,
    right: &ArtifactIntegrity,
) -> bool {
    if left.size.is_none() || left.size != right.size {
        return false;
    }
    let mut shared = false;
    for a in &left.hashes {
        for b in right.hashes.iter().filter(|b| b.algorithm == a.algorithm) {
            if !a.value.eq_ignore_ascii_case(&b.value) {
                return false;
            }
            shared |= a.algorithm != HashAlgorithm::Md5;
        }
    }
    shared
}

/// Only required edges propagate activity; each optional resource is an explicit root.
pub fn active_dependency_nodes(
    roots: &[SelectionId],
    edges: &[ResourceDependencyEdge],
) -> HashSet<SelectionId> {
    let mut active: HashSet<_> = roots.iter().cloned().collect();
    let mut queue = roots.to_vec();
    while let Some(parent) = queue.pop() {
        for edge in edges.iter().filter(|edge| {
            edge.parent == parent
                && edge.kind == ResourceDependencyKind::Required
        }) {
            if active.insert(edge.child.clone()) {
                queue.push(edge.child.clone());
            }
        }
    }
    active
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(
        version: &str,
        source: SourceId,
        time: i64,
        hash: char,
    ) -> ResourceRelease {
        let reference = SourceReleaseRef {
            project: SourceProjectRef {
                source,
                project_id: SourceProjectId::new("abcdefgh").unwrap(),
            },
            release_id: SourceReleaseId::new(time.to_string()).unwrap(),
        };
        ResourceRelease {
            reference: reference.clone(),
            version: version.into(),
            channel: ReleaseChannel::Release,
            published_at: chrono::DateTime::from_timestamp(time, 0),
            compatibility: ReleaseCompatibility {
                game_versions: vec![],
                loaders: vec![],
            },
            artifacts: vec![ResourceArtifact {
                reference: SourceArtifactRef {
                    release: reference,
                    artifact_id: SourceArtifactId::new("primary").unwrap(),
                },
                file_name: Some("file.jar".into()),
                role: ArtifactRole::Primary,
                available: Some(true),
                integrity: ArtifactIntegrity {
                    size: Some(100),
                    hashes: vec![ContentHash {
                        algorithm: HashAlgorithm::Sha1,
                        value: hash.to_string().repeat(40),
                    }],
                },
                acquisition: ArtifactAcquisition::Unavailable,
                dependencies: vec![],
            }],
            equivalent_releases: vec![],
        }
    }

    #[test]
    fn shared_attachments_never_replace_the_selected_primary_release() {
        let mut old = release("2", SourceId::modrinth(), 1, 'a');
        let mut new = release("2", SourceId::curseforge(), 2, 'b');
        let mut attachment = old.artifacts[0].clone();
        attachment.role = ArtifactRole::Attachment;
        attachment.reference.artifact_id =
            SourceArtifactId::new("attachment").unwrap();
        old.artifacts.push(attachment.clone());
        attachment.reference.release = new.reference.clone();
        new.artifacts.push(attachment);
        assert_eq!(
            select_automatic_release(
                vec![old, new.clone()],
                &SourceId::modrinth()
            )
            .unwrap()
            .0
            .reference,
            new.reference
        );
    }

    #[test]
    fn preferred_source_only_replaces_equivalent_unique_primary_artifacts() {
        let oldest = release("2", SourceId::modrinth(), 1, 'a');
        let newest = release("2", SourceId::modrinth(), 2, 'a');
        let selected = release("2", SourceId::curseforge(), 3, 'a');
        let (result, basis) = select_automatic_release(
            vec![oldest, newest.clone(), selected.clone()],
            &SourceId::modrinth(),
        )
        .unwrap();
        assert_eq!(result.reference, newest.reference);
        assert_eq!(basis, ReleaseSelectionBasis::EquivalentPreferredSource);
        let lower = release("1", SourceId::modrinth(), 2, 'a');
        assert_eq!(
            select_automatic_release(
                vec![lower, selected.clone()],
                &SourceId::modrinth()
            )
            .unwrap()
            .0
            .reference,
            selected.reference
        );
        let mut ambiguous = newest;
        ambiguous.artifacts.push(ambiguous.artifacts[0].clone());
        assert_eq!(
            select_automatic_release(
                vec![ambiguous, selected.clone()],
                &SourceId::modrinth()
            )
            .unwrap()
            .0
            .reference,
            selected.reference
        );
    }

    #[test]
    fn preferred_source_does_not_replace_itself_with_an_older_release() {
        let old = release("1", SourceId::modrinth(), 1, 'a');
        let new = release("2", SourceId::modrinth(), 2, 'a');
        assert_eq!(
            select_automatic_release(
                vec![old, new.clone()],
                &SourceId::modrinth()
            )
            .unwrap()
            .0
            .reference,
            new.reference
        );
    }
    #[test]
    fn wildcard_preserves_major_and_minor_boundaries() {
        assert!(matches_game_version("1.20.x", "1.20.4"));
        assert!(!matches_game_version("1.20.x", "1.12"));
        assert!(!matches_game_version("1.20.x", "1.21"));
        assert!(matches_game_version("1.x", "1.21.1"));
        assert!(!matches_game_version("1.x", "2.0"));
        assert!(!matches_game_version("1.x.2", "1.20.2"));
    }

    #[test]
    fn optional_edges_do_not_activate_until_the_optional_root_is_added() {
        let edge = ResourceDependencyEdge {
            parent: SelectionId::new("a").unwrap(),
            child: SelectionId::new("b").unwrap(),
            declared_by: SourceArtifactRef {
                release: SourceReleaseRef {
                    project: SourceProjectRef {
                        source: SourceId::new("modrinth").unwrap(),
                        project_id: SourceProjectId::new("abcdefgh").unwrap(),
                    },
                    release_id: SourceReleaseId::new("release").unwrap(),
                },
                artifact_id: SourceArtifactId::new("file").unwrap(),
            },
            kind: ResourceDependencyKind::Required,
            version_constraint: None,
        };
        assert!(
            !active_dependency_nodes(
                &[SelectionId::new("root").unwrap()],
                &[edge.clone()]
            )
            .contains(&edge.child)
        );
        assert!(
            active_dependency_nodes(&[SelectionId::new("a").unwrap()], &[edge])
                .contains(&SelectionId::new("b").unwrap())
        );
    }

    #[test]
    fn target_paths_reject_traversal_and_preserve_scope() {
        let artifact = ResourceArtifact {
            reference: SourceArtifactRef {
                release: SourceReleaseRef {
                    project: SourceProjectRef {
                        source: SourceId::new("modrinth").unwrap(),
                        project_id: SourceProjectId::new("abcdefgh").unwrap(),
                    },
                    release_id: SourceReleaseId::new("release").unwrap(),
                },
                artifact_id: SourceArtifactId::new("file").unwrap(),
            },
            file_name: Some("example.jar".into()),
            role: ArtifactRole::Primary,
            available: Some(true),
            integrity: ArtifactIntegrity {
                size: Some(1),
                hashes: vec![],
            },
            acquisition: ArtifactAcquisition::Unavailable,
            dependencies: vec![],
        };
        assert_eq!(
            artifact_path(
                &InstallScope::Instance,
                ResourceType::Mod,
                &artifact
            )
            .unwrap(),
            "mods/example.jar"
        );
        assert!(!safe_relative_path("../escape.jar"));
    }
}

use super::*;

pub(crate) fn for_source(
    expression: &ResourceSearchExpression,
    source: &SourceId,
) -> ResourceResult<ResourceSearchExpression> {
    match expression {
        ResourceSearchExpression::All { terms } => {
            Ok(ResourceSearchExpression::All {
                terms: terms
                    .iter()
                    .map(|term| for_source(term, source))
                    .collect::<ResourceResult<_>>()?,
            })
        }
        ResourceSearchExpression::Any { terms } => {
            Ok(ResourceSearchExpression::Any {
                terms: terms
                    .iter()
                    .map(|term| for_source(term, source))
                    .collect::<ResourceResult<_>>()?,
            })
        }
        ResourceSearchExpression::Predicate {
            field,
            values,
            excluded,
        } => {
            let selected: Vec<_> = values
                .iter()
                .filter_map(|value| {
                    if field == "projectId" && *excluded {
                        if let Some((owner, id)) = value.split_once(':') {
                            return (owner == source.as_str())
                                .then(|| id.to_string());
                        }
                    }
                    if field == "category"
                        && let Some((owner, value)) = value.split_once(':')
                    {
                        return (owner == source.as_str())
                            .then(|| value.to_string());
                    }
                    Some(value.clone())
                })
                .collect();
            if selected.is_empty() {
                if field == "projectId" && *excluded {
                    return Ok(ResourceSearchExpression::All { terms: vec![] });
                }
                return Err(ResourceError::Unsupported(format!(
                    "source cannot express {field} filter"
                )));
            }
            Ok(ResourceSearchExpression::Predicate {
                field: field.clone(),
                values: selected,
                excluded: *excluded,
            })
        }
    }
}

pub(crate) fn modrinth(
    expression: &ResourceSearchExpression,
    depth: usize,
) -> ResourceResult<String> {
    if depth > 16 {
        return Err(ResourceError::InvalidRequest(
            "search filter nesting exceeded".into(),
        ));
    }
    match expression {
        ResourceSearchExpression::All { terms }
        | ResourceSearchExpression::Any { terms } => {
            if terms.is_empty() {
                return Ok("project_id != \"\"".into());
            }
            if terms.len() > 4096 {
                return Err(ResourceError::InvalidRequest(
                    "invalid search filter group".into(),
                ));
            }
            let operator =
                if matches!(expression, ResourceSearchExpression::All { .. }) {
                    " AND "
                } else {
                    " OR "
                };
            Ok(format!(
                "({})",
                terms
                    .iter()
                    .map(|term| modrinth(term, depth + 1))
                    .collect::<ResourceResult<Vec<_>>>()?
                    .join(operator)
            ))
        }
        ResourceSearchExpression::Predicate {
            field,
            values,
            excluded,
        } => {
            let field = match field.as_str() {
                "type" => "project_types",
                "allTypes" => "all_project_types",
                "category" | "loader" => "categories",
                "gameVersion" => "game_versions",
                "environment" => "environment",
                "license" => "license",
                "openSource" => "open_source",
                "projectId" => "project_id",
                "dependencyProject" => "compatible_dependency_project_ids",
                "requiredDependency" => "required_dependency_project_ids",
                "optionalDependency" => "optional_dependency_project_ids",
                "incompatibleDependency" => {
                    "incompatible_dependency_project_ids"
                }
                "embeddedDependency" => "embedded_dependency_project_ids",
                _ => {
                    return Err(ResourceError::Unsupported(
                        "unknown search filter field".into(),
                    ));
                }
            };
            if values.is_empty()
                || values.len() > 100
                || values.iter().any(|value| {
                    value.len() > 512 || value.chars().any(char::is_control)
                })
            {
                return Err(ResourceError::InvalidRequest(
                    "invalid search filter values".into(),
                ));
            }
            let quoted = values
                .iter()
                .map(|value| {
                    if field == "open_source"
                        && matches!(value.as_str(), "true" | "false")
                    {
                        Ok(value.clone())
                    } else {
                        serde_json::to_string(value)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(if quoted.len() == 1 {
                format!(
                    "{field} {} {}",
                    if *excluded { "!=" } else { "=" },
                    quoted[0]
                )
            } else {
                format!(
                    "{field} {} [{}]",
                    if *excluded { "NOT IN" } else { "IN" },
                    quoted.join(", ")
                )
            })
        }
    }
}

pub(crate) fn validate_curseforge(
    expression: &ResourceSearchExpression,
    depth: usize,
) -> ResourceResult<()> {
    if depth > 16 {
        return Err(ResourceError::InvalidRequest(
            "search filter nesting exceeded".into(),
        ));
    }
    match expression {
        ResourceSearchExpression::All { terms } => {
            for term in terms {
                validate_curseforge(term, depth + 1)?;
            }
            Ok(())
        }
        ResourceSearchExpression::Predicate {
            field,
            excluded: false,
            ..
        } if matches!(
            field.as_str(),
            "type" | "category" | "loader" | "gameVersion"
        ) =>
        {
            Ok(())
        }
        ResourceSearchExpression::Predicate {
            field,
            excluded: true,
            ..
        } if matches!(field.as_str(), "projectId" | "allTypes" | "type") => {
            Ok(())
        }
        _ => Err(ResourceError::Unsupported(
            "CurseForge cannot express these search filters".into(),
        )),
    }
}

//! Fleet-wide derived skill inventory.

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::CallerIdentity;
use crate::http::AppState;
use crate::traits::{InstanceStatus, ListFilter};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/skills", get(list_skills))
        .with_state(state)
}

async fn list_skills(
    State(state): State<AppState>,
    Extension(caller): Extension<CallerIdentity>,
) -> Result<Json<Vec<crate::skill_inventory::SkillInventoryEntry>>, StatusCode> {
    let rows = state
        .instances
        .list(
            &caller.user_id,
            ListFilter {
                status: Some(InstanceStatus::Live),
                include_destroyed: false,
            },
        )
        .await
        .map_err(super::instances::swarm_err_to_status)?;

    let mut out = Vec::new();
    for row in rows {
        let mut skills =
            match crate::skill_inventory::list_instance_skills(state.state_files.as_ref(), &row.id)
                .await
            {
                Ok(skills) => skills,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        instance = %row.id,
                        "fleet skills: inventory derivation failed"
                    );
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            };
        out.append(&mut skills);
    }
    out.sort_by(|a, b| {
        a.skill
            .cmp(&b.skill)
            .then_with(|| a.instance_id.cmp(&b.instance_id))
    });
    Ok(Json(out))
}

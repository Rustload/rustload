//! Download statistics.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;

use super::AppState;
use crate::{
    error::ApiResult,
    stats::{self, Report},
};

#[derive(Deserialize)]
pub struct StatsQuery {
    #[serde(default = "default_days")]
    days: i64,
}

fn default_days() -> i64 {
    30
}

pub async fn get(State(state): State<AppState>, Query(query): Query<StatsQuery>) -> ApiResult<Json<Report>> {
    let days = query.days.clamp(1, 365) as u32;
    Ok(Json(stats::report(state.db(), days, chrono::Local::now().date_naive()).await?))
}

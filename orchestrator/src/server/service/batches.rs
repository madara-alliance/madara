use std::str::FromStr;

use crate::core::client::database::{AggregatorBatchDbQuery, BatchIndexSort, SnosBatchDbQuery};
use crate::core::config::Config;
use crate::server::error::BlockRouteError;
use crate::server::types::{AggregatorBatchListResponse, BatchQuery, BatchSortOrder, SnosBatchListResponse};
use crate::types::batch::{AggregatorBatchStatus, SnosBatchStatus};

const DEFAULT_BATCH_QUERY_LIMIT: i64 = 50;
const MAX_BATCH_QUERY_LIMIT: i64 = 500;

pub struct BatchService;

impl BatchService {
    pub async fn query_snos_batches(
        config: &Config,
        query: BatchQuery,
    ) -> Result<SnosBatchListResponse, BlockRouteError> {
        let db_query = SnosBatchDbQuery {
            indexes: query.index.map(|index| vec![index]),
            statuses: parse_snos_statuses(query.status.as_deref())?,
            limit: Some(validate_limit(query.limit)?),
            orchestrator_version: None,
            sort: sort_from_query(query.sort),
        };

        let batches = config
            .database()
            .get_snos_batches(db_query)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;

        Ok(SnosBatchListResponse { batches: batches.iter().map(Into::into).collect() })
    }

    pub async fn query_aggregator_batches(
        config: &Config,
        query: BatchQuery,
    ) -> Result<AggregatorBatchListResponse, BlockRouteError> {
        let db_query = AggregatorBatchDbQuery {
            indexes: query.index.map(|index| vec![index]),
            statuses: parse_aggregator_statuses(query.status.as_deref())?,
            limit: Some(validate_limit(query.limit)?),
            orchestrator_version: None,
            sort: sort_from_query(query.sort),
        };

        let batches = config
            .database()
            .get_aggregator_batches(db_query)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;

        Ok(AggregatorBatchListResponse { batches: batches.iter().map(Into::into).collect() })
    }
}

fn validate_limit(limit: Option<i64>) -> Result<i64, BlockRouteError> {
    match limit {
        Some(value) if value <= 0 => Err(BlockRouteError::InvalidQuery("limit must be a positive integer".to_string())),
        Some(value) if value > MAX_BATCH_QUERY_LIMIT => {
            Err(BlockRouteError::InvalidQuery(format!("limit must be less than or equal to {MAX_BATCH_QUERY_LIMIT}")))
        }
        Some(value) => Ok(value),
        None => Ok(DEFAULT_BATCH_QUERY_LIMIT),
    }
}

fn sort_from_query(sort: BatchSortOrder) -> BatchIndexSort {
    match sort {
        BatchSortOrder::Asc => BatchIndexSort::Asc,
        BatchSortOrder::Desc => BatchIndexSort::Desc,
    }
}

fn parse_snos_statuses(status: Option<&str>) -> Result<Option<Vec<SnosBatchStatus>>, BlockRouteError> {
    parse_statuses("snos", status, || {
        vec![SnosBatchStatus::Closed, SnosBatchStatus::SnosJobCreated, SnosBatchStatus::Completed]
    })
}

fn parse_aggregator_statuses(status: Option<&str>) -> Result<Option<Vec<AggregatorBatchStatus>>, BlockRouteError> {
    parse_statuses("aggregator", status, || {
        vec![
            AggregatorBatchStatus::Closed,
            AggregatorBatchStatus::PendingAggregatorRun,
            AggregatorBatchStatus::PendingAggregatorVerification,
            AggregatorBatchStatus::ReadyForStateUpdate,
            AggregatorBatchStatus::Completed,
            AggregatorBatchStatus::AggregationFailed,
            AggregatorBatchStatus::VerificationFailed,
            AggregatorBatchStatus::StateUpdateFailed,
        ]
    })
}

fn parse_statuses<T>(
    kind: &str,
    status: Option<&str>,
    closed_statuses: impl FnOnce() -> Vec<T>,
) -> Result<Option<Vec<T>>, BlockRouteError>
where
    T: FromStr,
{
    match status.map(str::trim) {
        None | Some("") => Ok(None),
        Some(status) if status.eq_ignore_ascii_case("closed") => Ok(Some(closed_statuses())),
        Some(status) => status
            .parse()
            .map(|status| Some(vec![status]))
            .map_err(|_| BlockRouteError::InvalidQuery(format!("unsupported {kind} batch status '{status}'"))),
    }
}

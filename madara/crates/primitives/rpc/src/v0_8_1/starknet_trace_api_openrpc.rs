use serde::{Deserialize, Serialize};

use super::{FeeEstimate, TransactionTrace};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SimulateTransactionsResult {
    pub fee_estimation: FeeEstimate,
    pub transaction_trace: TransactionTrace,
}

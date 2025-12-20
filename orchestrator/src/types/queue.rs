use strum_macros::{Display, EnumIter};

#[derive(Display, Debug, Clone, PartialEq, Eq, EnumIter, Hash)]
pub enum QueueType {
    #[strum(serialize = "worker_trigger")]
    WorkerTrigger,
}

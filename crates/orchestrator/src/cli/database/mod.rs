use crate::database::mongodb::MongoDBValidatedArgs;

pub mod mongodb;

#[derive(Debug, Clone)]
pub enum DatabaseValidatedArgs {
    MongoDB(MongoDBValidatedArgs),
}
